//! Hardware-visible memory seam shared by CPU and DMA masters.

use crate::apu::{Apu, StereoSample};
use crate::cartridge::{
    gpio::Gpio as CartridgeGpio,
    save::{self, Device as SaveDevice},
    tilt,
};
use crate::dma::Channel as DmaChannel;
use crate::timer::Timer;
use crate::types::Cycle;
use crate::video::{
    AffineTransition, LineLayers, scanline_memo, tile_row_cache, timing::PpuTiming,
};
use evenfall_state::Codec;
use std::ops::Deref;
use std::sync::Arc;

mod kernel;
pub(crate) use kernel::{EngineMemory, PrefetchClock, RomData, RunRegisters, VideoMemory};
pub use kernel::{begin_arm_iwram_slice, fetch_arm_iwram_in_slice};
use kernel::{oam_read, palette_read, vram_read};

pub const SCREEN_WIDTH: usize = 240;
pub const SCREEN_HEIGHT: usize = 160;
pub(crate) const BIOS_SIZE: usize = 16 * 1024;
pub(crate) const EWRAM_SIZE: usize = 256 * 1024;
pub(crate) const IWRAM_SIZE: usize = 32 * 1024;
const VRAM_SIZE: usize = 96 * 1024;
const PALETTE_SIZE: usize = 1024;
const OAM_SIZE: usize = 1024;
const MAX_SCROLL_SEGMENTS: usize = 64;
/// Timed inputs a run can take without the bus growing its input buffer.
const EXTERNAL_INPUT_CAPACITY: usize = 64;
const EWRAM_BASE: u32 = 0x0200_0000;
const IWRAM_BASE: u32 = 0x0300_0000;
const MMIO_BASE: u32 = 0x0400_0000;
const VRAM_BASE: u32 = 0x0600_0000;
const GAME_PAK_BASE: u32 = 0x0800_0000;
const GAME_PAK_WINDOW_SIZE: u32 = 32 * 1024 * 1024;

/// Shared, read-only cartridge or BIOS bytes. Adopting a Vec keeps its byte allocation.
/// Cloning shares the allocation; even with an exclusive handle, no mutable view is exposed.
///
/// ```compile_fail
/// use evenfall_gba::bus::Media;
/// let mut rom = Media::from_vec(vec![0x42]);
/// let bytes: &mut [u8] = &mut rom;
/// bytes[0] = 0;
/// ```
#[derive(Clone)]
pub struct Media(Arc<Vec<u8>>);

impl Media {
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self(Arc::new(bytes))
    }

    pub fn from_slice(bytes: &[u8]) -> Self {
        Self::from_vec(bytes.to_vec())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for Media {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessWidth {
    Byte,
    Half,
    #[default]
    Word,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
#[codec(tag_bits = 1)]
pub enum AccessKind {
    #[default]
    Nonsequential,
    Sequential,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessSource {
    CpuCode,
    CpuData,
    Dma,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub address: u32,
    pub width: AccessWidth,
    pub kind: AccessKind,
    pub source: AccessSource,
}

/// KEYINPUT's low ten bits; false means released.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
pub struct KeyState {
    pub a: bool,
    pub b: bool,
    pub select: bool,
    pub start: bool,
    pub right: bool,
    pub left: bool,
    pub up: bool,
    pub down: bool,
    pub r: bool,
    pub l: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedInput {
    pub at: Cycle,
    pub change: TimedInputChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimedInputChange {
    Keys(KeyState),
    SolarLevel(u8),
    GyroSample(u16),
    Tilt(tilt::Sample),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadResult {
    pub value: u32,
    pub cycles: Cycle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteResult {
    pub cycles: Cycle,
}

/// IE, IF, IME, POSTFLG and HALTCNT, with the IRQ line they drive: a
/// flat stretch of the bus state, grouped so that an engine run can
/// read and write them through one reference ([`EngineMemory`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
pub struct IrqRegisters {
    pub interrupt_enable_mask: u16,
    pub interrupt_request_flags: u16,
    pub interrupt_master_enabled: bool,
    /// The cycle at which a request that has risen (IME set and an enabled
    /// flag raised) reaches the CPU, which sees it [`IRQ_LATENCY`] cycles
    /// late; `None` when the CPU already sees the request or there is none.
    pub irq_line_rises_at: Option<Cycle>,
    /// Whether the CPU sees an interrupt request: derived from the
    /// registers and [`Self::irq_line_rises_at`].
    #[codec(skip)]
    pub irq_line: bool,
    pub post_boot: u8,
    pub power_state: PowerState,
}

/// Cycles from the cycle an interrupt request rises to the first
/// instruction boundary at which the CPU can take it. The mGBA suite's timer
/// IRQ table, recorded on hardware, runs four one-cycle instructions after
/// the overflow before the IRQ preempts the fifth. A request that falls
/// reaches the CPU at once.
pub const IRQ_LATENCY: Cycle = 4;

impl IrqRegisters {
    /// Whether IME is set and an enabled flag is raised: the request the
    /// interrupt controller drives, before it reaches the CPU.
    pub fn request(&self) -> bool {
        self.interrupt_master_enabled
            && self.interrupt_enable_mask & self.interrupt_request_flags != 0
    }

    /// Follows the registers after a change at `cycle`: a request that
    /// rises reaches the CPU [`IRQ_LATENCY`] cycles later.
    pub fn update_line(&mut self, cycle: Cycle) {
        if !self.request() {
            self.irq_line = false;
            self.irq_line_rises_at = None;
        } else if !self.irq_line && self.irq_line_rises_at.is_none() {
            self.irq_line_rises_at = Some(cycle + IRQ_LATENCY);
        }
    }

    /// Lets a risen request reach the CPU once `cycle` gets to it.
    #[inline(always)]
    pub fn reach(&mut self, cycle: Cycle) {
        if self.irq_line_rises_at.is_some_and(|at| at <= cycle) {
            self.irq_line_rises_at = None;
            self.irq_line = true;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
#[codec(tag_bits = 2)]
pub enum PowerState {
    #[default]
    Running,
    Halted,
    Stopped,
}

/// A DMA transfer that yielded the bus to the host part-way: the state
/// `Bus::run_dma` keeps in locals, from the beat that just completed on.
/// See `Bus::run_dma_burst`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaProgress {
    pub source: u32,
    pub destination: u32,
    pub control: u16,
    pub transfer_words: u32,
    /// The index of the beat that completed last.
    pub beat: u32,
    pub did_access_rom: bool,
    pub is_audio_fifo: bool,
    pub bulk: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SioLinkError {
    UnsupportedPeerCount,
    UnsupportedMode,
    ClockConflict,
    MissedDeadline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SioConnection {
    Cable2p,
    CableMulti,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
pub enum JoyBusCommand {
    #[default]
    Reset,
    Status,
    ReadFromGba,
    WriteToGba(u32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
pub struct JoyBusResponse {
    pub data: [u8; 5],
    #[codec(bits = 3)]
    pub len: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoyBusError {
    NotInJoyBusMode,
    CompletionBeforeCurrentCycle,
    TransactionPending,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub struct UartFifo {
    pub bytes: [u8; 4],
    #[codec(bits = 2)]
    pub read_index: u8,
    #[codec(bits = 2)]
    pub write_index: u8,
    #[codec(bits = 3)]
    pub count: u8,
}

impl UartFifo {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn push(&mut self, value: u8, capacity: u8) -> bool {
        if self.count >= capacity {
            return false;
        }
        self.bytes[usize::from(self.write_index)] = value;
        self.write_index = self.write_index.wrapping_add(1) & 3;
        self.count += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.count == 0 {
            return None;
        }
        let value = self.bytes[usize::from(self.read_index)];
        self.read_index = self.read_index.wrapping_add(1) & 3;
        self.count -= 1;
        Some(value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
pub struct GamePakPrefetch {
    pub active: bool,
    pub head_address: u32,
    pub last_address: u32,
    #[codec(bits = 4)]
    pub count: u8,
    #[codec(bits = 4)]
    pub capacity: u8,
    #[codec(bits = 3)]
    pub opcode_bytes: u8,
    pub countdown: Cycle,
    pub duty: Cycle,
}

/// The video implementation owns raster history operations; the records are
/// declared here because their order and names are part of Bus's state schema.
#[derive(Clone, Copy, Debug, Default, Codec)]
pub struct ScrollSegment {
    pub start_x: u16,
    pub hofs: u16,
    pub vofs: u16,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub struct BackdropSegment {
    pub start_x: u16,
    pub color: u16,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub struct WindowSegment {
    pub start_x: u16,
    pub win0h: u16,
    pub win0v: u16,
    pub win1h: u16,
    pub win1v: u16,
    pub winin: u16,
    pub winout: u16,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub struct BlendSegment {
    pub start_x: u16,
    pub bldcnt: u16,
    pub bldalpha: u16,
    pub bldy: u16,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub struct AffineSegment {
    pub start_x: u16,
    pub pa: i16,
    pub pc: i16,
    pub ref_x: i32,
    pub ref_y: i32,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub struct MosaicSegment {
    pub start_x: u16,
    pub mosaic: u16,
}

/// The current line's per-pixel window masks. Every line refills them with
/// 16-byte stores, which on a cache-line boundary never split a line; split
/// stores stall, and where the field lands otherwise depends on `Bus`'s
/// layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(align(64))]
pub struct WindowMasks(pub [u8; SCREEN_WIDTH]);

impl Deref for WindowMasks {
    type Target = [u8; SCREEN_WIDTH];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for WindowMasks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum LineScan {
    #[default]
    Unknown,
    Absent,
    Present,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CompactObject {
    pub attr0: u16,
    pub attr1: u16,
    pub attr2: u16,
    pub affine: bool,
    pub object_width: u8,
    pub object_height: u8,
    pub display_width: u8,
    pub display_height: u8,
    pub object_y: u8,
    pub first_processed_x: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ObjectLineData {
    pub attr0: u16,
    pub attr1: u16,
    pub attr2: u16,
    pub affine: bool,
    pub object_width: u8,
    pub object_height: u8,
    pub display_width: u8,
    pub display_height: u8,
    pub display_y: u8,
    pub first_processed_x: u8,
    pub processed_end: u8,
}

/// Complete live register key for memoized scanline composition.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanlineRegisters {
    pub dispcnt: u16,
    pub green_swap: u16,
    pub bg_controls: [u16; 4],
    pub bg_horizontal_offsets: [u16; 4],
    pub bg_vertical_offsets: [u16; 4],
    pub windows: [u16; 6],
    pub blend: [u16; 3],
    pub mosaic: u16,
    pub bg_affine_parameters: [[i16; 4]; 2],
    pub bg_affine_current: [[i32; 2]; 2],
}

/// A ROM or mirrored IWRAM view valid until the caller reclassifies its PC.
#[derive(Clone, Copy, Debug, Default)]
pub struct InstructionFetchRegion {
    pub bytes: Option<*const u8>,
    pub address: u32,
    pub valid_starts: u32,
    pub region_class: InstructionFetchClass,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstructionFetchClass {
    Rom,
    Iwram,
    #[default]
    Other,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ThumbStream {
    pub armed: bool,
    pub limit: u32,
    pub duty: Cycle,
    pub deadline: Cycle,
}

#[derive(Codec)]
#[codec(module = "evenfall.gba.bus")]
pub struct Bus {
    pub ewram: Box<[u8; EWRAM_SIZE]>,
    pub iwram: Box<[u8; IWRAM_SIZE]>,
    #[codec(skip)]
    pub(crate) code: crate::cpu::engine::CodeCache,
    pub palette: [u8; PALETTE_SIZE],
    pub vram: Box<[u8; VRAM_SIZE]>,
    pub oam: [u8; OAM_SIZE],
    /// Encoded last: its length depends on the save type.
    #[codec(last)]
    pub save: SaveDevice,
    pub frame_pixels: Box<[u16; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub frame_priorities: Box<[u8; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub frame_layers: Box<[u8; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub frame_second_pixels: Box<[u16; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub frame_second_priorities: Box<[u8; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub frame_second_layers: Box<[u8; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub frame_semitransparent: Box<[bool; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub frame_obj_window: Box<[bool; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub line_needs_second_surface: bool,
    #[codec(skip)]
    pub line_has_color_effects: bool,
    #[codec(skip)]
    pub line_semitransparent_obj: LineScan,
    #[codec(skip)]
    pub line_window_masks: WindowMasks,
    #[codec(skip)]
    pub line_unmasked_layers: u8,
    #[codec(skip)]
    pub line_window_key: u64,
    #[codec(skip)]
    pub tile_row_cache: Box<tile_row_cache::Cache>,
    #[codec(skip)]
    pub(crate) line_layers: Box<LineLayers>,
    #[codec(skip)]
    pub scanline_memo: Box<scanline_memo::Memo<ScanlineRegisters, SCREEN_HEIGHT>>,
    #[codec(skip)]
    pub video_active_interval: u16,
    #[codec(skip)]
    pub video_requested_interval: u16,
    #[codec(skip)]
    pub video_skip_remaining: u16,
    #[codec(skip)]
    pub video_render_current: bool,
    #[codec(skip)]
    pub video_framebuffer_canonical: bool,
    #[codec(skip)]
    pub video_publication_active: bool,
    #[codec(skip)]
    pub video_published_pixels: Box<[u16; SCREEN_WIDTH * SCREEN_HEIGHT]>,
    #[codec(skip)]
    pub video_frame_id: Cycle,
    #[codec(skip)]
    pub video_frame_cycle: Cycle,
    #[codec(skip)]
    pub(crate) rom: Media,
    #[codec(skip)]
    pub(crate) bios: Media,
    pub cartridge_gpio: Option<CartridgeGpio>,
    pub cartridge_tilt: Option<tilt::Tilt>,
    pub timers: [Timer; 4],
    #[codec(bits = 10)]
    pub prescaler_origin: u16,
    pub dma_channels: [DmaChannel; 4],
    pub apu: Apu,
    pub ppu_timing: PpuTiming,
    pub dispcnt: u16,
    pub green_swap_enabled: bool,
    pub bg_controls: [u16; 4],
    pub bg_horizontal_offsets: [u16; 4],
    pub bg_vertical_offsets: [u16; 4],
    #[codec(encode_with = encode_scroll_segments)]
    pub scroll_segments: [[ScrollSegment; MAX_SCROLL_SEGMENTS]; 4],
    pub scroll_segment_counts: [u8; 4],
    pub scroll_segment_line: u16,
    #[codec(encode_with = encode_backdrop_segments)]
    pub backdrop_segments: [BackdropSegment; MAX_SCROLL_SEGMENTS],
    pub backdrop_segment_count: u8,
    pub backdrop_segment_line: u16,
    #[codec(encode_with = encode_win_segments)]
    pub win_segments: [WindowSegment; MAX_SCROLL_SEGMENTS],
    pub win_segment_count: u8,
    pub win_segment_line: u16,
    #[codec(encode_with = encode_blend_segments)]
    pub blend_segments: [BlendSegment; MAX_SCROLL_SEGMENTS],
    pub blend_segment_count: u8,
    pub blend_segment_line: u16,
    #[codec(encode_with = encode_affine_segments)]
    pub affine_segments: [[AffineSegment; MAX_SCROLL_SEGMENTS]; 2],
    pub affine_segment_counts: [u8; 2],
    pub affine_segment_line: u16,
    #[codec(encode_with = encode_mosaic_segments)]
    pub mosaic_segments: [MosaicSegment; MAX_SCROLL_SEGMENTS],
    pub mosaic_segment_count: u8,
    pub mosaic_segment_line: u16,
    pub bg_affine_parameters: [[i16; 4]; 2],
    pub bg_reference_points: [[i32; 2]; 2],
    pub bg_affine_current: [[i32; 2]; 2],
    pub win0h: u16,
    pub win1h: u16,
    pub win0v: u16,
    pub win1v: u16,
    pub winin: u16,
    pub winout: u16,
    pub mosaic: u16,
    pub bldcnt: u16,
    pub bldalpha: u16,
    pub bldy: u16,
    pub completed_frame_count: Cycle,
    #[codec(flatten)]
    pub irq: IrqRegisters,
    /// Whether a store an engine run made moved the cycle at which an
    /// interrupt request reaches the CPU, so that the run's device horizon
    /// is found again when it finishes.
    #[codec(skip)]
    pub(crate) irq_arrival_moved: bool,
    pub waitcnt: u16,
    #[codec(skip)]
    pub game_pak_timing_key: u16,
    #[codec(skip)]
    pub game_pak_n_cycles: [u8; 3],
    #[codec(skip)]
    pub game_pak_s_cycles: [u8; 3],
    pub game_pak_prefetch: GamePakPrefetch,
    #[codec(skip)]
    pub duty_valid_until: u32,
    pub fetch_kind: AccessKind,
    #[codec(skip)]
    pub oam_compact: [CompactObject; 128],
    #[codec(skip)]
    pub oam_compact_count: u8,
    #[codec(skip)]
    pub oam_compact_valid: bool,
    #[codec(skip)]
    pub object_line: [ObjectLineData; 128],
    #[codec(skip)]
    pub object_line_count: u8,
    #[codec(skip)]
    pub object_line_index: u16,
    pub game_pak_code_active: bool,
    pub prefetch_disabled_bug_pending: bool,
    pub prefetch_buffer_was_disabled: bool,
    pub keys: KeyState,
    pub keypad_control: u16,
    pub keypad_condition_met: bool,
    /// The timed inputs of the current synchronous run, copied from the
    /// caller; empty between runs.
    #[codec(skip)]
    pub external_inputs: Vec<TimedInput>,
    #[codec(skip)]
    pub external_input_index: usize,
    pub sio_data: [u16; 4],
    pub sio_control: u16,
    pub sio_send: u16,
    pub sio_start_cycle: Option<Cycle>,
    pub sio_completion_cycle: Option<Cycle>,
    pub sio_receive_value: u32,
    pub sio_multiplayer_receive: [u16; 4],
    #[codec(bits = 2)]
    pub sio_multiplayer_id: u8,
    pub uart_tx_fifo: UartFifo,
    pub uart_rx_fifo: UartFifo,
    pub uart_tx_active: Option<u8>,
    pub uart_tx_completion_cycle: Option<Cycle>,
    pub uart_tx_peer_scheduled: bool,
    pub uart_rx_arrival_cycle: Option<Cycle>,
    pub uart_rx_arrival_value: u8,
    pub uart_error: bool,
    pub uart_cts_ready: bool,
    pub rcnt: u16,
    pub joy_control: u16,
    pub joy_receive: u32,
    pub joy_transmit: u32,
    pub joy_status: u16,
    pub joy_pending_command: Option<JoyBusCommand>,
    pub joy_completion_cycle: Option<Cycle>,
    pub joy_response: Option<JoyBusResponse>,
    pub dma_start_cycles: [Option<Cycle>; 4],
    #[codec(bits = 4)]
    pub pending_dma: u8,
    #[codec(skip)]
    pub servicing_dma: bool,
    #[codec(skip)]
    pub dma_burst_start: Cycle,
    /// A burst that still owns the bus yielded it to the host: at
    /// `MAX_DMA_BURST_CYCLES` inside a CPU access, or at a host deadline in
    /// `continue_dma_stall`. `servicing_dma` stays set until it ends.
    #[codec(skip)]
    pub dma_stalled: bool,
    /// Where the running burst yields: its first transfer start or beat
    /// boundary at or after this cycle.
    #[codec(skip)]
    pub dma_yield_at: Cycle,
    /// The transfers a stalled burst interrupted, one per channel, and
    /// their channels as a mask.
    #[codec(skip)]
    pub dma_progress: [Option<DmaProgress>; 4],
    #[codec(skip)]
    pub dma_progress_mask: u8,
    #[codec(skip)]
    pub active_dma_channel: Option<u8>,
    pub dma_transfer_latch: u32,
    pub open_bus_value: u32,
    /// The word of the most recent opcode fetch from the BIOS image, which
    /// reads of the BIOS return while the CPU runs elsewhere; in HLE mode,
    /// the value the HLE BIOS sets.
    pub bios_latch: u32,
    /// Whether the most recent opcode fetch was from the BIOS, which opens
    /// the image to data reads. Not durable: a state load
    /// derives it from the CPU ([`crate::cpu::Cpu::last_fetch_in_bios`]).
    #[codec(skip)]
    pub bios_code_active: bool,
    #[codec(skip)]
    pub current_cycle: Cycle,
    #[codec(skip)]
    pub batch_active: bool,
    #[codec(skip)]
    pub batch_device_cycle: Cycle,
    #[codec(skip)]
    pub batch_deadline: Cycle,
    #[codec(skip)]
    pub devices_deferred: bool,
    #[codec(skip)]
    pub apu_cycle: Cycle,
    #[codec(skip)]
    pub timer_cycle: Cycle,
    #[codec(skip)]
    pub timer_deadline: Cycle,
    #[codec(skip)]
    pub fifo_clock: crate::apu::FifoClock,
    #[codec(skip)]
    pub dma_epoch: u32,
}

impl Bus {
    pub fn init(rom: &[u8]) -> Self {
        Self::init_with_bios_and_save(rom, &[], save::Type::Sram)
    }

    pub fn init_with_save(rom: &[u8], save_type: save::Type) -> Self {
        Self::init_with_bios_and_save(rom, &[], save_type)
    }

    pub fn init_with_save_data(rom: &[u8], save_type: save::Type, data: &[u8]) -> Self {
        let mut bus = Self::init_with_save(rom, save_type);
        bus.save = SaveDevice::init_with_data(save_type, data);
        bus
    }

    pub fn init_with_bios(rom: &[u8], bios: &[u8]) -> Self {
        Self::init_with_bios_and_save(rom, bios, save::Type::Sram)
    }

    pub fn init_with_bios_and_save(rom: &[u8], bios: &[u8], save_type: save::Type) -> Self {
        Self::init_shared(Media::from_slice(rom), Media::from_slice(bios), save_type)
    }

    fn init_shared(rom: Media, bios: Media, save_type: save::Type) -> Self {
        assert!(
            bios.is_empty() || bios.len() == BIOS_SIZE,
            "external GBA BIOS must be exactly 16 KiB"
        );
        Self {
            ewram: boxed_array(0),
            iwram: boxed_array(0),
            code: crate::cpu::engine::CodeCache::default(),
            palette: [0; PALETTE_SIZE],
            vram: boxed_array(0),
            oam: [0; OAM_SIZE],
            save: SaveDevice::init(save_type),
            frame_pixels: boxed_array(0),
            frame_priorities: boxed_array(4),
            frame_layers: boxed_array(5),
            frame_second_pixels: boxed_array(0),
            frame_second_priorities: boxed_array(4),
            frame_second_layers: boxed_array(5),
            frame_semitransparent: boxed_array(false),
            frame_obj_window: boxed_array(false),
            line_needs_second_surface: true,
            line_has_color_effects: true,
            line_semitransparent_obj: LineScan::Unknown,
            line_window_masks: WindowMasks([0x3f; SCREEN_WIDTH]),
            line_unmasked_layers: 0x3f,
            line_window_key: 1 << 63,
            tile_row_cache: Box::default(),
            line_layers: Box::default(),
            scanline_memo: Box::default(),
            video_active_interval: 1,
            video_requested_interval: 1,
            video_skip_remaining: 0,
            video_render_current: true,
            video_framebuffer_canonical: true,
            video_publication_active: false,
            video_published_pixels: boxed_array(0),
            video_frame_id: 0,
            video_frame_cycle: 0,
            rom,
            bios,
            cartridge_gpio: None,
            cartridge_tilt: None,
            timers: std::array::from_fn(|_| Timer::default()),
            prescaler_origin: 0,
            dma_channels: std::array::from_fn(|_| DmaChannel::default()),
            apu: Apu::default(),
            ppu_timing: PpuTiming::default(),
            dispcnt: 0x0080,
            green_swap_enabled: false,
            bg_controls: [0; 4],
            bg_horizontal_offsets: [0; 4],
            bg_vertical_offsets: [0; 4],
            scroll_segments: [[ScrollSegment::default(); MAX_SCROLL_SEGMENTS]; 4],
            scroll_segment_counts: [0; 4],
            scroll_segment_line: 0xffff,
            backdrop_segments: [BackdropSegment::default(); MAX_SCROLL_SEGMENTS],
            backdrop_segment_count: 0,
            backdrop_segment_line: 0xffff,
            win_segments: [WindowSegment::default(); MAX_SCROLL_SEGMENTS],
            win_segment_count: 0,
            win_segment_line: 0xffff,
            blend_segments: [BlendSegment::default(); MAX_SCROLL_SEGMENTS],
            blend_segment_count: 0,
            blend_segment_line: 0xffff,
            affine_segments: [[AffineSegment::default(); MAX_SCROLL_SEGMENTS]; 2],
            affine_segment_counts: [0; 2],
            affine_segment_line: 0xffff,
            mosaic_segments: [MosaicSegment::default(); MAX_SCROLL_SEGMENTS],
            mosaic_segment_count: 0,
            mosaic_segment_line: 0xffff,
            bg_affine_parameters: [[0x100, 0, 0, 0x100]; 2],
            bg_reference_points: [[0; 2]; 2],
            bg_affine_current: [[0; 2]; 2],
            win0h: 0,
            win1h: 0,
            win0v: 0,
            win1v: 0,
            winin: 0,
            winout: 0,
            mosaic: 0,
            bldcnt: 0,
            bldalpha: 0,
            bldy: 0,
            completed_frame_count: 0,
            irq: IrqRegisters::default(),
            irq_arrival_moved: false,
            waitcnt: 0,
            game_pak_timing_key: 0,
            game_pak_n_cycles: [5; 3],
            game_pak_s_cycles: [3, 5, 9],
            game_pak_prefetch: GamePakPrefetch::default(),
            duty_valid_until: 0,
            fetch_kind: AccessKind::Nonsequential,
            oam_compact: [CompactObject::default(); 128],
            oam_compact_count: 0,
            oam_compact_valid: false,
            object_line: [ObjectLineData::default(); 128],
            object_line_count: 0,
            object_line_index: 0xffff,
            game_pak_code_active: false,
            prefetch_disabled_bug_pending: false,
            prefetch_buffer_was_disabled: false,
            keys: KeyState::default(),
            keypad_control: 0,
            keypad_condition_met: false,
            external_inputs: Vec::with_capacity(EXTERNAL_INPUT_CAPACITY),
            external_input_index: 0,
            sio_data: [0; 4],
            sio_control: 0,
            sio_send: 0,
            sio_start_cycle: None,
            sio_completion_cycle: None,
            sio_receive_value: 0xffff_ffff,
            sio_multiplayer_receive: [0xffff; 4],
            sio_multiplayer_id: 0,
            uart_tx_fifo: UartFifo::default(),
            uart_rx_fifo: UartFifo::default(),
            uart_tx_active: None,
            uart_tx_completion_cycle: None,
            uart_tx_peer_scheduled: false,
            uart_rx_arrival_cycle: None,
            uart_rx_arrival_value: 0,
            uart_error: false,
            uart_cts_ready: false,
            rcnt: 0x8000,
            joy_control: 0,
            joy_receive: 0,
            joy_transmit: 0,
            joy_status: 0,
            joy_pending_command: None,
            joy_completion_cycle: None,
            joy_response: None,
            dma_start_cycles: [None; 4],
            pending_dma: 0,
            servicing_dma: false,
            dma_burst_start: 0,
            dma_stalled: false,
            dma_yield_at: 0,
            dma_progress: [None; 4],
            dma_progress_mask: 0,
            active_dma_channel: None,
            dma_transfer_latch: 0,
            open_bus_value: 0,
            bios_latch: 0xe129_f000,
            bios_code_active: false,
            current_cycle: 0,
            batch_active: false,
            batch_device_cycle: 0,
            batch_deadline: 0,
            devices_deferred: false,
            apu_cycle: 0,
            timer_cycle: 0,
            timer_deadline: 0,
            fifo_clock: crate::apu::FifoClock::default(),
            dma_epoch: 0,
        }
    }

    pub fn init_with_bios_and_save_data(
        rom: &[u8],
        bios: &[u8],
        save_type: save::Type,
        data: &[u8],
    ) -> Self {
        let mut bus = Self::init_with_bios_and_save(rom, bios, save_type);
        bus.save = SaveDevice::init_with_data(save_type, data);
        bus
    }

    /// Initialize a Bus using already-owned media without copying bytes.
    pub fn init_with_media(
        rom: Media,
        bios: Media,
        save_type: save::Type,
        save_data: Option<&[u8]>,
    ) -> Self {
        let mut bus = Self::init_shared(rom, bios, save_type);
        if let Some(data) = save_data {
            bus.save = SaveDevice::init_with_data(save_type, data);
        }
        bus
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    pub fn bios(&self) -> &[u8] {
        &self.bios
    }

    pub fn write_state(&self, sink: &mut evenfall_state::Sink<'_>) -> evenfall_state::Result<()> {
        use evenfall_state::Error;
        if self.servicing_dma
            || self.active_dma_channel.is_some()
            || self.dma_stalled
            || !self.external_inputs.is_empty()
            || self.external_input_index != 0
        {
            return Err(Error::InvalidSaveState);
        }
        self.validate_state()?;
        self.apu.write_state(&mut evenfall_state::Sink::new(None, None))?;
        self.encode(sink)
    }

    pub fn write_state_schema(sink: &mut evenfall_state::Sink<'_>) -> evenfall_state::Result<()> {
        Self::schema(sink)
    }

    pub fn read_state(
        &mut self,
        source: &mut evenfall_state::Source<'_>,
    ) -> evenfall_state::Result<()> {
        // Decode into fresh transient APU storage; Codec visits durable APU
        // fields only, and never decodes its host sample queue.
        let sample_output_enabled = self.apu.sample_output_enabled;
        let high_speed_audio_enabled = self.apu.high_speed_audio_enabled;
        self.apu = Apu::default();
        self.apu.sample_output_enabled = sample_output_enabled;
        self.apu.high_speed_audio_enabled = high_speed_audio_enabled;
        self.decode(source)?;
        self.code.clear();
        self.external_inputs.clear();
        self.external_input_index = 0;
        self.servicing_dma = false;
        self.clear_dma_stall();
        self.active_dma_channel = None;
        self.apu_cycle = self.current_cycle;
        self.timer_cycle = self.current_cycle;
        self.duty_valid_until = 0;
        self.frame_priorities.fill(4);
        self.frame_layers.fill(5);
        self.frame_second_pixels.fill(0);
        self.frame_second_priorities.fill(4);
        self.frame_second_layers.fill(5);
        self.frame_semitransparent.fill(false);
        self.frame_obj_window.fill(false);
        self.oam_compact_valid = false;
        self.object_line_index = 0xffff;
        self.scanline_memo.invalidate();
        self.reset_video_policy(1);
        self.irq.irq_line = self.irq.request() && self.irq.irq_line_rises_at.is_none();
        self.validate_state()?;
        self.apu.write_state(&mut evenfall_state::Sink::new(None, None))?;
        Ok(())
    }

    /// Makes this bus a copy of `source`, reusing this bus's allocations.
    /// Both must hold the same ROM and BIOS, which are not copied, and
    /// neither may be inside a run. Everything else is copied, transient
    /// timing and render caches included, so the copy continues exactly as
    /// `source` would. As after any wholesale memory change, decoded code is
    /// forgotten and the scanline memo invalidated.
    pub(crate) fn copy_from(&mut self, source: &Self) {
        debug_assert!(self.rom.len() == source.rom.len() && self.bios.len() == source.bios.len());
        debug_assert!(source.external_inputs.is_empty());
        clone_fields_from!(
            self,
            source,
            [
                ewram, iwram, palette, vram, oam, save, frame_pixels, frame_priorities,
                frame_layers, frame_second_pixels, frame_second_priorities, frame_second_layers,
                frame_semitransparent, frame_obj_window, line_needs_second_surface,
                line_has_color_effects, line_semitransparent_obj, line_window_masks,
                line_unmasked_layers, line_window_key, tile_row_cache, line_layers,
                scanline_memo, video_active_interval, video_requested_interval,
                video_skip_remaining, video_render_current, video_framebuffer_canonical,
                video_publication_active, video_published_pixels, video_frame_id,
                video_frame_cycle, cartridge_gpio, cartridge_tilt, timers, prescaler_origin,
                dma_channels, apu, ppu_timing, dispcnt, green_swap_enabled, bg_controls,
                bg_horizontal_offsets, bg_vertical_offsets, scroll_segments,
                scroll_segment_counts, scroll_segment_line, backdrop_segments,
                backdrop_segment_count, backdrop_segment_line, win_segments, win_segment_count,
                win_segment_line, blend_segments, blend_segment_count, blend_segment_line,
                affine_segments, affine_segment_counts, affine_segment_line, mosaic_segments,
                mosaic_segment_count, mosaic_segment_line, bg_affine_parameters,
                bg_reference_points, bg_affine_current, win0h, win1h, win0v, win1v, winin,
                winout, mosaic, bldcnt, bldalpha, bldy, completed_frame_count, irq,
                irq_arrival_moved, waitcnt,
                game_pak_timing_key, game_pak_n_cycles, game_pak_s_cycles, game_pak_prefetch,
                duty_valid_until, fetch_kind, oam_compact, oam_compact_count, oam_compact_valid,
                object_line, object_line_count, object_line_index, game_pak_code_active,
                prefetch_disabled_bug_pending, prefetch_buffer_was_disabled, keys,
                keypad_control, keypad_condition_met, external_inputs, external_input_index,
                sio_data, sio_control, sio_send, sio_start_cycle, sio_completion_cycle,
                sio_receive_value, sio_multiplayer_receive, sio_multiplayer_id, uart_tx_fifo,
                uart_rx_fifo, uart_tx_active, uart_tx_completion_cycle, uart_tx_peer_scheduled,
                uart_rx_arrival_cycle, uart_rx_arrival_value, uart_error, uart_cts_ready, rcnt,
                joy_control, joy_receive, joy_transmit, joy_status, joy_pending_command,
                joy_completion_cycle, joy_response, dma_start_cycles, pending_dma,
                servicing_dma, dma_burst_start, dma_stalled, dma_yield_at, dma_progress,
                dma_progress_mask, active_dma_channel,
                dma_transfer_latch, open_bus_value, bios_latch, bios_code_active, current_cycle,
                batch_active, batch_device_cycle, batch_deadline, devices_deferred, apu_cycle,
                timer_cycle, timer_deadline, fifo_clock, dma_epoch,
            ],
            keep [code, rom, bios]
        );
        self.code.clear();
        self.scanline_memo.invalidate();
    }

    fn validate_state(&self) -> evenfall_state::Result<()> {
        use evenfall_state::Error::InvalidSaveState;
        if self.ppu_timing.line >= 228 || self.ppu_timing.cycle_in_line >= 1232 {
            return Err(InvalidSaveState);
        }
        let prescaler = self.prescaler_counter();
        let timer_zero = &self.timers[0];
        let zero_without_count_up = Timer {
            control: timer_zero.control & !(1 << 2),
            counter: timer_zero.counter,
            reload: timer_zero.reload,
            prescaler_phase: timer_zero.prescaler_phase,
            starting: timer_zero.starting,
        };
        zero_without_count_up.validate_state(prescaler).map_err(|_| InvalidSaveState)?;
        for timer in &self.timers[1..] {
            timer.validate_state(prescaler).map_err(|_| InvalidSaveState)?;
        }
        if let Some(gpio) = &self.cartridge_gpio {
            gpio.validate_state(self.current_cycle).map_err(|_| InvalidSaveState)?;
        }
        if let Some(at) = self.irq.irq_line_rises_at
            && (!self.irq.request()
                || at <= self.current_cycle
                || at > self.current_cycle + IRQ_LATENCY)
        {
            return Err(InvalidSaveState);
        }
        for deadline in [
            self.sio_completion_cycle,
            self.uart_tx_completion_cycle,
            self.uart_rx_arrival_cycle,
            self.joy_completion_cycle,
        ] {
            if deadline.is_some_and(|d| d <= self.current_cycle) {
                return Err(InvalidSaveState);
            }
        }
        for deadline in self.dma_start_cycles {
            if deadline.is_some_and(|d| d <= self.current_cycle) {
                return Err(InvalidSaveState);
            }
        }
        if self.joy_pending_command.is_some() != self.joy_completion_cycle.is_some() {
            return Err(InvalidSaveState);
        }
        if self.scroll_segment_counts.iter().any(|&c| usize::from(c) > MAX_SCROLL_SEGMENTS)
            || self.affine_segment_counts.iter().any(|&c| usize::from(c) > MAX_SCROLL_SEGMENTS)
            || [
                self.backdrop_segment_count,
                self.win_segment_count,
                self.blend_segment_count,
                self.mosaic_segment_count,
            ]
            .iter()
            .any(|&c| usize::from(c) > MAX_SCROLL_SEGMENTS)
        {
            return Err(InvalidSaveState);
        }
        let mut index = 0usize;
        while index < usize::from(self.backdrop_segment_count) {
            let segment = self.backdrop_segments[index];
            if segment.start_x == 0xffff {
                if index + 1 >= usize::from(self.backdrop_segment_count) {
                    return Err(InvalidSaveState);
                }
                let payload = self.backdrop_segments[index + 1];
                let vram_index =
                    usize::from(payload.start_x) | (usize::from(payload.color & 1) << 16);
                if (segment.color & 0xff) >= SCREEN_WIDTH as u16
                    || payload.color > 0x1ff
                    || vram_index >= VRAM_SIZE
                {
                    return Err(InvalidSaveState);
                }
                index += 2;
            } else {
                if (segment.start_x & 0xff) >= SCREEN_WIDTH as u16 {
                    return Err(InvalidSaveState);
                }
                index += 1;
            }
        }
        if usize::from(self.uart_tx_fifo.count) > self.uart_tx_fifo.bytes.len()
            || usize::from(self.uart_rx_fifo.count) > self.uart_rx_fifo.bytes.len()
            || self
                .joy_response
                .is_some_and(|response| usize::from(response.len) > response.data.len())
            || self.game_pak_prefetch.count > self.game_pak_prefetch.capacity
            || self.game_pak_prefetch.capacity > 8
        {
            return Err(InvalidSaveState);
        }
        Ok(())
    }

    pub fn read(&mut self, access: Access) -> ReadResult {
        if self.batch_active && (4..8).contains(&(access.address >> 24)) {
            self.flush_device_batch();
            self.batch_device_cycle = self.current_cycle;
            if self.batch_deadline <= self.current_cycle {
                self.batch_deadline = self.next_device_boundary();
            }
        }
        match access.source {
            AccessSource::CpuCode => {
                self.fetch_kind = AccessKind::Sequential;
                self.bios_code_active = access.address < BIOS_SIZE as u32;
            }
            AccessSource::CpuData => self.fetch_kind = AccessKind::Nonsequential,
            AccessSource::Dma => {}
        }
        if access.source == AccessSource::CpuCode && !(8..=13).contains(&(access.address >> 24)) {
            self.game_pak_code_active = false;
            self.game_pak_prefetch = GamePakPrefetch::default();
        }
        let address = access.address;
        let width = access.width;
        match address >> 24 {
            0 if address < BIOS_SIZE as u32 => {
                let value = self.read_protected_bios(address, width, access.source);
                self.complete_access(access, value, 1)
            }
            2 => self.complete_access(access, self.read_ewram(address, width), ewram_cycles(width)),
            3 => self.complete_access(access, self.read_iwram(address, width), 1),
            4 => {
                let value = self.read_mmio(address, width);
                self.complete_read(value, 1)
            }
            5 => {
                self.complete_access(access, self.read_palette(address, width), video_cycles(width))
            }
            6 => self.complete_access(access, self.read_vram(address, width), video_cycles(width)),
            7 => self.complete_access(access, self.read_oam(address, width), 1),
            8..=13
                if width == AccessWidth::Half
                    && self.save.is_eeprom()
                    && self.is_eeprom_address(address) =>
            {
                let started_at = self.current_cycle;
                self.stop_game_pak_prefetch(Some(access.source));
                let value = u32::from(self.save.read_eeprom_bit(self.current_cycle));
                let cycles = self.game_pak_cycles(access);
                let mut result = self.complete_read(value, cycles);
                result.cycles = self.current_cycle - started_at;
                result
            }
            8..=13 => self.read_game_pak(access),
            14..=15 => {
                let started_at = self.current_cycle;
                self.stop_game_pak_prefetch(Some(access.source));
                let offset = address & 0x00ff_ffff;
                let byte = if let Some(tilt) = &self.cartridge_tilt {
                    if tilt::Tilt::accepts_read(offset) {
                        tilt.read(offset)
                    } else {
                        self.save.read_byte(address as u16, self.current_cycle)
                    }
                } else {
                    self.save.read_byte(address as u16, self.current_cycle)
                };
                let value = match width {
                    AccessWidth::Byte => u32::from(byte),
                    AccessWidth::Half => u32::from(byte) * 0x0101,
                    AccessWidth::Word => u32::from(byte) * 0x0101_0101,
                };
                let mut result = self.complete_read(value, self.sram_cycles());
                result.cycles = self.current_cycle - started_at;
                result
            }
            _ => {
                let shift = match width {
                    AccessWidth::Byte => (address & 3) * 8,
                    AccessWidth::Half => (address & 2) * 8,
                    AccessWidth::Word => 0,
                };
                let mask = match width {
                    AccessWidth::Byte => 0xff,
                    AccessWidth::Half => 0xffff,
                    AccessWidth::Word => u32::MAX,
                };
                self.complete_read((self.open_bus_value >> shift) & mask, 1)
            }
        }
    }

    #[inline(always)]
    pub fn write(&mut self, access: Access, value: u32) -> WriteResult {
        let started_at = self.current_cycle;
        let address = access.address;
        let width = access.width;
        let region = address >> 24;
        if access.source == AccessSource::CpuData {
            self.fetch_kind = AccessKind::Nonsequential;
        }
        match region {
            2 => {
                self.advance(ewram_cycles(width));
                self.write_ewram(address, width, value);
            }
            3 => {
                self.advance(1);
                self.write_iwram(address, width, value);
            }
            4 => {
                let batch = self.batch_active;
                if batch {
                    self.flush_device_batch();
                    self.batch_active = false;
                    self.batch_deadline = 0;
                }
                self.advance(1);
                self.write_mmio(address, width, value);
                if batch {
                    self.batch_active = true;
                    self.batch_device_cycle = self.current_cycle;
                    self.batch_deadline = self.next_device_boundary();
                }
            }
            5 => {
                if width == AccessWidth::Word {
                    // The two palette halfwords become visible on separate cycles.
                    for beat in 0..2 {
                        let at = address.wrapping_add(beat * 2);
                        self.advance(1);
                        self.flush_device_batch();
                        self.begin_palette_segments(at, AccessWidth::Half);
                        self.write_palette(at, AccessWidth::Half, value >> (beat * 16));
                        self.end_palette_segments(at, AccessWidth::Half);
                    }
                } else {
                    self.advance(video_cycles(width));
                    self.flush_device_batch();
                    self.begin_palette_segments(address, width);
                    self.write_palette(address, width, value);
                    self.end_palette_segments(address, width);
                }
            }
            6 => self.write_vram_timed(address, width, value),
            7 => {
                self.advance(1);
                self.write_oam(address, width, value);
            }
            8..=13 if width == AccessWidth::Half && self.is_eeprom_access(address) => {
                self.stop_game_pak_prefetch(Some(access.source));
                let cycles = self.game_pak_cycles(access);
                self.advance(cycles);
                self.save.write_eeprom_bit(value as u8, self.current_cycle);
            }
            8..=13 => {
                self.stop_game_pak_prefetch(Some(access.source));
                let cycles = self.game_pak_cycles(access);
                self.advance(cycles);
                if self.is_cartridge_gpio_write(access) {
                    match width {
                        AccessWidth::Byte => {}
                        AccessWidth::Half => self.write_cartridge_gpio_half(address, value as u16),
                        AccessWidth::Word => {
                            self.write_cartridge_gpio_half(address, value as u16);
                            self.write_cartridge_gpio_half(
                                address.wrapping_add(2),
                                (value >> 16) as u16,
                            );
                        }
                    }
                }
            }
            14..=15 => {
                self.stop_game_pak_prefetch(Some(access.source));
                self.advance(self.sram_cycles());
                let shift = match width {
                    AccessWidth::Byte => 0,
                    AccessWidth::Half => (address & 1) * 8,
                    AccessWidth::Word => (address & 3) * 8,
                };
                let byte = (value >> shift) as u8;
                let offset = address & 0x00ff_ffff;
                if let Some(tilt) = &mut self.cartridge_tilt {
                    if tilt::Tilt::accepts_write(offset) {
                        tilt.write(offset, byte);
                    } else {
                        self.save.write_byte(address as u16, byte, self.current_cycle);
                    }
                } else {
                    self.save.write_byte(address as u16, byte, self.current_cycle);
                }
            }
            _ => self.advance(1),
        }
        self.service_pending_dma();
        WriteResult { cycles: self.current_cycle - started_at }
    }

    pub fn elapse(&mut self, cycles: Cycle) {
        self.advance(cycles);
        self.service_pending_dma();
    }

    pub fn elapse_cpu_internal(&mut self, cycles: Cycle) {
        if self.waitcnt & (1 << 14) == 0 && self.game_pak_code_active {
            self.prefetch_disabled_bug_pending = true;
        }
        self.elapse(cycles);
    }

    pub fn read_cpu_data(&mut self, address: u32, width: AccessWidth, kind: AccessKind) -> u32 {
        self.read(Access { address, width, kind, source: AccessSource::CpuData }).value
    }

    pub fn write_cpu_data(&mut self, access: Access, value: u32) {
        debug_assert_eq!(access.source, AccessSource::CpuData);
        self.write(access, value);
    }

    pub fn write_cpu_word(&mut self, address: u32, value: u32, kind: AccessKind) {
        self.write_cpu_data(
            Access { address, width: AccessWidth::Word, kind, source: AccessSource::CpuData },
            value,
        );
    }

    /// Return a block's charge only if every beat and its internal cycle fit
    /// strictly before the armed device deadline in a single ordinary RAM image.
    pub fn cpu_ram_block_cycles(&self, first: u32, beats: u32, internal: Cycle) -> Option<Cycle> {
        debug_assert!(first & 3 == 0 && beats != 0);
        let last = first.wrapping_add((beats - 1).wrapping_mul(4));
        if last < first || last >> 24 != first >> 24 {
            return None;
        }
        let cost: Cycle = match first >> 24 {
            2 => 6,
            3 => 1,
            _ => return None,
        };
        let cycles = cost * Cycle::from(beats) + internal;
        if !self.batch_active
            || self.current_cycle.saturating_add(cycles) >= self.batch_deadline
            || self.pending_dma != 0
        {
            return None;
        }
        Some(cycles)
    }

    pub fn cpu_ram_word(&self, address: u32) -> u32 {
        debug_assert_eq!(address & 3, 0);
        if address >> 24 == 3 {
            self.read_iwram(address, AccessWidth::Word)
        } else {
            self.read_ewram(address, AccessWidth::Word)
        }
    }

    pub fn set_cpu_ram_word(&mut self, address: u32, value: u32) {
        debug_assert_eq!(address & 3, 0);
        if address >> 24 == 3 {
            self.write_iwram(address, AccessWidth::Word, value);
        } else {
            self.write_ewram(address, AccessWidth::Word, value);
        }
    }

    pub fn finish_cpu_ram_load_block(&mut self, _cycles: Cycle, _last_word: u32) {
        self.fetch_kind = AccessKind::Nonsequential;
        self.open_bus_value = _last_word;
        if self.waitcnt & (1 << 14) == 0 && self.game_pak_code_active {
            self.prefetch_disabled_bug_pending = true;
        }
        self.current_cycle += _cycles;
        self.advance_game_pak_prefetch(_cycles);
    }

    pub fn finish_cpu_ram_store_block(&mut self, _cycles: Cycle) {
        self.fetch_kind = AccessKind::Nonsequential;
        self.current_cycle += _cycles;
        self.advance_game_pak_prefetch(_cycles);
    }

    pub fn fetch_instruction(&mut self, address: u32, width: AccessWidth) -> u32 {
        self.read(Access { address, width, kind: self.fetch_kind, source: AccessSource::CpuCode })
            .value
    }

    #[inline(always)]
    pub fn fetch_instruction_in_region(
        &mut self,
        region: &mut InstructionFetchRegion,
        address: u32,
        width: AccessWidth,
    ) -> u32 {
        if address.wrapping_sub(region.address) >= region.valid_starts {
            *region = self.instruction_fetch_region(address);
        }
        // read() remains the reference fetch path, including prefetch, DMA,
        // and open-bus completion. The view also admits guarded fused refills.
        self.fetch_instruction(address, width)
    }

    fn instruction_fetch_region(&self, address: u32) -> InstructionFetchRegion {
        if address >> 24 == 3 {
            return InstructionFetchRegion {
                address: address & 0xff00_0000,
                valid_starts: 0x0100_0000,
                region_class: InstructionFetchClass::Iwram,
                ..InstructionFetchRegion::default()
            };
        }
        if !(8..=13).contains(&(address >> 24)) {
            return InstructionFetchRegion {
                address: address & 0xff00_0000,
                valid_starts: 0x0100_0000,
                ..InstructionFetchRegion::default()
            };
        }
        let offset = (address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1);
        let window_base = address - offset;
        let mut first = 0;
        let mut end = self.rom.len().min(GAME_PAK_WINDOW_SIZE as usize) as u32;
        if offset < 0xc4 {
            end = end.min(0xc4);
        } else {
            first = 0xca;
        }
        if window_base == 0x0c00_0000 {
            end =
                end.min(if self.rom.len() <= 16 * 1024 * 1024 { 0x0100_0000 } else { 0x01ff_ff00 });
        }
        if offset < first || end < first + 4 || offset > end - 4 {
            return InstructionFetchRegion::default();
        }
        InstructionFetchRegion {
            bytes: Some(self.rom.as_ptr().wrapping_add(first as usize)),
            address: window_base + first,
            valid_starts: end - first - 3,
            region_class: InstructionFetchClass::Rom,
        }
    }

    pub fn fetch_thumb_in_region(
        &mut self,
        region: &mut InstructionFetchRegion,
        address: u32,
        stream: &mut ThumbStream,
    ) -> u32 {
        stream.armed = false;
        self.fetch_instruction_in_region(region, address, AccessWidth::Half)
    }

    pub fn fetch_thumb_refill_in_region(
        &mut self,
        region: &mut InstructionFetchRegion,
        address: u32,
    ) -> [u32; 2] {
        if let Some(words) = self.fused_thumb_game_pak_refill(region, address) {
            return words;
        }
        self.fetch_kind = AccessKind::Nonsequential;
        let first = self.fetch_instruction_in_region(region, address, AccessWidth::Half);
        self.fetch_kind = AccessKind::Sequential;
        let second =
            self.fetch_instruction_in_region(region, address.wrapping_add(2), AccessWidth::Half);
        [first, second]
    }

    pub fn fused_thumb_game_pak_refill(
        &mut self,
        region: &mut InstructionFetchRegion,
        address: u32,
    ) -> Option<[u32; 2]> {
        if address & 1 != 0 {
            return None;
        }
        if address.wrapping_sub(region.address) >= region.valid_starts {
            *region = self.instruction_fetch_region(address);
        }
        let offset = address.wrapping_sub(region.address);
        if region.bytes.is_none() || offset + 2 >= region.valid_starts {
            return None;
        }
        if self.waitcnt & (1 << 14) == 0 || self.game_pak_timing_key != self.waitcnt {
            return None;
        }
        let prefetch = &self.game_pak_prefetch;
        if prefetch.active
            && prefetch.opcode_bytes == 2
            && ((prefetch.count != 0 && address == prefetch.head_address)
                || (prefetch.countdown != 0 && address == prefetch.last_address))
        {
            return None;
        }
        let next = address + 2;
        let miss_cycles = self.game_pak_half_cycles(address, AccessKind::Nonsequential);
        let duty = self.prefetch_duty(next, 2);
        let end = self.current_cycle + miss_cycles + duty;
        if !self.batch_active || end >= self.batch_deadline || self.pending_dma != 0 {
            return None;
        }
        let index = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        let first = u32::from(u16::from_le_bytes(self.rom[index..index + 2].try_into().unwrap()));
        let second =
            u32::from(u16::from_le_bytes(self.rom[index + 2..index + 4].try_into().unwrap()));
        self.fetch_kind = AccessKind::Sequential;
        self.bios_code_active = false;
        self.game_pak_code_active = true;
        self.prefetch_disabled_bug_pending = false;
        self.prefetch_buffer_was_disabled = false;
        self.start_game_pak_prefetch(address, 2);
        debug_assert_eq!(self.game_pak_prefetch.countdown, duty);
        self.continue_game_pak_prefetch(2);
        self.game_pak_prefetch.head_address = next + 2;
        self.open_bus_value = half_on_both_lanes(second);
        self.current_cycle = end;
        Some([first, second])
    }

    pub fn begin_device_batch(&mut self) {
        debug_assert!(!self.batch_active);
        debug_assert_eq!(self.apu_cycle, self.current_cycle);
        debug_assert_eq!(self.timer_cycle, self.current_cycle);
        self.batch_device_cycle = self.current_cycle;
        self.batch_active = true;
        self.devices_deferred = true;
        self.refresh_fifo_clock();
        self.refresh_timer_deadline();
        self.batch_deadline = self.next_device_boundary();
    }

    pub fn end_device_batch(&mut self) {
        self.synchronize_devices();
        self.devices_deferred = false;
        self.fifo_clock = crate::apu::FifoClock::default();
        self.timer_deadline = 0;
        self.batch_active = false;
        self.batch_device_cycle = 0;
        self.batch_deadline = 0;
    }

    pub fn enter_halt(&mut self) {
        self.flush_device_batch();
        self.batch_deadline = 0;
        self.irq.power_state = PowerState::Halted;
    }

    pub fn enter_stop(&mut self) {
        self.synchronize_devices();
        self.batch_deadline = 0;
        self.irq.power_state = PowerState::Stopped;
    }

    pub fn idle_halt_until(&mut self, deadline: Cycle) -> bool {
        debug_assert_eq!(self.irq.power_state, PowerState::Halted);
        debug_assert!(deadline >= self.current_cycle);
        self.apply_due_external_inputs();
        while !self.halt_wake_pending() && self.current_cycle < deadline && !self.dma_stalled {
            let limit = deadline - self.current_cycle;
            let step = if self.batch_active && self.batch_deadline > self.current_cycle {
                limit.min(self.batch_deadline - self.current_cycle)
            } else {
                limit.min(self.next_device_boundary() - self.current_cycle)
            };
            self.elapse(step);
        }
        self.halt_wake_pending()
    }

    pub fn idle_stop_until(&mut self, deadline: Cycle) -> bool {
        debug_assert_eq!(self.irq.power_state, PowerState::Stopped);
        debug_assert!(deadline >= self.current_cycle);
        self.apply_due_external_inputs();
        while !self.stop_wake_pending() && self.current_cycle < deadline {
            let Some(serial_at) = [
                self.sio_completion_cycle,
                self.uart_tx_completion_cycle,
                self.uart_rx_arrival_cycle,
                self.joy_completion_cycle,
            ]
            .into_iter()
            .flatten()
            .min() else {
                return false;
            };
            let stop_at = serial_at.min(deadline);
            if self
                .next_external_input_cycle()
                .is_some_and(|at| at > self.current_cycle && at <= stop_at)
            {
                return false;
            }
            if stop_at > self.current_cycle {
                let device_point =
                    if self.batch_active { self.batch_device_cycle } else { self.current_cycle };
                if device_point > self.apu_cycle {
                    self.sync_apu_to(device_point);
                }
                if device_point > self.timer_cycle {
                    self.sync_timers_to(device_point);
                }
                self.prescaler_origin =
                    self.prescaler_origin.wrapping_add((stop_at - self.timer_cycle) as u16) & 1023;
                self.current_cycle = stop_at;
                if self.batch_active {
                    self.batch_device_cycle = stop_at;
                }
                self.apu_cycle = stop_at;
                self.timer_cycle = stop_at;
                self.refresh_fifo_clock();
                self.refresh_timer_deadline();
            }
            if self.sio_completion_cycle.is_some_and(|at| at <= stop_at) {
                self.complete_sio_transfer();
            }
            if self.uart_tx_completion_cycle.is_some_and(|at| at <= stop_at) {
                self.complete_uart_transmit();
            }
            if self.uart_rx_arrival_cycle.is_some_and(|at| at <= stop_at) {
                self.complete_uart_receive();
            }
            if self.joy_completion_cycle.is_some_and(|at| at <= stop_at) {
                self.complete_joy_bus_command();
            }
        }
        self.stop_wake_pending()
    }

    pub fn schedule_joy_bus(
        &mut self,
        command: JoyBusCommand,
        completion_cycle: Cycle,
    ) -> Result<(), JoyBusError> {
        if self.rcnt & 0xc000 != 0xc000 {
            return Err(JoyBusError::NotInJoyBusMode);
        }
        if completion_cycle < self.current_cycle {
            return Err(JoyBusError::CompletionBeforeCurrentCycle);
        }
        if self.joy_pending_command.is_some() || self.joy_response.is_some() {
            return Err(JoyBusError::TransactionPending);
        }
        self.joy_pending_command = Some(command);
        self.joy_completion_cycle = Some(completion_cycle);
        if completion_cycle == self.current_cycle {
            self.complete_joy_bus_command();
        }
        Ok(())
    }

    pub fn take_joy_bus_response(&mut self) -> Option<JoyBusResponse> {
        self.joy_response.take()
    }

    pub fn soft_reset_entry_address(&mut self) -> u32 {
        let reenter_ewram = self.iwram[0x7ffa] != 0;
        self.iwram[0x7e00..].fill(0);
        self.code.clear();
        if reenter_ewram { EWRAM_BASE } else { GAME_PAK_BASE }
    }

    pub fn register_ram_reset(&mut self, flags: u8) {
        self.synchronize_devices();
        self.batch_deadline = 0;
        self.dispcnt = 0x0080;
        self.sio_data[0] = 0;
        self.scanline_memo.mark_registers();

        if flags & 1 != 0 {
            self.ewram.fill(0);
        }
        if flags & 2 != 0 {
            self.iwram[..0x7e00].fill(0);
        }
        self.code.clear();
        if flags & 4 != 0 {
            self.palette.fill(0);
            self.scanline_memo.mark_palette();
        }
        if flags & 0x0c != 0 {
            self.backdrop_segment_line = 0xffff;
        }
        if flags & 8 != 0 {
            self.vram.fill(0);
            self.scanline_memo.mark_all_vram();
        }
        if flags & 0x10 != 0 {
            self.oam.fill(0);
            self.scanline_memo.mark_oam();
            self.oam_compact_valid = false;
            self.object_line_index = 0xffff;
        }
        if flags & 0x20 != 0 {
            self.sio_control = 0;
            self.rcnt = 0x8000;
            self.sio_send = 0;
            self.sio_start_cycle = None;
            self.sio_completion_cycle = None;
            self.sio_receive_value = u32::MAX;
            self.sio_multiplayer_receive = [0xffff; 4];
            self.sio_multiplayer_id = 0;
            self.clear_uart_state();
            self.joy_control = 0;
            self.joy_receive = 0;
            self.joy_transmit = 0;
            self.joy_status = 0;
            self.joy_pending_command = None;
            self.joy_completion_cycle = None;
            self.joy_response = None;
        }
        if flags & 0x40 != 0 {
            self.apu.reset_registers();
            self.refresh_timer_deadline();
        }
        if flags & 0x80 != 0 {
            self.reset_general_registers();
        }
    }

    pub fn hard_reset(&mut self) {
        self.register_ram_reset(0xff);
        self.iwram[0x7e00..].fill(0);
        self.code.clear();
        self.frame_pixels.fill(0);
        self.frame_second_pixels.fill(0);
        self.frame_priorities.fill(4);
        self.frame_layers.fill(5);
        self.frame_second_priorities.fill(4);
        self.frame_second_layers.fill(5);
        self.frame_semitransparent.fill(false);
        self.frame_obj_window.fill(false);
        self.scanline_memo.invalidate();
        self.ppu_timing = PpuTiming::default();
        self.green_swap_enabled = false;
        let sample_output_enabled = self.apu.sample_output_enabled;
        let high_speed_audio_enabled = self.apu.high_speed_audio_enabled;
        self.apu = Apu::default();
        self.apu.sample_output_enabled = sample_output_enabled;
        self.apu.high_speed_audio_enabled = high_speed_audio_enabled;
        self.apu.write_half(MMIO_BASE + 0x82, 0x880e);
        self.keypad_control = 0;
        self.keypad_condition_met = false;
        self.sio_data = [0; 4];
        self.irq.post_boot = 1;
        self.irq.power_state = PowerState::Running;
        self.game_pak_prefetch = GamePakPrefetch::default();
        self.duty_valid_until = 0;
        self.game_pak_code_active = false;
        self.prefetch_disabled_bug_pending = false;
        self.prefetch_buffer_was_disabled = false;
        self.open_bus_value = 0;
        self.reset_video_policy(self.video_requested_interval);
    }

    fn reset_general_registers(&mut self) {
        self.ppu_timing.write_dispstat(0);
        self.bg_controls = [0; 4];
        self.bg_horizontal_offsets = [0; 4];
        self.bg_vertical_offsets = [0; 4];
        self.scroll_segment_line = 0xffff;
        self.scroll_segment_counts = [0; 4];
        self.backdrop_segment_line = 0xffff;
        self.backdrop_segment_count = 0;
        self.bg_affine_parameters = [[0x100, 0, 0, 0x100]; 2];
        self.bg_reference_points = [[0; 2]; 2];
        self.bg_affine_current = [[0; 2]; 2];
        self.win0h = 0;
        self.win1h = 0;
        self.win0v = 0;
        self.win1v = 0;
        self.winin = 0;
        self.winout = 0;
        self.win_segment_line = 0xffff;
        self.win_segment_count = 0;
        self.mosaic = 0;
        self.mosaic_segment_line = 0xffff;
        self.mosaic_segment_count = 0;
        self.bldcnt = 0;
        self.bldalpha = 0;
        self.bldy = 0;
        self.blend_segment_line = 0xffff;
        self.blend_segment_count = 0;
        self.affine_segment_line = 0xffff;
        self.affine_segment_counts = [0; 2];
        self.dma_channels = std::array::from_fn(|_| DmaChannel::default());
        self.timers = std::array::from_fn(|_| Timer::default());
        self.refresh_fifo_clock();
        self.refresh_timer_deadline();
        self.dma_start_cycles = [None; 4];
        self.pending_dma = 0;
        self.servicing_dma = false;
        self.clear_dma_stall();
        self.active_dma_channel = None;
        self.dma_transfer_latch = 0;
        self.irq.interrupt_enable_mask = 0;
        self.irq.interrupt_request_flags = 0;
        self.irq.interrupt_master_enabled = false;
        self.update_irq_line();
        self.write_waitcnt(0);
    }

    pub fn inspect_byte(&self, address: u32) -> u8 {
        match address >> 24 {
            0 if address < BIOS_SIZE as u32 => self.read_bios(address, AccessWidth::Byte) as u8,
            2 => self.read_ewram(address, AccessWidth::Byte) as u8,
            3 => self.read_iwram(address, AccessWidth::Byte) as u8,
            4 => {
                let aligned = address & !1;
                if aligned == MMIO_BASE + 0x12a && self.is_uart_mode() && address & 1 != 0 {
                    return 0;
                }
                (self.peek_mmio_half(aligned).unwrap_or_else(|| self.unused_mmio_half(aligned))
                    >> ((address & 1) * 8)) as u8
            }
            5 => self.palette[(address as usize) & (PALETTE_SIZE - 1)],
            6 => vram_index(address, self.dispcnt).map_or(0, |index| self.vram[index]),
            7 => self.oam[(address as usize) & (OAM_SIZE - 1)],
            8..=13 => self.read_game_pak_rom(address, AccessWidth::Byte) as u8,
            14..=15 => {
                let offset = address & 0x00ff_ffff;
                if let Some(tilt) = &self.cartridge_tilt
                    && tilt::Tilt::accepts_read(offset)
                {
                    tilt.read(offset)
                } else {
                    self.save.peek_byte(address as u16, self.current_cycle)
                }
            }
            _ => (self.open_bus_value >> ((address & 3) * 8)) as u8,
        }
    }

    /// A debugger's zero-time byte write. Only work RAM is writable: it has no
    /// raster history or device side effects, and the write clears decoded
    /// code it covers as a CPU store would. Returns false elsewhere.
    pub fn poke_byte(&mut self, address: u32, value: u8) -> bool {
        match address >> 24 {
            2 => self.write_ewram(address, AccessWidth::Byte, value.into()),
            3 => self.write_iwram(address, AccessWidth::Byte, value.into()),
            _ => return false,
        }
        true
    }

    pub fn now(&self) -> Cycle {
        self.current_cycle
    }

    pub fn save_data(&self) -> &[u8] {
        self.save.data()
    }

    pub fn save_type(&self) -> save::Type {
        self.save.resolved_type()
    }

    pub fn enable_rtc(&mut self, epoch_seconds: u64) {
        if let Some(gpio) = &mut self.cartridge_gpio {
            gpio.attach_rtc(epoch_seconds);
        } else {
            self.cartridge_gpio = Some(CartridgeGpio::init_rtc(epoch_seconds));
        }
    }

    pub fn enable_solar_sensor(&mut self, light_level: u8) {
        self.cartridge_gpio
            .get_or_insert_with(CartridgeGpio::init)
            .attach_solar_sensor(light_level);
    }

    pub fn set_solar_level(&mut self, light_level: u8) {
        if let Some(gpio) = &mut self.cartridge_gpio {
            gpio.set_solar_level(light_level);
        }
    }

    pub fn enable_gyro(&mut self, sample: u16) {
        self.cartridge_gpio.get_or_insert_with(CartridgeGpio::init).attach_gyro(sample);
    }

    pub fn set_gyro_sample(&mut self, sample: u16) {
        if let Some(gpio) = &mut self.cartridge_gpio {
            gpio.set_gyro_sample(sample);
        }
    }

    pub fn enable_rumble(&mut self) {
        self.cartridge_gpio.get_or_insert_with(CartridgeGpio::init).attach_rumble();
    }

    pub fn rumble_active(&self) -> bool {
        self.cartridge_gpio.as_ref().is_some_and(CartridgeGpio::is_rumbling)
    }

    pub fn enable_tilt(&mut self, sample: tilt::Sample) {
        self.cartridge_tilt = Some(tilt::Tilt::init(sample));
    }

    pub fn set_tilt(&mut self, sample: tilt::Sample) {
        if let Some(tilt) = &mut self.cartridge_tilt {
            tilt.set_input(sample);
        }
    }

    pub fn set_keys(&mut self, keys: KeyState) {
        self.keys = keys;
        self.update_keypad_condition();
    }

    /// Installs the inputs of one synchronous run, which lasts until
    /// `end_external_inputs`. The bus keeps a copy, so it borrows nothing.
    pub fn begin_external_inputs(&mut self, inputs: &[TimedInput]) {
        debug_assert!(self.external_inputs.is_empty());
        if inputs.is_empty() {
            return;
        }
        self.external_inputs.extend_from_slice(inputs);
        self.external_input_index = 0;
        self.apply_due_external_inputs();
        if self.batch_active {
            self.flush_device_batch();
            self.batch_deadline = self.next_device_boundary();
        }
    }

    pub fn external_inputs_consumed(&self) -> usize {
        self.external_input_index
    }

    pub fn end_external_inputs(&mut self) {
        self.external_inputs.clear();
        self.external_input_index = 0;
        if self.batch_active {
            self.flush_device_batch();
            self.batch_deadline = self.next_device_boundary();
        }
    }

    fn next_external_input_cycle(&self) -> Option<Cycle> {
        self.external_inputs.get(self.external_input_index).map(|input| input.at)
    }

    #[inline(never)]
    fn apply_due_external_inputs(&mut self) {
        while let Some(&input) = self.external_inputs.get(self.external_input_index) {
            if input.at > self.current_cycle {
                break;
            }
            match input.change {
                TimedInputChange::Keys(keys) => self.set_keys(keys),
                TimedInputChange::SolarLevel(level) => self.set_solar_level(level),
                TimedInputChange::GyroSample(sample) => self.set_gyro_sample(sample),
                TimedInputChange::Tilt(sample) => self.set_tilt(sample),
            }
            self.external_input_index += 1;
        }
    }

    fn pressed_keys(&self) -> u16 {
        [
            self.keys.a,
            self.keys.b,
            self.keys.select,
            self.keys.start,
            self.keys.right,
            self.keys.left,
            self.keys.up,
            self.keys.down,
            self.keys.r,
            self.keys.l,
        ]
        .into_iter()
        .enumerate()
        .fold(0, |bits, (index, pressed)| bits | (u16::from(pressed) << index))
    }

    fn read_key_input(&self) -> u16 {
        !self.pressed_keys() & 0x03ff
    }

    fn update_keypad_condition(&mut self) {
        let selected = self.keypad_control & 0x03ff;
        let matching = self.pressed_keys() & selected;
        let condition = if self.keypad_control & (1 << 15) != 0 {
            selected != 0 && matching == selected
        } else {
            matching != 0
        };
        if condition && !self.keypad_condition_met && self.keypad_control & (1 << 14) != 0 {
            self.irq.interrupt_request_flags |= 1 << 12;
            self.update_irq_line();
        }
        self.keypad_condition_met = condition;
    }

    pub fn irq_pending(&self) -> bool {
        self.irq.request()
    }

    /// Follows IE, IF and IME after a change at the current cycle. A request
    /// that rises reaches the CPU at a device boundary: an open batch stops
    /// there, even when the change came from outside a register write.
    /// Out of line: it runs only when those registers change.
    #[inline(never)]
    pub fn update_irq_line(&mut self) {
        self.irq.update_line(self.current_cycle);
        if let Some(at) = self.irq.irq_line_rises_at
            && self.batch_active
            && at < self.batch_deadline
        {
            self.batch_deadline = at;
        }
    }

    pub fn power_state(&self) -> PowerState {
        self.irq.power_state
    }

    pub fn resume_power(&mut self) {
        self.irq.power_state = PowerState::Running;
    }

    /// Whether a halted CPU wakes: an enabled flag is raised, and any
    /// interrupt request it drives has reached the CPU, so that the CPU
    /// wakes into the IRQ rather than running on before it.
    pub fn halt_wake_pending(&self) -> bool {
        self.irq.interrupt_enable_mask & self.irq.interrupt_request_flags != 0
            && self.irq.irq_line_rises_at.is_none()
    }

    pub fn stop_wake_pending(&self) -> bool {
        self.irq.interrupt_enable_mask
            & self.irq.interrupt_request_flags
            & ((1 << 7) | (1 << 12) | (1 << 13))
            != 0
    }

    pub fn completed_frames(&self) -> Cycle {
        self.completed_frame_count
    }

    /// Dots the video has advanced since power-on: completed frames, lines
    /// and the dot within the line, including cycles a device batch has not
    /// yet delivered. It moves with the master clock except in STOP, where
    /// the video is frozen.
    pub(crate) fn video_progress(&self) -> Cycle {
        let pending =
            if self.batch_active { self.current_cycle - self.batch_device_cycle } else { 0 };
        self.completed_frame_count * 228 * 1232
            + Cycle::from(self.ppu_timing.line) * 1232
            + Cycle::from(self.ppu_timing.cycle_in_line)
            + pending
    }

    /// A cheat device's store: a CPU store's effects on RAM, I/O and video
    /// memory, without its cycles. Writes elsewhere (BIOS, Game Pak, save
    /// media) are ignored. An I/O write that starts a DMA runs it as a CPU
    /// store would.
    pub(crate) fn cheat_write(&mut self, address: u32, width: AccessWidth, value: u32) {
        let address = address & !(width.bytes() as u32 - 1);
        // Video memory takes a word as two halfword beats, as a CPU store does.
        let (beats, beat_width) =
            if width == AccessWidth::Word { (2, AccessWidth::Half) } else { (1, width) };
        match address >> 24 {
            2 => self.write_ewram(address, width, value),
            3 => self.write_iwram(address, width, value),
            4 => {
                let batch = self.batch_active;
                if batch {
                    self.flush_device_batch();
                    self.batch_active = false;
                    self.batch_deadline = 0;
                }
                self.write_mmio(address, width, value);
                if batch {
                    self.batch_active = true;
                    self.batch_device_cycle = self.current_cycle;
                    self.batch_deadline = self.next_device_boundary();
                }
                self.service_pending_dma();
            }
            5 => {
                self.flush_device_batch();
                for beat in 0..beats {
                    let at = address + beat * 2;
                    self.begin_palette_segments(at, beat_width);
                    self.write_palette(at, beat_width, value >> (beat * 16));
                    self.end_palette_segments(at, beat_width);
                }
            }
            6 => {
                self.flush_device_batch();
                for beat in 0..beats {
                    self.write_vram_at_current_dot(
                        address + beat * 2,
                        beat_width,
                        value >> (beat * 16),
                    );
                }
            }
            7 => self.write_oam(address, width, value),
            _ => {}
        }
    }

    pub fn framebuffer(&self) -> &[u16] {
        &self.frame_pixels[..]
    }

    pub fn set_video_interval(&mut self, interval: u16) {
        debug_assert!((1..=16).contains(&interval));
        self.synchronize_devices();
        if interval == self.video_requested_interval {
            return;
        }
        if interval != 1 && !self.video_publication_active {
            self.video_published_pixels.copy_from_slice(&self.frame_pixels[..]);
            self.video_publication_active = true;
        }
        self.video_requested_interval = interval;
    }

    pub fn current_audio_sample(&self) -> StereoSample {
        self.apu.current_sample()
    }

    pub fn drain_audio_samples(&mut self, output: &mut [StereoSample]) -> usize {
        self.sync_apu_to(self.current_cycle);
        self.apu.drain_samples(output)
    }

    pub fn set_audio_output(&mut self, enabled: bool) {
        self.synchronize_devices();
        self.apu.set_sample_output(enabled);
    }

    pub fn set_high_speed_audio(&mut self, enabled: bool) {
        self.synchronize_devices();
        self.apu.set_high_speed_audio(enabled);
    }

    pub fn audio_sample_rate(&self) -> u32 {
        self.apu.sample_rate()
    }

    pub fn published_framebuffer(&self) -> &[u16] {
        if self.video_publication_active {
            &self.video_published_pixels[..]
        } else {
            &self.frame_pixels[..]
        }
    }

    pub fn is_framebuffer_canonical(&self) -> bool {
        self.video_framebuffer_canonical
    }

    fn reset_video_policy(&mut self, interval: u16) {
        self.video_active_interval = interval;
        self.video_requested_interval = interval;
        self.video_skip_remaining = interval - 1;
        self.video_render_current = true;
        self.video_framebuffer_canonical = true;
        self.video_publication_active = interval != 1;
        self.video_frame_id = 0;
        self.video_frame_cycle = 0;
        if self.video_publication_active {
            self.video_published_pixels.copy_from_slice(&self.frame_pixels[..]);
        }
    }

    pub fn use_full_video(&mut self) {
        debug_assert!(self.video_framebuffer_canonical);
        self.video_active_interval = 1;
        self.video_requested_interval = 1;
        self.video_skip_remaining = 0;
        self.video_render_current = true;
        self.video_publication_active = false;
    }

    pub fn accept_canonical_framebuffer(&mut self, pixels: &[u16]) {
        assert_eq!(pixels.len(), self.frame_pixels.len());
        if pixels.as_ptr() != self.frame_pixels.as_ptr() {
            self.frame_pixels.copy_from_slice(pixels);
        }
        self.scanline_memo.invalidate();
        self.video_framebuffer_canonical = true;
    }

    pub fn next_rendered_frame_cycle(&self) -> Cycle {
        let frame_cycles: Cycle = 228 * 1232;
        let visible_end: Cycle = (SCREEN_HEIGHT as Cycle - 1) * 1232 + 960;
        let phase =
            Cycle::from(self.ppu_timing.line) * 1232 + Cycle::from(self.ppu_timing.cycle_in_line);
        if self.video_render_current && phase < visible_end {
            return self.current_cycle.saturating_add(visible_end - phase);
        }
        let skipped = if self.video_requested_interval != self.video_active_interval {
            0
        } else {
            Cycle::from(self.video_skip_remaining)
        };
        self.current_cycle
            .saturating_add((frame_cycles - phase) + skipped * frame_cycles + visible_end)
    }

    pub fn set_bios_latch(&mut self, value: u32) {
        self.bios_latch = value;
    }

    /// The HLE IRQ entry's accesses in one step: the BIOS opcode fetches
    /// at 18h, 1Ch, 20h, 128h, 12Ch and 130h, the six word stores of
    /// `saved` from `stack` up, the fetches at 134h, 138h and 13Ch, the load
    /// of the handler address from 3007FFCh and its internal cycle. Returns
    /// the handler address, or `None` without effect where the result could
    /// differ from making the accesses one by one (`Cpu::charge_hle_irq_entry`,
    /// the reference): outside the device batch, with DMA pending, when a
    /// device event falls inside the sequence, or with the stack outside
    /// IWRAM and EWRAM.
    pub(crate) fn fused_hle_irq_entry(&mut self, stack: u32, saved: [u32; 6]) -> Option<u32> {
        let word = hle_stack_word_cycles(stack)?;
        if !self.fuses_hle_trampoline(6 + 6 * word + 3 + 1 + 1) {
            return None;
        }
        self.begin_hle_bios_fetches();
        for (index, value) in saved.into_iter().enumerate() {
            let address = stack.wrapping_add(4 * index as u32);
            if word == 1 {
                self.write_iwram(address, AccessWidth::Word, value);
            } else {
                self.write_ewram(address, AccessWidth::Word, value);
            }
        }
        let handler = self.read_iwram(0x0300_7ffc, AccessWidth::Word);
        self.open_bus_value = handler;
        self.fetch_kind = AccessKind::Nonsequential;
        self.current_cycle += 6 + 6 * word + 3 + 1 + 1;
        Some(handler)
    }

    /// The HLE IRQ return's accesses in one step, from a full pipeline: the
    /// BIOS opcode fetch at 140h, the six word loads from `stack` up, one
    /// internal cycle and the fetch at 144h. Returns whether it made them;
    /// it declines as [`Self::fused_hle_irq_entry`] does, and
    /// `Cpu::complete_hle_irq` then makes them one by one.
    pub(crate) fn fused_hle_irq_return(&mut self, stack: u32) -> bool {
        let Some(word) = hle_stack_word_cycles(stack) else {
            return false;
        };
        if !self.fuses_hle_trampoline(1 + 6 * word + 1 + 1) {
            return false;
        }
        // The loads leave only their time. The last access is the fetch,
        // which leaves the latch on the data bus without an image.
        self.begin_hle_bios_fetches();
        self.open_bus_value = self.bios_latch;
        self.fetch_kind = AccessKind::Sequential;
        self.current_cycle += 1 + 6 * word + 1 + 1;
        true
    }

    /// Whether `cycles` of CPU accesses to the BIOS, IWRAM and EWRAM starting
    /// now meet no device event and no DMA: each access then only moves the
    /// clock, as [`Self::advance`] does inside the batch. HLE runs without a
    /// BIOS image, whose fetches would latch its words instead.
    fn fuses_hle_trampoline(&self, cycles: Cycle) -> bool {
        #[cfg(test)]
        if !tests::FUSED_HLE.get() {
            return false;
        }
        let fused = self.bios.len() != BIOS_SIZE
            && self.batch_active
            && self.pending_dma == 0
            && self.current_cycle + cycles < self.batch_deadline;
        #[cfg(test)]
        {
            let counter = if fused { &tests::FUSED_HLE_TAKEN } else { &tests::FUSED_HLE_DECLINED };
            counter.set(counter.get() + 1);
        }
        fused
    }

    /// What the first BIOS opcode fetch of a fused sequence leaves: the
    /// Game Pak is no longer the code source, and its prefetch buffer is
    /// discarded before any of the sequence's cycles pass, so they don't
    /// advance it.
    fn begin_hle_bios_fetches(&mut self) {
        self.bios_code_active = true;
        self.game_pak_code_active = false;
        self.game_pak_prefetch = GamePakPrefetch::default();
    }

    /// Records the bus where an HLE IRQ entry or return ends its accesses,
    /// fused or not, for the test that compares the two.
    #[cfg(test)]
    pub(crate) fn trace_hle_trampoline(&self) {
        let trace = format!(
            "{} {:08x} {:?} {:08x} {} {} {:?} {} {} {} {}",
            self.current_cycle,
            self.open_bus_value,
            self.fetch_kind,
            self.bios_latch,
            self.bios_code_active,
            self.game_pak_code_active,
            self.game_pak_prefetch,
            self.batch_active,
            self.batch_device_cycle,
            self.batch_deadline,
            self.pending_dma,
        );
        tests::HLE_TRACE.with_borrow_mut(|traces| traces.push(trace));
    }

    pub fn prescaler_counter(&self) -> u16 {
        (self.timer_cycle.wrapping_sub(u64::from(self.prescaler_origin)) as u16) & 1023
    }

    /// A bus read of the BIOS with its read protection (GBATEK,
    /// Unpredictable Things: Reading from BIOS Memory). The image answers an
    /// opcode fetch, whose aligned word it latches, and a data read while the
    /// most recent opcode fetch was from the BIOS; any other read returns the
    /// latch. Without an image every read returns the HLE BIOS's latch.
    fn read_protected_bios(
        &mut self,
        address: u32,
        width: AccessWidth,
        source: AccessSource,
    ) -> u32 {
        if self.bios.len() != BIOS_SIZE
            || (source != AccessSource::CpuCode && !self.bios_code_active)
        {
            return self.bios_latch_lane(address, width);
        }
        if source == AccessSource::CpuCode {
            self.bios_latch = self.read_bios(address & !3, AccessWidth::Word);
        }
        self.read_bios(address, width)
    }

    fn bios_latch_lane(&self, address: u32, width: AccessWidth) -> u32 {
        let shift = match width {
            AccessWidth::Byte => (address & 3) * 8,
            AccessWidth::Half => (address & 2) * 8,
            AccessWidth::Word => 0,
        };
        let mask = match width {
            AccessWidth::Byte => 0xff,
            AccessWidth::Half => 0xffff,
            AccessWidth::Word => u32::MAX,
        };
        (self.bios_latch >> shift) & mask
    }

    /// The image's bytes, unprotected, as the debugger sees them; the latch
    /// without an image.
    fn read_bios(&self, address: u32, width: AccessWidth) -> u32 {
        if self.bios.len() != BIOS_SIZE {
            return self.bios_latch_lane(address, width);
        }
        (0..width.bytes())
            .map(|offset| u32::from(self.bios[address as usize + offset]) << (offset * 8))
            .fold(0, |a, b| a | b)
    }

    fn read_game_pak_rom(&self, address: u32, width: AccessWidth) -> u32 {
        let rom = self.rom.as_slice();
        let first = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        let end = first + width.bytes();
        if end <= rom.len()
            && end <= GAME_PAK_WINDOW_SIZE as usize
            && (self.cartridge_gpio.is_none() || first > 0xc9 || end <= 0xc4)
        {
            return (0..width.bytes()).fold(0u32, |value, offset| {
                value | (u32::from(rom[first + offset]) << (offset * 8))
            });
        }
        (0..width.bytes()).fold(0u32, |value, offset| {
            let byte_address = address.wrapping_add(offset as u32);
            let index = ((byte_address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
            let port = self.cartridge_gpio.as_ref().and_then(|gpio| gpio.read_byte(index as u32));
            let byte = port.unwrap_or_else(|| {
                rom.get(index).copied().unwrap_or_else(|| {
                    let halfword = (byte_address >> 1) as u16;
                    if byte_address & 1 == 0 { halfword as u8 } else { (halfword >> 8) as u8 }
                })
            });
            value | (u32::from(byte) << (offset * 8))
        })
    }

    fn read_ewram(&self, address: u32, width: AccessWidth) -> u32 {
        read_ram(&self.ewram, (address.wrapping_sub(EWRAM_BASE) as usize) & (EWRAM_SIZE - 1), width)
    }

    #[inline(always)]
    fn write_ewram(&mut self, address: u32, width: AccessWidth, value: u32) {
        let index = (address.wrapping_sub(EWRAM_BASE) as usize) & (EWRAM_SIZE - 1);
        self.code.write_ewram(index, width.bytes());
        write_ram(&mut self.ewram, index, width, value);
    }

    fn read_iwram(&self, address: u32, width: AccessWidth) -> u32 {
        read_ram(&self.iwram, (address.wrapping_sub(IWRAM_BASE) as usize) & (IWRAM_SIZE - 1), width)
    }

    #[inline(always)]
    fn write_iwram(&mut self, address: u32, width: AccessWidth, value: u32) {
        let index = (address.wrapping_sub(IWRAM_BASE) as usize) & (IWRAM_SIZE - 1);
        self.code.write_iwram(index, width.bytes());
        write_ram(&mut self.iwram, index, width, value);
    }

    fn write_waitcnt(&mut self, value: u16) {
        let old_prefetch = self.waitcnt & (1 << 14) != 0;
        let new_waitcnt = value & 0x7fff;
        let new_prefetch = new_waitcnt & (1 << 14) != 0;
        if old_prefetch && !new_prefetch {
            self.prefetch_buffer_was_disabled = true;
        }
        if new_prefetch {
            self.prefetch_disabled_bug_pending = false;
        }
        self.waitcnt = new_waitcnt;
        self.duty_valid_until = 0;
    }

    fn sram_cycles(&self) -> Cycle {
        [5, 4, 3, 9][usize::from(self.waitcnt & 3)]
    }

    fn refresh_game_pak_timing(&mut self) {
        (self.game_pak_n_cycles, self.game_pak_s_cycles) = game_pak_timing(self.waitcnt);
        self.game_pak_timing_key = self.waitcnt;
    }

    fn game_pak_half_cycles(&self, address: u32, requested_kind: AccessKind) -> Cycle {
        let window = ((address - GAME_PAK_BASE) / GAME_PAK_WINDOW_SIZE) as usize;
        if address & 0x1ffff == 0 || requested_kind == AccessKind::Nonsequential {
            Cycle::from(self.game_pak_n_cycles[window])
        } else {
            Cycle::from(self.game_pak_s_cycles[window])
        }
    }

    fn game_pak_cycles(&mut self, access: Access) -> Cycle {
        if self.game_pak_timing_key != self.waitcnt {
            self.refresh_game_pak_timing();
        }
        let aligned = access.address
            & !match access.width {
                AccessWidth::Byte | AccessWidth::Half => 1,
                AccessWidth::Word => 3,
            };
        let first = self.game_pak_half_cycles(aligned, access.kind);
        if access.width == AccessWidth::Word {
            first + self.game_pak_half_cycles(aligned.wrapping_add(2), AccessKind::Sequential)
        } else {
            first
        }
    }

    fn is_eeprom_address(&self, address: u32) -> bool {
        (0x0d00_0000..0x0e00_0000).contains(&address)
            && (self.rom.len() <= 16 * 1024 * 1024 || address >= 0x0dff_ff00)
    }

    fn is_eeprom_access(&self, address: u32) -> bool {
        self.save.is_eeprom()
            && self.active_dma_channel == Some(3)
            && self.is_eeprom_address(address)
    }

    fn is_cartridge_gpio_write(&self, access: Access) -> bool {
        if self.cartridge_gpio.is_none() {
            return false;
        }
        let offset = (access.address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1);
        CartridgeGpio::is_register(offset)
            || (access.width == AccessWidth::Word
                && CartridgeGpio::is_register(offset.wrapping_add(2)))
    }

    #[inline(always)]
    fn write_cartridge_gpio_half(&mut self, address: u32, value: u16) {
        let offset = (address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1);
        if self
            .cartridge_gpio
            .as_mut()
            .is_some_and(|gpio| gpio.write_half(offset, value, self.current_cycle))
        {
            self.irq.interrupt_request_flags |= 1 << 13;
            self.update_irq_line();
        }
    }

    fn read_mmio(&mut self, address: u32, width: AccessWidth) -> u32 {
        match width {
            AccessWidth::Half => u32::from(
                self.try_read_mmio_half(address).unwrap_or_else(|| self.unused_mmio_half(address)),
            ),
            AccessWidth::Word => {
                assert_eq!(address & 3, 0, "unaligned MMIO word read");
                let low = self.try_read_mmio_half(address);
                let high = self.try_read_mmio_half(address + 2);
                if low.is_none() && high.is_none() {
                    self.open_bus_value
                } else {
                    u32::from(low.unwrap_or(0)) | (u32::from(high.unwrap_or(0)) << 16)
                }
            }
            AccessWidth::Byte => {
                let aligned = address & !1;
                if aligned == MMIO_BASE + 0x12a && self.is_uart_mode() && address & 1 != 0 {
                    return 0;
                }
                let value = self
                    .try_read_mmio_half(aligned)
                    .unwrap_or_else(|| self.unused_mmio_half(aligned));
                u32::from((value >> ((address & 1) * 8)) & 0xff)
            }
        }
    }

    fn unused_mmio_half(&self, address: u32) -> u16 {
        if self.peek_mmio_half(address ^ 2).is_some() {
            0
        } else {
            (self.open_bus_value >> ((address & 2) * 8)) as u16
        }
    }

    fn try_read_mmio_half(&mut self, address: u32) -> Option<u16> {
        if (MMIO_BASE + 0x60..MMIO_BASE + 0xa0).contains(&address) {
            self.sync_apu_to(self.current_cycle);
        }
        if (MMIO_BASE + 0x100..=MMIO_BASE + 0x10e).contains(&address) {
            self.sync_timers_to(self.current_cycle);
        }
        if address == MMIO_BASE + 0x128 && self.is_uart_mode() {
            let value = self.read_sio_control();
            self.uart_error = false;
            return Some(value);
        }
        if address == MMIO_BASE + 0x12a && self.is_uart_mode() {
            return Some(self.read_uart_data());
        }
        if address == MMIO_BASE + 0x150 || address == MMIO_BASE + 0x152 {
            let value = self.peek_mmio_half(address);
            self.joy_status &= !(1 << 1);
            return value;
        }
        self.peek_mmio_half(address)
    }

    /// The value a halfword read of a readable I/O register returns, or
    /// `None` for an unused or write-only one, which reads as
    /// [`Self::unused_mmio_half`] describes.
    fn peek_mmio_half(&self, address: u32) -> Option<u16> {
        if (MMIO_BASE + 0xb0..MMIO_BASE + 0xe0).contains(&address) && address & 1 == 0 {
            let offset = address - (MMIO_BASE + 0xb0);
            let index = (offset / 12) as usize;
            // Only the control halfword reads back.
            return (offset % 12 == 10).then(|| self.dma_channels[index].read_control(index));
        }
        if (MMIO_BASE + 0x100..=MMIO_BASE + 0x10e).contains(&address) && address & 1 == 0 {
            let offset = address - (MMIO_BASE + 0x100);
            let timer = &self.timers[(offset / 4) as usize];
            return Some(if offset & 2 != 0 { timer.read_control() } else { timer.read_counter() });
        }
        let offset = address.wrapping_sub(MMIO_BASE);
        if let Some(index) = Self::background_control_index(address) {
            // Bit 13, the affine overflow flag, exists only for BG2 and BG3.
            let mask = if index < 2 { 0xdfff } else { 0xffff };
            return Some(self.bg_controls[index] & mask);
        }
        if Self::background_offset_register(address).is_some()
            || Self::affine_parameter_register(address).is_some()
            || Self::affine_reference_register(address).is_some()
        {
            return None;
        }
        if let Some(value) = self.apu.read_half(address) {
            return Some(value);
        }
        if (0x120..=0x126).contains(&offset) && offset & 1 == 0 {
            return Some(self.read_sio_data(((offset - 0x120) / 2) as usize));
        }
        Some(match offset {
            0x000 => self.dispcnt,
            0x002 => u16::from(self.green_swap_enabled),
            0x004 => self.ppu_timing.read_dispstat(),
            0x006 => self.ppu_timing.read_vcount(),
            0x048 => self.winin,
            0x04a => self.winout,
            0x050 => self.bldcnt,
            0x052 => self.bldalpha,
            0x128 => self.read_sio_control(),
            0x12a => {
                if self.is_uart_mode() {
                    self.peek_uart_data()
                } else {
                    self.sio_send
                }
            }
            0x130 => self.read_key_input(),
            0x132 => self.keypad_control,
            0x134 => self.read_rcnt(),
            0x136 | 0x142 | 0x15a => 0,
            0x140 => self.joy_control & 0x47,
            0x150 => self.joy_receive as u16,
            0x152 => (self.joy_receive >> 16) as u16,
            0x154 | 0x156 => 0,
            0x158 => self.joy_status & 0x3a,
            0x200 => self.irq.interrupt_enable_mask,
            0x202 => self.irq.interrupt_request_flags,
            0x204 => self.waitcnt,
            0x208 => u16::from(self.irq.interrupt_master_enabled),
            0x300 => u16::from(self.irq.post_boot),
            _ => return None,
        })
    }

    fn write_mmio(&mut self, address: u32, width: AccessWidth, value: u32) {
        if Apu::accepts_fifo_write(address, width.bytes() as u8) {
            self.write_fifo(address, value, width.bytes() as u8);
            return;
        }
        match width {
            AccessWidth::Half => self.write_mmio_half(address, value as u16),
            AccessWidth::Word => {
                assert_eq!(address & 3, 0, "unaligned MMIO word write");
                self.write_mmio_half(address, value as u16);
                self.write_mmio_half(address + 2, (value >> 16) as u16);
            }
            AccessWidth::Byte => self.write_mmio_byte(address, value as u8),
        }
    }

    fn write_mmio_byte(&mut self, address: u32, value: u8) {
        if address == MMIO_BASE + 0x300 {
            self.irq.post_boot = value & 1;
            return;
        }
        if address == MMIO_BASE + 0x301 {
            self.irq.power_state =
                if value & 0x80 != 0 { PowerState::Stopped } else { PowerState::Halted };
            return;
        }
        if (MMIO_BASE + 0x60..MMIO_BASE + 0xa0).contains(&address) {
            self.sync_apu_to(self.current_cycle);
        }
        if self.apu.write_byte(address, value) {
            self.refresh_after_apu_write(address);
            return;
        }
        let aligned = address & !1;
        if aligned == MMIO_BASE + 0x006 || aligned == MMIO_BASE + 0x130 {
            return;
        }
        if aligned == MMIO_BASE + 0x12a && self.is_uart_mode() {
            if address & 1 == 0 {
                self.write_uart_data(value);
            }
            return;
        }
        if aligned == MMIO_BASE + 0x202 {
            self.irq.interrupt_request_flags &= !(u16::from(value) << ((address & 1) * 8));
            self.update_irq_line();
            return;
        }
        let Some(old) = self.mmio_byte_write_value(aligned) else {
            return;
        };
        let shift = (address & 1) * 8;
        let mask = 0xff_u16 << shift;
        self.write_mmio_half(aligned, (old & !mask) | (u16::from(value) << shift));
    }

    fn mmio_byte_write_value(&self, address: u32) -> Option<u16> {
        let offset = address.wrapping_sub(MMIO_BASE);
        if (0xb0..0xe0).contains(&offset) {
            let channel = &self.dma_channels[((offset - 0xb0) / 12) as usize];
            return Some(match (offset - 0xb0) % 12 {
                0 => channel.source as u16,
                2 => (channel.source >> 16) as u16,
                4 => channel.destination as u16,
                6 => (channel.destination >> 16) as u16,
                8 => channel.word_count,
                10 => channel.control,
                _ => return None,
            });
        }
        if (0x100..=0x10e).contains(&offset) {
            let timer = &self.timers[((offset - 0x100) / 4) as usize];
            return Some(if offset & 2 == 0 { timer.reload } else { timer.control });
        }
        if let Some((bg, vertical)) = Self::background_offset_register(address) {
            return Some(if vertical {
                self.bg_vertical_offsets[bg]
            } else {
                self.bg_horizontal_offsets[bg]
            });
        }
        if let Some((bg, coord, high)) = Self::affine_reference_register(address) {
            let raw = self.bg_reference_points[bg][coord] as u32;
            return Some(if high { (raw >> 16) as u16 } else { raw as u16 });
        }
        if let Some((bg, parameter)) = Self::affine_parameter_register(address) {
            return Some(self.bg_affine_parameters[bg][parameter] as u16);
        }
        match offset {
            0x040 => return Some(self.win0h),
            0x042 => return Some(self.win1h),
            0x044 => return Some(self.win0v),
            0x046 => return Some(self.win1v),
            0x04c => return Some(self.mosaic),
            0x054 => return Some(self.bldy),
            _ => {}
        }
        if offset == 0x154 {
            return Some(self.joy_transmit as u16);
        }
        if offset == 0x156 {
            return Some((self.joy_transmit >> 16) as u16);
        }
        self.peek_mmio_half(address)
    }

    pub(crate) fn write_mmio_half(&mut self, address: u32, value: u16) {
        let offset = address.wrapping_sub(MMIO_BASE);
        if offset < 0x60 {
            self.scanline_memo.mark_registers();
        }
        if (0xb0..0xe0).contains(&offset) && offset & 1 == 0 {
            let index = ((offset - 0xb0) / 12) as usize;
            let channel = &mut self.dma_channels[index];
            let mut immediate = false;
            match (offset - 0xb0) % 12 {
                0 => channel.write_source_low(value),
                2 => channel.write_source_high(index, value),
                4 => channel.write_destination_low(value),
                6 => channel.write_destination_high(index, value),
                8 => channel.word_count = value,
                10 => {
                    let newly_enabled = channel.write_control(value);
                    if !channel.enabled() {
                        self.dma_start_cycles[index] = None;
                    }
                    if newly_enabled
                        && (1..=2).contains(&index)
                        && channel.start_timing() == 3
                        && Apu::fifo_for_address(channel.destination).is_some()
                    {
                        channel.control =
                            (channel.control & !((3 << 5) | (1 << 10))) | (2 << 5) | (1 << 10);
                    }
                    if newly_enabled {
                        channel.align_latched_addresses();
                    }
                    immediate = newly_enabled && channel.start_timing() == 0;
                }
                _ => unreachable!(),
            }
            if immediate && self.dma_start_cycles[index].is_none() {
                self.dma_start_cycles[index] = Some(self.current_cycle.saturating_add(2));
            }
            return;
        }
        if (0x100..=0x10e).contains(&offset) && offset & 1 == 0 {
            self.sync_timers_to(self.current_cycle);
            self.sync_apu_to(self.current_cycle);
            let index = ((offset - 0x100) / 4) as usize;
            if offset & 2 != 0 {
                self.timers[index].write_control(value, self.prescaler_counter());
                // A cascading timer counts overflows, not cycles: it has no
                // start delay. Timer 0 ignores the count-up bit.
                if index != 0 && self.timers[index].count_up_enabled() {
                    self.timers[index].starting = false;
                }
            } else {
                self.timers[index].write_reload(value);
            }
            self.refresh_fifo_clock();
            self.refresh_timer_deadline();
            return;
        }
        if let Some(index) = Self::background_control_index(address) {
            let priority_changed = (self.bg_controls[index] ^ value) & 3 != 0;
            if priority_changed {
                self.begin_scroll_segment(index);
            }
            self.bg_controls[index] = value;
            if priority_changed {
                self.end_scroll_segment(index);
            }
            return;
        }
        if let Some((index, vertical)) = Self::background_offset_register(address) {
            self.begin_scroll_segment(index);
            if vertical {
                self.bg_vertical_offsets[index] = value & 0x1ff;
            } else {
                self.bg_horizontal_offsets[index] = value & 0x1ff;
            }
            self.end_scroll_segment(index);
            return;
        }
        if let Some((background, parameter)) = Self::affine_parameter_register(address) {
            self.begin_affine_segment();
            self.bg_affine_parameters[background][parameter] = value as i16;
            self.end_affine_segment(background, AffineTransition::PreserveAccumulator);
            return;
        }
        if let Some((background, coordinate, high)) = Self::affine_reference_register(address) {
            self.begin_affine_segment();
            let old = self.bg_reference_points[background][coordinate] as u32;
            let raw = if high {
                (old & 0xffff) | (u32::from(value & 0x0fff) << 16)
            } else {
                (old & 0x0fff_0000) | u32::from(value)
            };
            let point = ((raw << 4) as i32) >> 4;
            self.bg_reference_points[background][coordinate] = point;
            self.bg_affine_current[background][coordinate] = point;
            self.end_affine_segment(background, AffineTransition::ReloadReferenceOrigin);
            return;
        }
        if (0x60..0xa0).contains(&offset) {
            self.sync_apu_to(self.current_cycle);
        }
        if self.apu.write_half(address, value) {
            self.refresh_after_apu_write(address);
            return;
        }
        if (0x120..=0x126).contains(&offset) && offset & 1 == 0 {
            if (self.sio_control >> 12) & 3 == 1 && offset < 0x124 {
                self.sio_data[((offset - 0x120) / 2) as usize] = value;
            }
            return;
        }
        match offset {
            0x000 => {
                let changed = (self.dispcnt ^ value) & ((1 << 13) | (1 << 14)) != 0
                    || (self.dispcnt & 0x80 == 0 && value & 0x80 != 0);
                if changed {
                    self.begin_window_segment();
                }
                self.dispcnt = value;
                if changed {
                    self.end_window_segment();
                }
            }
            0x002 => self.green_swap_enabled = value & 1 != 0,
            0x004 => self.ppu_timing.write_dispstat(value),
            0x132 => {
                self.keypad_control = value & 0xc3ff;
                self.update_keypad_condition();
            }
            0x200 => {
                self.irq.interrupt_enable_mask = value & 0x3fff;
                self.update_irq_line();
            }
            0x202 => {
                self.irq.interrupt_request_flags &= !value;
                self.update_irq_line();
            }
            0x204 => self.write_waitcnt(value),
            0x208 => {
                self.irq.interrupt_master_enabled = value & 1 != 0;
                self.update_irq_line();
            }
            0x300 => {
                self.irq.post_boot = (value & 1) as u8;
                self.irq.power_state =
                    if value & 0x8000 != 0 { PowerState::Stopped } else { PowerState::Halted };
            }
            0x040 | 0x042 | 0x044 | 0x046 | 0x048 | 0x04a => {
                self.begin_window_segment();
                match offset {
                    0x040 => self.win0h = value,
                    0x042 => self.win1h = value,
                    0x044 => self.win0v = value,
                    0x046 => self.win1v = value,
                    0x048 => self.winin = value & 0x3f3f,
                    0x04a => self.winout = value & 0x3f3f,
                    _ => unreachable!(),
                }
                self.end_window_segment();
            }
            0x04c => {
                self.begin_mosaic_segment();
                self.mosaic = value;
                self.end_mosaic_segment();
            }
            0x050 | 0x052 | 0x054 => {
                self.begin_blend_segment();
                match offset {
                    0x050 => self.bldcnt = value & 0x3fff,
                    0x052 => self.bldalpha = value & 0x1f1f,
                    0x054 => self.bldy = value & 0x001f,
                    _ => unreachable!(),
                }
                self.end_blend_segment();
            }
            0x128 => self.write_sio_control(value),
            0x12a => {
                if self.is_uart_mode() {
                    self.write_uart_data(value as u8);
                } else if (self.sio_control >> 12) & 3 != 3 {
                    self.sio_send = value;
                }
            }
            0x134 => self.write_rcnt(value),
            0x136 | 0x142 | 0x150 | 0x152 | 0x15a => {}
            0x140 => {
                let pending = (self.joy_control & 7) & !(value & 7);
                self.joy_control = pending | (value & 0x40);
            }
            0x154 => {
                self.joy_transmit = (self.joy_transmit & 0xffff_0000) | u32::from(value);
                self.joy_status |= 1 << 3;
            }
            0x156 => {
                self.joy_transmit = (self.joy_transmit & 0x0000_ffff) | (u32::from(value) << 16);
                self.joy_status |= 1 << 3;
            }
            0x158 => self.joy_status = (self.joy_status & 0x000a) | (value & 0x0030),
            _ => {}
        }
    }

    fn read_sio_data(&self, index: usize) -> u16 {
        match (self.sio_control >> 12) & 3 {
            1 if index < 2 => self.sio_data[index],
            2 => self.sio_data[index],
            _ => 0,
        }
    }

    fn is_uart_mode(&self) -> bool {
        self.rcnt & 0x8000 == 0 && (self.sio_control >> 12) & 3 == 3
    }

    fn read_sio_control(&self) -> u16 {
        if (self.sio_control >> 12) & 3 == 3 {
            self.sio_control | self.uart_status()
        } else {
            self.sio_control
        }
    }

    fn uart_status(&self) -> u16 {
        let full = if self.sio_control & 0x100 != 0 {
            self.uart_tx_fifo.count == 4
        } else {
            self.uart_tx_active.is_some() || self.uart_tx_fifo.count != 0
        };
        (if full { 0x10 } else { 0 })
            | (if self.uart_rx_fifo.count == 0 { 0x20 } else { 0 })
            | (if self.uart_error { 0x40 } else { 0 })
    }

    fn peek_uart_data(&self) -> u16 {
        if self.uart_rx_fifo.count == 0 {
            0
        } else {
            u16::from(self.uart_rx_fifo.bytes[usize::from(self.uart_rx_fifo.read_index)])
        }
    }

    fn read_uart_data(&mut self) -> u16 {
        let old = self.uart_status();
        let value = self.uart_rx_fifo.pop().map_or(0, u16::from);
        self.raise_uart_status_edges(old);
        value
    }

    fn write_uart_data(&mut self, value: u8) {
        let old = self.uart_status();
        self.uart_tx_fifo.push(value, if self.sio_control & 0x100 != 0 { 4 } else { 1 });
        self.maybe_start_uart_transmit();
        self.raise_uart_status_edges(old);
    }

    fn raise_uart_status_edges(&mut self, old: u16) {
        if self.uart_status() & !old & 0x70 != 0 && self.sio_control & 0x4000 != 0 {
            self.irq.interrupt_request_flags |= 1 << 7;
            self.update_irq_line();
        }
    }

    fn maybe_start_uart_transmit(&mut self) {
        if !self.is_uart_mode()
            || self.sio_control & 0x400 == 0
            || self.uart_tx_active.is_some()
            || (self.sio_control & 4 != 0 && !self.uart_cts_ready)
        {
            return;
        }
        self.uart_tx_active = self.uart_tx_fifo.pop();
        if self.uart_tx_active.is_some() {
            self.uart_tx_peer_scheduled = false;
            let rates: [Cycle; 4] = [9600, 38400, 57600, 115200];
            let data_bits: Cycle = if self.sio_control & 0x80 != 0 { 8 } else { 7 };
            let bits = 1 + data_bits + if self.sio_control & 0x200 != 0 { 1 } else { 0 } + 1;
            let rate = rates[(self.sio_control & 3) as usize];
            self.uart_tx_completion_cycle =
                Some(self.current_cycle + (bits * 16_777_216).div_ceil(rate));
        }
    }

    fn write_sio_control(&mut self, value: u16) {
        let old_submode = (self.sio_control >> 12) & 3;
        let old_uart = self.is_uart_mode();
        let old_fifo = self.sio_control & 0x100 != 0;
        let new_submode = (value >> 12) & 3;
        self.sio_control = match new_submode {
            0 | 1 => (value & 0x7f8b) | 4,
            2 => (value & 0x7f83) | if old_submode == 2 { self.sio_control & 0x7c } else { 0x0c },
            3 => value & 0x7f8f,
            _ => unreachable!(),
        };
        self.sio_start_cycle = None;
        self.sio_completion_cycle = None;
        let uart = self.is_uart_mode();
        if old_uart != uart {
            self.clear_uart_state();
        }
        if uart {
            if old_uart && old_fifo && self.sio_control & 0x100 == 0 {
                self.uart_tx_fifo.clear();
                self.uart_rx_fifo.clear();
            }
            self.maybe_start_uart_transmit();
            return;
        }
        if self.rcnt & 0x8000 == 0 && new_submode <= 2 && self.sio_control & 0x80 != 0 {
            self.sio_start_cycle = Some(self.current_cycle);
            if new_submode <= 1 {
                self.sio_receive_value = u32::MAX;
                if self.sio_control & 1 != 0 {
                    let bits: Cycle = if new_submode == 0 { 8 } else { 32 };
                    let cycles_per_bit = if self.sio_control & 2 != 0 { 8 } else { 64 };
                    self.sio_completion_cycle = Some(self.current_cycle + bits * cycles_per_bit);
                }
            }
        }
    }

    fn write_rcnt(&mut self, value: u16) {
        let old_uart = self.is_uart_mode();
        let old_joy_bus = self.rcnt & 0xc000 == 0xc000;
        self.rcnt = if value & 0xc000 == 0x8000 { value & 0xc1ff } else { value & 0xc1f0 };
        if old_uart != self.is_uart_mode() {
            self.clear_uart_state();
        }
        if old_joy_bus != (self.rcnt & 0xc000 == 0xc000) {
            self.joy_pending_command = None;
            self.joy_completion_cycle = None;
            self.joy_response = None;
        }
        if self.rcnt & 0x8000 != 0 {
            self.sio_start_cycle = None;
            self.sio_completion_cycle = None;
            self.sio_receive_value = u32::MAX;
        }
    }

    fn clear_uart_state(&mut self) {
        self.uart_tx_fifo.clear();
        self.uart_rx_fifo.clear();
        self.uart_tx_active = None;
        self.uart_tx_completion_cycle = None;
        self.uart_tx_peer_scheduled = false;
        self.uart_rx_arrival_cycle = None;
        self.uart_rx_arrival_value = 0;
        self.uart_error = false;
        self.uart_cts_ready = false;
    }

    fn complete_sio_transfer(&mut self) {
        self.sio_start_cycle = None;
        self.sio_completion_cycle = None;
        self.sio_control &= !0x80;
        match (self.sio_control >> 12) & 3 {
            0 => self.sio_send = (self.sio_send & 0xff00) | (self.sio_receive_value as u8 as u16),
            1 => {
                self.sio_data[0] = self.sio_receive_value as u16;
                self.sio_data[1] = (self.sio_receive_value >> 16) as u16;
            }
            2 => {
                self.sio_data = self.sio_multiplayer_receive;
                self.sio_control =
                    (self.sio_control & !0x30) | (u16::from(self.sio_multiplayer_id) << 4);
            }
            _ => return,
        }
        if self.sio_control & 0x4000 != 0 {
            self.irq.interrupt_request_flags |= 1 << 7;
            self.update_irq_line();
        }
    }

    #[inline(never)]
    fn complete_uart_transmit(&mut self) {
        let old = self.uart_status();
        self.uart_tx_active = None;
        self.uart_tx_completion_cycle = None;
        self.uart_tx_peer_scheduled = false;
        self.uart_cts_ready = false;
        self.maybe_start_uart_transmit();
        self.raise_uart_status_edges(old);
    }

    fn complete_uart_receive(&mut self) {
        let old = self.uart_status();
        self.uart_rx_arrival_cycle = None;
        if self.is_uart_mode()
            && self.sio_control & 0x800 != 0
            && !self
                .uart_rx_fifo
                .push(self.uart_rx_arrival_value, if self.sio_control & 0x100 != 0 { 4 } else { 1 })
        {
            self.uart_error = true;
        }
        self.raise_uart_status_edges(old);
    }

    fn complete_joy_bus_command(&mut self) {
        let Some(command) = self.joy_pending_command.take() else {
            return;
        };
        self.joy_completion_cycle = None;
        let mut response = JoyBusResponse::default();
        let reset = matches!(command, JoyBusCommand::Reset);
        match command {
            JoyBusCommand::Status | JoyBusCommand::Reset => {
                if reset {
                    self.joy_control |= 1;
                    self.raise_joy_bus_irq();
                }
                response.data[..3].copy_from_slice(&[0, 4, self.joy_status as u8]);
                response.len = 3;
            }
            JoyBusCommand::WriteToGba(value) => {
                self.joy_receive = value;
                self.joy_control |= 1 << 1;
                self.joy_status |= 1 << 1;
                response.data[0] = self.joy_status as u8;
                response.len = 1;
                self.raise_joy_bus_irq();
            }
            JoyBusCommand::ReadFromGba => {
                response.data[..4].copy_from_slice(&self.joy_transmit.to_le_bytes());
                response.data[4] = self.joy_status as u8;
                response.len = 5;
                self.joy_control |= 1 << 2;
                self.joy_status &= !(1 << 3);
                self.raise_joy_bus_irq();
            }
        }
        self.joy_response = Some(response);
    }

    fn raise_joy_bus_irq(&mut self) {
        if self.joy_control & 0x40 != 0 {
            self.irq.interrupt_request_flags |= 1 << 7;
            self.update_irq_line();
        }
    }
    fn read_rcnt(&self) -> u16 {
        let serial_lines = if (self.sio_control >> 12) & 3 <= 1 { 5 } else { 15 };
        match self.rcnt & 0xc000 {
            0 | 0x4000 => (self.rcnt & 0xc1f0) | serial_lines,
            0x8000 => self.rcnt & 0xc1ff,
            0xc000 => (self.rcnt & 0xc1f0) | 0x000c,
            _ => unreachable!(),
        }
    }

    fn synchronize_devices(&mut self) {
        self.flush_device_batch();
        let requests = self.sync_apu_to(self.current_cycle);
        debug_assert_eq!(requests, 0);
        self.sync_timers_to(self.current_cycle);
    }

    #[inline(never)]
    fn flush_device_batch(&mut self) {
        if !self.batch_active || self.current_cycle == self.batch_device_cycle {
            return;
        }
        let cycles = self.current_cycle - self.batch_device_cycle;
        debug_assert!(self.current_cycle < self.batch_deadline);
        self.ppu_timing.elapse_before_boundary(cycles);
        if !self.devices_deferred {
            self.sync_apu_to(self.current_cycle);
            self.sync_timers_to(self.current_cycle);
        }
        self.batch_device_cycle = self.current_cycle;
    }

    fn sync_apu_to(&mut self, target: Cycle) -> u8 {
        let cycles = target - self.apu_cycle;
        if cycles == 0 {
            return 0;
        }
        let from = self.apu_cycle;
        self.apu_cycle = target;
        if self.fifo_clock.delegated() {
            self.apu.elapse_clocked(from, target, &mut self.fifo_clock)
        } else {
            self.apu.elapse(cycles);
            0
        }
    }

    fn sync_timers_to(&mut self, target: Cycle) {
        let cycles = target - self.timer_cycle;
        if cycles == 0 {
            return;
        }
        self.timer_cycle = target;
        for (index, timer) in self.timers.iter_mut().enumerate() {
            if index != 0 && timer.count_up_enabled() {
                continue;
            }
            if index < 2 && self.fifo_clock.next[index] != crate::apu::FifoClock::inactive {
                timer.elapse(cycles);
            } else {
                timer.elapse_before_overflow(cycles);
            }
        }
    }

    fn next_timer_overflow(&self) -> Cycle {
        if self.devices_deferred {
            return self.timer_deadline;
        }
        self.compute_timer_deadline()
    }

    #[inline(always)]
    fn compute_timer_deadline(&self) -> Cycle {
        self.timers
            .iter()
            .enumerate()
            .filter_map(|(index, timer)| {
                if index != 0 && timer.count_up_enabled() {
                    None
                } else if index < 2
                    && self.fifo_clock.next[index] != crate::apu::FifoClock::inactive
                {
                    self.apu.overflows_until_refill(index).map(|overflows| {
                        self.fifo_clock.next[index]
                            + (overflows - 1) * self.fifo_clock.period[index]
                    })
                } else {
                    timer
                        .cycles_until_overflow()
                        .map(|cycles| self.timer_cycle.saturating_add(cycles))
                }
            })
            .min()
            .unwrap_or(Cycle::MAX)
    }

    #[inline(never)]
    fn refresh_timer_deadline(&mut self) {
        if self.devices_deferred {
            self.timer_deadline = self.compute_timer_deadline();
        }
    }

    #[inline(never)]
    fn refresh_fifo_clock(&mut self) {
        self.fifo_clock = crate::apu::FifoClock::default();
        if !self.devices_deferred {
            return;
        }
        debug_assert_eq!(self.apu_cycle, self.timer_cycle);
        for index in 0..2 {
            let timer = &self.timers[index];
            if !timer.enabled() || timer.irq_enabled() || (index != 0 && timer.count_up_enabled()) {
                continue;
            }
            let successor = &self.timers[index + 1];
            if successor.enabled() && successor.count_up_enabled() {
                continue;
            }
            self.fifo_clock.next[index] = self.timer_cycle + timer.cycles_until_overflow().unwrap();
            self.fifo_clock.period[index] = timer.overflow_period();
        }
    }

    fn refresh_after_apu_write(&mut self, address: u32) {
        let aligned = address & !1;
        if aligned == MMIO_BASE + 0x82 || aligned == MMIO_BASE + 0x84 {
            self.refresh_timer_deadline();
        }
    }

    fn write_fifo(&mut self, address: u32, value: u32, byte_count: u8) {
        if !self.fifo_clock.delegated() {
            self.apu.write_fifo(address, value, byte_count);
            return;
        }
        if self.fifo_clock.next[0].min(self.fifo_clock.next[1]) <= self.current_cycle {
            self.sync_apu_to(self.current_cycle);
        }
        self.apu.write_fifo(address, value, byte_count);
        if !self.apu.fifo_needs_refill(address) {
            return;
        }
        self.refresh_timer_deadline();
        if self.batch_active && self.timer_deadline < self.batch_deadline {
            self.batch_deadline = self.timer_deadline;
        }
    }

    fn next_device_boundary(&self) -> Cycle {
        let mut boundary =
            self.current_cycle.saturating_add(self.ppu_timing.cycles_until_next_event());
        boundary = boundary.min(self.next_timer_overflow());
        for at in [
            self.sio_completion_cycle,
            self.uart_tx_completion_cycle,
            self.uart_rx_arrival_cycle,
            self.joy_completion_cycle,
        ]
        .into_iter()
        .flatten()
        {
            boundary = boundary.min(at);
        }
        for at in self.dma_start_cycles.into_iter().flatten() {
            boundary = boundary.min(at);
        }
        if let Some(at) = self.irq.irq_line_rises_at {
            boundary = boundary.min(at);
        }
        if let Some(at) = self.next_external_input_cycle() {
            boundary = boundary.min(at);
        }
        boundary
    }

    #[inline(always)]
    fn advance(&mut self, cycles: Cycle) {
        let end = self.current_cycle + cycles;
        if self.batch_active && end < self.batch_deadline {
            self.current_cycle = end;
            self.advance_game_pak_prefetch(cycles);
            return;
        }
        self.advance_slow(cycles);
    }

    #[inline(never)]
    fn advance_slow(&mut self, cycles: Cycle) {
        if !self.batch_active {
            self.advance_eager(cycles, 0);
            return;
        }
        let next = self.batch_deadline;
        self.flush_device_batch();
        self.batch_active = false;
        self.batch_deadline = 0;
        self.advance_eager(cycles, next);
        self.batch_active = true;
        self.batch_device_cycle = self.current_cycle;
        self.batch_deadline = self.next_device_boundary();
    }

    #[inline(always)]
    fn advance_eager(&mut self, cycles: Cycle, first_boundary: Cycle) {
        let mut remaining = cycles;
        let mut known_boundary = first_boundary;
        while remaining != 0 {
            let step = if known_boundary > self.current_cycle {
                let step = remaining.min(known_boundary - self.current_cycle);
                known_boundary = 0;
                step
            } else if remaining == 1 {
                1
            } else {
                remaining.min(self.next_device_boundary().saturating_sub(self.current_cycle))
            };
            assert!(step != 0, "event deadline must be strictly future");
            self.advance_step(step);
            remaining -= step;
            if !self.servicing_dma && self.pending_dma != 0 {
                self.service_pending_dma();
            }
        }
    }

    #[inline(never)]
    fn advance_step(&mut self, cycles: Cycle) {
        self.advance_game_pak_prefetch(cycles);
        let mut events = crate::video::timing::Events::default();
        self.ppu_timing.elapse(cycles, &mut events);
        let end = self.current_cycle + cycles;
        let mut audio_requests = 0;
        let mut timer_irq = false;
        if !self.devices_deferred {
            audio_requests |= self.sync_apu_to(end);
        }
        if !self.devices_deferred || end >= self.timer_deadline {
            debug_assert!(!self.devices_deferred || end == self.timer_deadline);
            let elapsed = end - self.timer_cycle;
            self.timer_cycle = end;
            let mut previous = 0;
            for index in 0..4 {
                let timer = &mut self.timers[index];
                let overflows = if index != 0 && timer.count_up_enabled() {
                    timer.cascade(previous)
                } else {
                    timer.elapse(elapsed)
                };
                if overflows != 0 && timer.irq_enabled() {
                    self.irq.interrupt_request_flags |= 1 << (index + 3);
                    timer_irq = true;
                }
                if index < 2
                    && overflows != 0
                    && self.fifo_clock.next[index] == crate::apu::FifoClock::inactive
                {
                    audio_requests |= self.sync_apu_to(end);
                    audio_requests |= self.apu.timer_overflow(index, overflows);
                }
                previous = overflows;
            }
            if self.devices_deferred {
                audio_requests |= self.sync_apu_to(end);
                self.refresh_fifo_clock();
            }
            self.refresh_timer_deadline();
        }
        self.current_cycle = end;
        if timer_irq {
            self.update_irq_line();
        }
        self.apply_due_external_inputs();
        for (index, at) in self.dma_start_cycles.iter_mut().enumerate() {
            if *at == Some(self.current_cycle) {
                *at = None;
                self.pending_dma |= 1 << index;
            }
        }
        if self.sio_completion_cycle == Some(self.current_cycle) {
            self.complete_sio_transfer();
        }
        if self.uart_tx_completion_cycle == Some(self.current_cycle) {
            self.complete_uart_transmit();
        }
        if self.uart_rx_arrival_cycle == Some(self.current_cycle) {
            self.complete_uart_receive();
        }
        if self.joy_completion_cycle == Some(self.current_cycle) {
            self.complete_joy_bus_command();
        }
        if events.entered_vblank
            || events.hblank_any
            || events.frames_completed != 0
            || events.vblank_irq
            || events.hblank_irq
            || events.vcount_irq
        {
            // By reference, not moved: see `PpuTiming::elapse`.
            self.deliver_lcd_events(&events);
        }
        if audio_requests != 0 {
            self.queue_audio_dma(audio_requests);
        }
        self.irq.reach(end);
    }

    fn deliver_lcd_events(&mut self, events: &crate::video::timing::Events) {
        for line in events.hblank_lines.iter() {
            if self.video_render_current {
                self.render_scanline(line);
                if line == SCREEN_HEIGHT - 1 {
                    self.video_framebuffer_canonical = true;
                    self.video_frame_id = self.completed_frame_count + 1;
                    self.video_frame_cycle = self.current_cycle;
                    if self.video_publication_active {
                        self.video_published_pixels.copy_from_slice(&self.frame_pixels[..]);
                        if self.video_active_interval == 1 && self.video_requested_interval == 1 {
                            self.video_publication_active = false;
                        }
                    }
                }
            } else {
                self.video_framebuffer_canonical = false;
            }
            self.advance_affine_references(line);
        }
        if events.frames_completed != 0 {
            self.bg_affine_current = self.bg_reference_points;
            self.scanline_memo.mark_registers();
            self.scroll_segment_line = 0xffff;
            self.backdrop_segment_line = 0xffff;
            self.win_segment_line = 0xffff;
            self.blend_segment_line = 0xffff;
            self.affine_segment_line = 0xffff;
            self.mosaic_segment_line = 0xffff;
            debug_assert_eq!(events.frames_completed, 1);
            self.latch_video_frame();
        }
        self.completed_frame_count += events.frames_completed;
        if events.vblank_irq {
            self.irq.interrupt_request_flags |= 1;
        }
        if events.hblank_irq {
            self.irq.interrupt_request_flags |= 1 << 1;
        }
        if events.vcount_irq {
            self.irq.interrupt_request_flags |= 1 << 2;
        }
        self.update_irq_line();
        if events.entered_vblank {
            self.queue_dma_timing(1);
        }
        if events.hblank_any {
            self.queue_dma_timing(2);
        }
    }

    fn service_pending_dma(&mut self) {
        if self.pending_dma == 0 || self.servicing_dma {
            return;
        }
        self.dma_burst_start = self.current_cycle;
        self.dma_yield_at = self.current_cycle.saturating_add(Self::MAX_DMA_BURST_CYCLES);
        self.run_dma_burst();
    }

    /// Longest a DMA burst runs inside the CPU access it follows: about
    /// one second of guest time and many
    /// times the longest single transfer. Only requests that return faster
    /// than transfers complete keep the bus that long, and then the CPU
    /// never reaches it again. Servicing such a burst inside one access
    /// would never return to the host, so at this limit the burst yields
    /// (see [`Self::run_dma_burst`]) and the machine continues it from the
    /// next instruction boundary with [`Self::continue_dma_stall`].
    pub const MAX_DMA_BURST_CYCLES: Cycle = 1 << 24;

    /// Runs pending DMA until no request remains, resuming any transfers a
    /// stall interrupted first. At the first transfer start or beat
    /// boundary at or after `dma_yield_at` the burst yields instead: it
    /// records the transfers in progress, keeps its requests, sets
    /// `dma_stalled` and leaves `servicing_dma` set, so no other burst
    /// starts until [`Self::continue_dma_stall`] resumes this one exactly
    /// where it stopped.
    #[inline(never)]
    fn run_dma_burst(&mut self) {
        self.servicing_dma = true;
        self.dma_stalled = false;
        let sub_batch = !self.batch_active;
        if sub_batch {
            // advance_slow temporarily clears batch_active while preserving
            // devices_deferred. A DMA sub-batch must preserve that deferral.
            self.batch_device_cycle = self.current_cycle;
            self.batch_active = true;
            self.batch_deadline = self.next_device_boundary();
        }
        if self.dma_progress_mask != 0 {
            // The outermost interrupted transfer resumes the others.
            self.run_dma(7 - self.dma_progress_mask.leading_zeros() as usize);
        }
        while !self.dma_stalled && self.pending_dma != 0 {
            let index = self.pending_dma.trailing_zeros() as usize;
            self.pending_dma &= !(1 << index);
            if self.dma_channels[index].enabled() {
                if self.dma_yield_due(index) {
                    break;
                }
                self.run_dma(index);
            }
        }
        if sub_batch {
            self.flush_device_batch();
            self.batch_active = false;
            self.batch_device_cycle = 0;
            self.batch_deadline = 0;
        }
        self.servicing_dma = self.dma_stalled;
    }

    /// Whether the burst yields before channel `index`'s transfer starts;
    /// if so, its request stays pending.
    #[inline(always)]
    fn dma_yield_due(&mut self, index: usize) -> bool {
        if self.current_cycle < self.dma_yield_at {
            return false;
        }
        self.pending_dma |= 1 << index;
        self.dma_stalled = true;
        true
    }

    /// Whether a DMA burst owns the bus across host calls: the CPU is
    /// stalled at an instruction boundary until the burst ends.
    pub fn dma_stalled(&self) -> bool {
        self.dma_stalled
    }

    /// Continues a stalled burst until it releases the bus or reaches its
    /// first transfer start or beat boundary at or after `until`.
    pub fn continue_dma_stall(&mut self, until: Cycle) {
        debug_assert!(self.dma_stalled && self.servicing_dma);
        if until <= self.current_cycle {
            return;
        }
        self.dma_yield_at = until;
        self.run_dma_burst();
    }

    fn clear_dma_stall(&mut self) {
        self.dma_stalled = false;
        self.dma_progress = [None; 4];
        self.dma_progress_mask = 0;
    }

    /// Records channel `index`'s transfer, which yields after `beat`.
    #[cold]
    #[inline(never)]
    fn yield_dma_transfer(&mut self, index: usize, progress: DmaProgress) {
        self.dma_stalled = true;
        self.dma_progress[index] = Some(progress);
        self.dma_progress_mask |= 1 << index;
    }

    fn run_dma(&mut self, index: usize) {
        let previous_active = self.active_dma_channel;
        self.active_dma_channel = Some(index as u8);
        let resumed = self.dma_progress[index].take();
        let (mut source, mut destination, control, is_audio_fifo, transfer_words) = match resumed {
            Some(progress) => (
                progress.source,
                progress.destination,
                progress.control,
                progress.is_audio_fifo,
                progress.transfer_words,
            ),
            None => {
                self.dma_epoch = self.dma_epoch.wrapping_add(1);
                let destination = self.dma_channels[index].latched_destination;
                let control = self.dma_channels[index].control;
                let is_audio_fifo = (1..=2).contains(&index)
                    && (control >> 12) & 3 == 3
                    && Apu::fifo_for_address(destination).is_some();
                let transfer_words =
                    if is_audio_fifo { 4 } else { self.dma_channels[index].transfer_words(index) };
                (
                    self.dma_channels[index].latched_source,
                    destination,
                    control,
                    is_audio_fifo,
                    transfer_words,
                )
            }
        };
        let width = if is_audio_fifo || control & (1 << 10) != 0 {
            AccessWidth::Word
        } else {
            AccessWidth::Half
        };
        let unit_bytes = width.bytes() as u32;
        let mut did_access_rom = false;
        let bulk;
        let mut transferred = 0;
        if let Some(progress) = resumed {
            self.dma_progress_mask &= !(1 << index);
            did_access_rom = progress.did_access_rom;
            bulk = progress.bulk;
            transferred = progress.beat;
            // It yielded after this beat, where it lets higher-priority
            // requests in.
            self.service_higher_priority_dma(index);
            if self.dma_stalled || self.current_cycle >= self.dma_yield_at {
                self.yield_dma_transfer(index, progress);
                self.active_dma_channel = previous_active;
                return;
            }
            transferred += 1;
        } else {
            if index == 3
                && width == AccessWidth::Half
                && self.is_eeprom_address(destination)
                && self.save.is_auto()
            {
                self.save.resolve_eeprom_transfer(transfer_words);
            }
            bulk = !is_audio_fifo
                && (source == 0 || matches!(source >> 24, 2 | 3))
                && matches!(destination >> 24, 2 | 3 | 6 | 7);
        }
        while transferred < transfer_words {
            if bulk {
                let remaining = transfer_words - transferred;
                transferred +=
                    self.dma_bulk(width, control, &mut source, &mut destination, remaining);
                if transferred == transfer_words {
                    break;
                }
            }
            let mut source_kind = AccessKind::Sequential;
            let mut dest_kind = AccessKind::Sequential;
            if !did_access_rom {
                if (8..=13).contains(&(source >> 24)) {
                    source_kind = AccessKind::Nonsequential;
                    did_access_rom = true;
                } else if (8..=13).contains(&(destination >> 24)) {
                    dest_kind = AccessKind::Nonsequential;
                    did_access_rom = true;
                }
            }
            let value = if source == 0 {
                self.advance(1);
                self.open_bus_value = self.dma_transfer_latch;
                self.dma_transfer_latch
            } else {
                let value = self.read_dma_source(source, width, source_kind);
                self.dma_transfer_latch =
                    if width == AccessWidth::Half { (value & 0xffff) * 0x0001_0001 } else { value };
                value
            };
            self.write_dma_destination(destination, width, dest_kind, value);
            let source_control = ((control >> 7) & 3) as u8;
            source = if source == 0 {
                0
            } else if (8..=13).contains(&(source >> 24)) && source_control < 3 {
                source.wrapping_add(unit_bytes)
            } else {
                adjusted_dma_address(source, unit_bytes, source_control)
            };
            if !is_audio_fifo {
                destination =
                    adjusted_dma_address(destination, unit_bytes, ((control >> 5) & 3) as u8);
            }
            if transferred + 1 < transfer_words {
                if self.current_cycle < self.dma_yield_at {
                    self.service_higher_priority_dma(index);
                }
                if self.dma_stalled || self.current_cycle >= self.dma_yield_at {
                    let progress = DmaProgress {
                        source,
                        destination,
                        control,
                        transfer_words,
                        beat: transferred,
                        did_access_rom,
                        is_audio_fifo,
                        bulk,
                    };
                    self.yield_dma_transfer(index, progress);
                    self.active_dma_channel = previous_active;
                    return;
                }
            }
            transferred += 1;
        }
        self.advance(2);
        self.dma_channels[index].latched_source = source;
        let repeat = control & (1 << 9) != 0 && (control >> 12) & 3 != 0;
        if repeat && (control >> 5) & 3 == 3 {
            self.dma_channels[index].reload_latched_destination();
        } else {
            self.dma_channels[index].latched_destination = destination;
        }
        if repeat {
            let channel = &mut self.dma_channels[index];
            channel.latched_word_count = channel.word_count;
        }
        if !repeat {
            self.dma_channels[index].control &= !(1 << 15);
        }
        if is_audio_fifo {
            self.refresh_timer_deadline();
        }
        if control & (1 << 14) != 0 {
            self.irq.interrupt_request_flags |= 1 << (index + 8);
            self.update_irq_line();
        }
        self.active_dma_channel = previous_active;
    }

    /// Runs up to `remaining` beats of a DMA from RAM (or the latch) to RAM,
    /// VRAM or OAM at once, if nothing can observe them one at a time: they end
    /// before the device deadline, no DMA is pending, and each address stays
    /// in its region. Returns how many beats ran (possibly none); the beats
    /// leave the machine as the per-beat loop in [`Self::run_dma`] would.
    ///
    /// Beats advance a local clock; devices, which the per-beat loop brings
    /// up to each VRAM write, are brought up at the batch's end instead, and
    /// the prefetch buffer advances by the total. A VRAM write on a visible
    /// dot whose line's history has room goes through the per-beat path at
    /// its own cycle, so its raster history is recorded exactly.
    fn dma_bulk(
        &mut self,
        width: AccessWidth,
        control: u16,
        source: &mut u32,
        destination: &mut u32,
        remaining: u32,
    ) -> u32 {
        if !self.batch_active || self.pending_dma != 0 || self.dma_yield_at <= self.current_cycle {
            return 0;
        }
        #[cfg(test)]
        if !tests::BULK_DMA.get() {
            return 0;
        }
        let read_cost = match *source >> 24 {
            _ if *source == 0 => 1,
            2 => ewram_cycles(width),
            3 => 1,
            _ => return 0,
        };
        let target = *destination >> 24;
        let write_cost = match target {
            2 => ewram_cycles(width),
            3 | 7 => 1,
            6 => video_cycles(width),
            _ => return 0,
        };
        let unit = width.bytes() as u32;
        let (source_control, destination_control) =
            (((control >> 7) & 3) as u8, ((control >> 5) & 3) as u8);
        let room = self.batch_deadline.min(self.dma_yield_at) - self.current_cycle;
        let mut beats = u64::from(remaining).min((room - 1) / (read_cost + write_cost)) as u32;
        if *source != 0 {
            beats = beats.min(dma_beats_in_region(*source, unit, source_control));
        }
        beats = beats.min(dma_beats_in_region(*destination, unit, destination_control));
        if beats == 0 {
            return 0;
        }
        let start = self.current_cycle;
        let mut timing = self.ppu_timing;
        timing.elapse_before_boundary(start - self.batch_device_cycle);
        let mut at = BeatClock { cycle: start, line: timing.line, dot: timing.cycle_in_line };
        let mut open_bus = self.open_bus_value;
        let quiet = if target == 6 && destination_control == 0 {
            self.dma_vram_run(width, source_control, source, destination, beats, &mut at)
        } else {
            None
        };
        if let Some((_, value)) = quiet {
            open_bus = value;
        }
        for _ in quiet.map_or(0, |(beats, _)| beats)..beats {
            let value = if *source == 0 {
                at.elapse(1);
                open_bus = self.dma_transfer_latch;
                self.dma_transfer_latch
            } else {
                let value = if *source >> 24 == 2 {
                    self.read_ewram(*source, width)
                } else {
                    self.read_iwram(*source, width)
                };
                open_bus = value;
                at.elapse(read_cost);
                self.dma_transfer_latch =
                    if width == AccessWidth::Half { (value & 0xffff) * 0x0001_0001 } else { value };
                value
            };
            match target {
                2 => {
                    at.elapse(write_cost);
                    self.write_ewram(*destination, width, value);
                }
                3 => {
                    at.elapse(1);
                    self.write_iwram(*destination, width, value);
                }
                7 => {
                    at.elapse(1);
                    self.video_memory().write_oam(*destination, width, value);
                }
                _ => {
                    for half in 0..unit / 2 {
                        let address = destination.wrapping_add(half * 2);
                        let value = value >> (half * 16);
                        at.elapse(1);
                        let visible = usize::from(at.line) < SCREEN_HEIGHT && at.dot < 960;
                        if visible && !self.vram_history_full_on(at.line) {
                            self.current_cycle = at.cycle;
                            self.flush_device_batch();
                            self.write_vram_at_current_dot(address, AccessWidth::Half, value);
                        } else {
                            self.video_memory().write_vram(address, AccessWidth::Half, value);
                        }
                    }
                }
            }
            if *source != 0 {
                *source = adjusted_dma_address(*source, unit, source_control);
            }
            *destination = adjusted_dma_address(*destination, unit, destination_control);
        }
        let cycle = at.cycle;
        debug_assert!(cycle < self.batch_deadline);
        self.current_cycle = cycle;
        self.advance_game_pak_prefetch(cycle - start);
        self.open_bus_value = open_bus;
        #[cfg(test)]
        tests::BULK_BEATS.set(tests::BULK_BEATS.get() + u64::from(beats));
        beats
    }

    /// The first beats of a bulk DMA (see [`Self::dma_bulk`]) to VRAM with an
    /// incrementing destination, that write only where no visible dot sees
    /// them and below the mirrored upper 32 KiB, from the latch, a fixed
    /// source or an incrementing one in a RAM mirror: one comparison and copy
    /// per memo page, which is marked only if it changed, as the beat loop
    /// marks it. Returns how many beats ran and the last value read (the
    /// open-bus latch), or None for none.
    #[inline(never)]
    fn dma_vram_run(
        &mut self,
        width: AccessWidth,
        source_control: u8,
        source: &mut u32,
        destination: &mut u32,
        beats: u32,
        at: &mut BeatClock,
    ) -> Option<(u32, u32)> {
        let unit = width.bytes() as u32;
        let fixed = *source == 0 || source_control == 2;
        let offset = destination.wrapping_sub(VRAM_BASE) & 0x1_ffff;
        if (!fixed && source_control != 0) || offset & (unit - 1) != 0 || offset >= 0x1_8000 {
            return None;
        }
        let read_cost = match *source >> 24 {
            _ if *source == 0 => 1,
            2 => ewram_cycles(width),
            _ => 1,
        };
        let beat_cost = read_cost + video_cycles(width);
        // Positions after this one the PPU shows no dot a write could
        // change: to the end of an HBlank, of VBlank, or of a line whose
        // raster history is full.
        let (line, dot) = (Cycle::from(at.line), Cycle::from(at.dot));
        let quiet = if line >= SCREEN_HEIGHT as Cycle {
            (227 - line) * 1232 + 1231 - dot
        } else if dot >= 959 || self.vram_history_full_on(at.line) {
            1231 - dot
        } else {
            0
        };
        let mut count = u64::from(beats).min(quiet / beat_cost) as u32;
        count = count.min((0x1_8000 - offset) / unit);
        let mask = if *source >> 24 == 2 { EWRAM_SIZE } else { IWRAM_SIZE } as u32 - 1;
        if !fixed {
            count = count.min((mask + 1 - (*source & mask)) / unit);
        }
        if count == 0 {
            return None;
        }
        let bytes = (count * unit) as usize;
        let first = offset as usize;
        let (value, pattern) = if *source == 0 {
            (self.dma_transfer_latch, Some(self.dma_transfer_latch))
        } else if fixed {
            let value = if *source >> 24 == 2 {
                self.read_ewram(*source, width)
            } else {
                self.read_iwram(*source, width)
            };
            let pattern =
                if width == AccessWidth::Half { (value & 0xffff) * 0x0001_0001 } else { value };
            self.dma_transfer_latch = pattern;
            (value, Some(pattern))
        } else {
            let last = source.wrapping_add((count - 1) * unit);
            let value = if *source >> 24 == 2 {
                self.read_ewram(last, width)
            } else {
                self.read_iwram(last, width)
            };
            self.dma_transfer_latch =
                if width == AccessWidth::Half { (value & 0xffff) * 0x0001_0001 } else { value };
            (value, None)
        };
        let from = (*source & mask) as usize;
        let ram: &[u8] = if *source >> 24 == 2 { &self.ewram[..] } else { &self.iwram[..] };
        let mut page = first;
        while page < first + bytes {
            let end = ((page / scanline_memo::PAGE_SIZE + 1) * scanline_memo::PAGE_SIZE)
                .min(first + bytes);
            let target = &mut self.vram[page..end];
            let changed = match pattern {
                Some(pattern) => {
                    // The pattern's bytes as they fall from this page offset.
                    let word = pattern.rotate_right(((page & 3) * 8) as u32).to_le_bytes();
                    let (chunks, rest) = target.as_chunks::<4>();
                    let changed =
                        chunks.iter().any(|chunk| *chunk != word) || rest != &word[..rest.len()];
                    if changed {
                        for (i, byte) in target.iter_mut().enumerate() {
                            *byte = word[i & 3];
                        }
                    }
                    changed
                }
                None => {
                    let data = &ram[from + (page - first)..from + (end - first)];
                    let changed = target != data;
                    target.copy_from_slice(data);
                    changed
                }
            };
            if changed {
                self.scanline_memo.mark_vram(page);
            }
            page = end;
        }
        at.elapse_many(Cycle::from(count) * beat_cost);
        #[cfg(test)]
        tests::VRAM_RUN_BEATS.set(tests::VRAM_RUN_BEATS.get() + u64::from(count));
        if !fixed {
            *source = source.wrapping_add(count * unit);
        }
        *destination = destination.wrapping_add(count * unit);
        Some((count, value))
    }

    fn read_dma_source(&mut self, address: u32, width: AccessWidth, kind: AccessKind) -> u32 {
        let access = Access { address, width, kind, source: AccessSource::Dma };
        match address >> 24 {
            2 => self.complete_read(self.read_ewram(address, width), ewram_cycles(width)).value,
            3 => self.complete_read(self.read_iwram(address, width), 1).value,
            8..=13
                if !(width == AccessWidth::Half
                    && self.save.is_eeprom()
                    && self.is_eeprom_address(address)) =>
            {
                self.read_game_pak(access).value
            }
            _ => self.read(access).value,
        }
    }

    fn write_dma_destination(
        &mut self,
        address: u32,
        width: AccessWidth,
        kind: AccessKind,
        value: u32,
    ) {
        match address >> 24 {
            2 => {
                self.advance(ewram_cycles(width));
                self.write_ewram(address, width, value);
            }
            3 => {
                self.advance(1);
                self.write_iwram(address, width, value);
            }
            6 => self.write_vram_timed(address, width, value),
            _ if Apu::accepts_fifo_write(address, width.bytes() as u8) => {
                self.advance(1);
                self.write_fifo(address, value, width.bytes() as u8);
            }
            _ => {
                self.write(Access { address, width, kind, source: AccessSource::Dma }, value);
                return;
            }
        }
        self.service_pending_dma();
    }

    #[inline(always)]
    fn service_higher_priority_dma(&mut self, current_index: usize) {
        if current_index == 0 {
            return;
        }
        let higher_mask = (1u8 << current_index) - 1;
        if self.dma_progress_mask & higher_mask != 0 {
            // A stall interrupted a higher-priority transfer in here.
            let index = 7 - (self.dma_progress_mask & higher_mask).leading_zeros() as usize;
            self.run_dma(index);
            if self.dma_stalled {
                return;
            }
        }
        while self.pending_dma & higher_mask != 0 {
            let index = (self.pending_dma & higher_mask).trailing_zeros() as usize;
            self.pending_dma &= !(1 << index);
            if self.dma_channels[index].enabled() {
                if self.dma_yield_due(index) {
                    return;
                }
                self.run_dma(index);
                if self.dma_stalled {
                    return;
                }
            }
        }
    }

    #[inline(always)]
    fn queue_dma_timing(&mut self, timing: u8) {
        for (index, channel) in self.dma_channels.iter().enumerate() {
            if channel.enabled()
                && channel.start_timing() == timing
                && self.dma_start_cycles[index].is_none()
            {
                self.dma_start_cycles[index] = Some(self.current_cycle.saturating_add(3));
            }
        }
    }

    fn queue_audio_dma(&mut self, requests: u8) {
        for index in 1..3 {
            let channel = &self.dma_channels[index];
            if !channel.enabled()
                || channel.start_timing() != 3
                || self.active_dma_channel == Some(index as u8)
            {
                continue;
            }
            if let Some(fifo) = Apu::fifo_for_address(channel.latched_destination)
                && requests & (1 << fifo as u8) != 0
            {
                self.pending_dma |= 1 << index;
            }
        }
    }

    fn latch_video_frame(&mut self) {
        if self.video_requested_interval != self.video_active_interval {
            self.video_active_interval = self.video_requested_interval;
            self.video_skip_remaining = 0;
        }
        self.video_render_current = self.video_skip_remaining == 0;
        if self.video_render_current {
            self.video_skip_remaining = self.video_active_interval - 1;
        } else {
            self.video_skip_remaining -= 1;
        }
    }

    fn read_palette(&self, address: u32, width: AccessWidth) -> u32 {
        palette_read(&self.palette, address, width)
    }

    #[inline]
    pub(crate) fn write_palette(&mut self, address: u32, width: AccessWidth, value: u32) {
        self.video_memory().write_palette(address, width, value);
    }

    fn read_vram(&self, address: u32, width: AccessWidth) -> u32 {
        vram_read(&self.vram, self.dispcnt, address, width)
    }

    #[inline(always)]
    fn write_vram_timed(&mut self, address: u32, width: AccessWidth, value: u32) {
        if width == AccessWidth::Word {
            for beat in 0..2 {
                let at = address.wrapping_add(beat * 2);
                self.advance(1);
                self.flush_device_batch();
                self.write_vram_at_current_dot(at, AccessWidth::Half, value >> (beat * 16));
            }
        } else {
            self.advance(video_cycles(width));
            self.flush_device_batch();
            self.write_vram_at_current_dot(address, width, value);
        }
    }

    fn write_vram_at_current_dot(&mut self, address: u32, width: AccessWidth, value: u32) {
        // Raster history is observed only by visible dots. The write still
        // marks VRAM dirty and each beat still advances to its own endpoint.
        if usize::from(self.ppu_timing.line) >= SCREEN_HEIGHT
            || self.ppu_timing.cycle_in_line >= 960
            || self.vram_history_full()
        {
            self.write_vram(address, width, value);
            return;
        }
        let snapshot = self.snapshot_vram_write(address, width);
        self.write_vram(address, width, value);
        self.record_vram_write(snapshot);
    }

    #[inline]
    pub(crate) fn write_vram(&mut self, address: u32, width: AccessWidth, value: u32) {
        self.video_memory().write_vram(address, width, value);
    }

    fn read_oam(&self, address: u32, width: AccessWidth) -> u32 {
        oam_read(&self.oam, address, width)
    }

    #[inline]
    pub(crate) fn write_oam(&mut self, address: u32, width: AccessWidth, value: u32) {
        self.video_memory().write_oam(address, width, value);
    }

    /// Video memory and the render caches its writes invalidate.
    #[inline(always)]
    pub(crate) fn video_memory(&mut self) -> VideoMemory<'_> {
        VideoMemory {
            palette: &mut self.palette,
            vram: &mut self.vram,
            oam: &mut self.oam,
            memo: &mut self.scanline_memo,
            oam_compact_valid: &mut self.oam_compact_valid,
            object_line_index: &mut self.object_line_index,
            dispcnt: self.dispcnt,
        }
    }

    #[inline(always)]
    fn complete_read(&mut self, value: u32, cycles: Cycle) -> ReadResult {
        self.complete_read_latching(value, value, cycles)
    }

    /// [`Self::complete_read`] for a read that leaves `latch` on the data
    /// bus.
    #[inline(always)]
    fn complete_read_latching(&mut self, value: u32, latch: u32, cycles: Cycle) -> ReadResult {
        let started_at = self.current_cycle;
        self.open_bus_value = latch;
        self.advance(cycles);
        self.service_pending_dma();
        ReadResult { value, cycles: self.current_cycle - started_at }
    }

    /// [`Self::complete_read`] for `access`: an opcode fetch leaves
    /// [`Self::fetch_latch`] on the data bus, any other read its value.
    #[inline(always)]
    fn complete_access(&mut self, access: Access, value: u32, cycles: Cycle) -> ReadResult {
        let latch = if access.source == AccessSource::CpuCode {
            self.fetch_latch(access.address, access.width, value)
        } else {
            value
        };
        self.complete_read_latching(value, latch, cycles)
    }

    /// The word an opcode fetch of `value` at `address` leaves on the data
    /// bus, which a later read of unused memory returns (GBATEK,
    /// Unpredictable Things: Reading from Unused Memory). An ARM fetch
    /// leaves its word. A Thumb fetch over a 16-bit bus (EWRAM, palette RAM,
    /// VRAM, Game Pak ROM) presents its halfword on both halves; one from
    /// the 32-bit BIOS or OAM reads the whole aligned word; IWRAM drives
    /// only the addressed half, and the other half keeps the bus's previous
    /// value.
    #[inline(always)]
    fn fetch_latch(&self, address: u32, width: AccessWidth, value: u32) -> u32 {
        if width != AccessWidth::Half {
            return value;
        }
        match address >> 24 {
            0 if address < BIOS_SIZE as u32 => self.bios_latch,
            2 | 5 | 6 | 8..=13 => half_on_both_lanes(value),
            3 => iwram_half_latch(self.open_bus_value, address, value),
            7 => self.read_oam(address & !3, AccessWidth::Word),
            _ => value,
        }
    }

    fn read_game_pak(&mut self, mut access: Access) -> ReadResult {
        let started_at = self.current_cycle;
        let value = self.read_game_pak_rom(access.address, access.width);
        if access.source != AccessSource::CpuCode {
            self.stop_game_pak_prefetch(Some(access.source));
            let fetch_started = self.current_cycle;
            let epoch = self.dma_epoch;
            let cycles = self.game_pak_cycles(access);
            self.complete_read(value, cycles);
            if access.source == AccessSource::CpuData {
                self.reprice_interrupted_game_pak_access(access, fetch_started, cycles, epoch);
            }
            return ReadResult { value, cycles: self.current_cycle - started_at };
        }
        self.game_pak_code_active = true;
        if self.prefetch_disabled_bug_pending {
            access.kind = AccessKind::Nonsequential;
            self.prefetch_disabled_bug_pending = false;
        }
        let opcode_bytes = if access.width == AccessWidth::Word { 4 } else { 2 };
        let aligned = access.address & !(u32::from(opcode_bytes) - 1);
        if self.waitcnt & (1 << 14) != 0
            && self.game_pak_prefetch.active
            && self.game_pak_prefetch.opcode_bytes == opcode_bytes
        {
            if self.game_pak_prefetch.count != 0 && aligned == self.game_pak_prefetch.head_address {
                self.game_pak_prefetch.count -= 1;
                self.game_pak_prefetch.head_address =
                    self.game_pak_prefetch.head_address.wrapping_add(u32::from(opcode_bytes));
                if self.game_pak_prefetch.countdown == 0
                    && self.game_pak_prefetch.count < self.game_pak_prefetch.capacity
                {
                    self.continue_game_pak_prefetch(opcode_bytes);
                }
                self.complete_access(access, value, 1);
                return ReadResult { value, cycles: self.current_cycle - started_at };
            }
            if self.game_pak_prefetch.countdown != 0
                && aligned == self.game_pak_prefetch.last_address
            {
                let remaining = self.game_pak_prefetch.countdown;
                self.complete_access(access, value, remaining);
                self.game_pak_prefetch.head_address = aligned.wrapping_add(u32::from(opcode_bytes));
                self.game_pak_prefetch.count = 0;
                return ReadResult { value, cycles: self.current_cycle - started_at };
            }
        }
        self.stop_game_pak_prefetch(None);
        if self.prefetch_buffer_was_disabled {
            access.kind = AccessKind::Nonsequential;
            self.prefetch_buffer_was_disabled = false;
        }
        let fetch_started = self.current_cycle;
        let epoch = self.dma_epoch;
        let cycles = self.game_pak_cycles(access);
        self.complete_access(access, value, cycles);
        self.reprice_interrupted_game_pak_access(access, fetch_started, cycles, epoch);
        if self.waitcnt & (1 << 14) != 0 {
            self.start_game_pak_prefetch(aligned, opcode_bytes);
        }
        ReadResult { value, cycles: self.current_cycle - started_at }
    }

    fn reprice_interrupted_game_pak_access(
        &mut self,
        mut access: Access,
        started: Cycle,
        cycles: Cycle,
        epoch: u32,
    ) {
        if access.kind != AccessKind::Sequential
            || self.dma_epoch == epoch
            || self.dma_burst_start <= started
            || self.dma_burst_start > started + cycles
        {
            return;
        }
        access.kind = AccessKind::Nonsequential;
        let extra = self.game_pak_cycles(access) - cycles;
        self.advance(extra);
        self.service_pending_dma();
    }

    fn start_game_pak_prefetch(&mut self, address: u32, opcode_bytes: u8) {
        let next = address.wrapping_add(u32::from(opcode_bytes));
        let duty = self.prefetch_duty(next, opcode_bytes);
        self.game_pak_prefetch = GamePakPrefetch {
            active: true,
            head_address: next,
            last_address: next,
            count: 0,
            capacity: if opcode_bytes == 4 { 4 } else { 8 },
            opcode_bytes,
            countdown: duty,
            duty,
        };
        self.duty_valid_until = prefetch_duty_valid_until(next, opcode_bytes);
    }

    /// Discards the buffer. A Game Pak data access (`collision` names its
    /// source) waits one cycle for a halfword read the buffer has in flight:
    /// a CPU access only when that read is in its last cycle, a DMA access
    /// whenever it has begun. The mGBA suite's `ldmia` overflow and DMA
    /// timing rows measure both rules.
    fn stop_game_pak_prefetch(&mut self, collision: Option<AccessSource>) {
        let prefetch = self.game_pak_prefetch;
        if let Some(source) = collision
            && prefetch.active
            && prefetch.countdown != 0
        {
            // An ARM word is two halfword reads of half the duty each.
            let half = if prefetch.opcode_bytes == 4 { prefetch.duty / 2 } else { prefetch.duty };
            let left = if prefetch.countdown > half {
                prefetch.countdown - half
            } else {
                prefetch.countdown
            };
            let waits = if source == AccessSource::Dma { left != half } else { left == 1 };
            if waits {
                self.advance(1);
            }
        }
        self.game_pak_prefetch = GamePakPrefetch::default();
    }

    #[inline(always)]
    fn advance_game_pak_prefetch(&mut self, mut cycles: Cycle) {
        if !self.game_pak_prefetch.active || self.game_pak_prefetch.countdown == 0 {
            return;
        }
        while cycles >= self.game_pak_prefetch.countdown {
            cycles -= self.game_pak_prefetch.countdown;
            self.game_pak_prefetch.count += 1;
            if self.waitcnt & (1 << 14) == 0
                || self.game_pak_prefetch.count >= self.game_pak_prefetch.capacity
            {
                self.game_pak_prefetch.countdown = 0;
                return;
            }
            self.continue_game_pak_prefetch(self.game_pak_prefetch.opcode_bytes);
        }
        self.game_pak_prefetch.countdown -= cycles;
    }

    fn continue_game_pak_prefetch(&mut self, opcode_bytes: u8) {
        let next = self.game_pak_prefetch.last_address.wrapping_add(u32::from(opcode_bytes));
        self.game_pak_prefetch.last_address = next;
        if next >= self.duty_valid_until || self.game_pak_timing_key != self.waitcnt {
            self.game_pak_prefetch.duty = self.prefetch_duty(next, opcode_bytes);
            self.duty_valid_until = prefetch_duty_valid_until(next, opcode_bytes);
        }
        self.game_pak_prefetch.countdown = self.game_pak_prefetch.duty;
    }

    fn prefetch_duty(&mut self, address: u32, opcode_bytes: u8) -> Cycle {
        if self.game_pak_timing_key != self.waitcnt {
            self.refresh_game_pak_timing();
        }
        let aligned = address & !(u32::from(opcode_bytes) - 1);
        let window = ((aligned - GAME_PAK_BASE) / GAME_PAK_WINDOW_SIZE) as usize;
        let sequential = Cycle::from(self.game_pak_s_cycles[window]);
        let first = if aligned & 0x1ffff == 0 {
            Cycle::from(self.game_pak_n_cycles[window])
        } else {
            sequential
        };
        first + if opcode_bytes == 4 { sequential } else { 0 }
    }
}

/// Couple ready serial endpoints in cable order. All peers share one cycle origin.
pub fn synchronize_sio(
    peers: &mut [&mut Bus],
    connection: SioConnection,
) -> Result<bool, SioLinkError> {
    if !(2..=4).contains(&peers.len()) || (connection == SioConnection::Cable2p && peers.len() != 2)
    {
        return Err(SioLinkError::UnsupportedPeerCount);
    }
    let mode = (peers[0].sio_control >> 12) & 3;
    if peers.iter().any(|peer| peer.rcnt & 0x8000 != 0 || (peer.sio_control >> 12) & 3 != mode) {
        return Ok(false);
    }
    match mode {
        0 | 1 => {
            let mut outgoing = [0u32; 4];
            let mut internal = [false; 4];
            for (index, peer) in peers.iter().enumerate() {
                if peer.sio_control & 0x80 == 0 {
                    return Ok(false);
                }
                internal[index] = peer.sio_control & 1 != 0;
                outgoing[index] = if mode == 0 {
                    u32::from(peer.sio_send as u8)
                } else {
                    u32::from(peer.sio_data[0]) | (u32::from(peer.sio_data[1]) << 16)
                };
            }
            let master = match connection {
                SioConnection::Cable2p => {
                    if internal[0] == internal[1] {
                        return if internal[0] {
                            Err(SioLinkError::ClockConflict)
                        } else {
                            Ok(false)
                        };
                    }
                    usize::from(!internal[0])
                }
                SioConnection::CableMulti => {
                    if !internal[0] {
                        if internal[1..peers.len()].contains(&true) {
                            return Err(SioLinkError::ClockConflict);
                        }
                        return Ok(false);
                    }
                    if internal[1..peers.len()].contains(&true) {
                        return Err(SioLinkError::ClockConflict);
                    }
                    0
                }
            };
            let deadline =
                peers[master].sio_completion_cycle.expect("internal SIO clock has a deadline");
            if peers.iter().any(|peer| peer.current_cycle > deadline) {
                return Err(SioLinkError::MissedDeadline);
            }
            match connection {
                SioConnection::Cable2p => {
                    peers[0].sio_receive_value = outgoing[1];
                    peers[1].sio_receive_value = outgoing[0];
                    arm_sio_completion(peers[1 - master], deadline);
                }
                SioConnection::CableMulti => {
                    peers[0].sio_receive_value = 0;
                    for index in 1..peers.len() {
                        peers[index].sio_receive_value = outgoing[index - 1];
                        arm_sio_completion(peers[index], deadline);
                    }
                }
            }
            Ok(true)
        }
        2 => {
            if connection != SioConnection::CableMulti {
                return Err(SioLinkError::UnsupportedMode);
            }
            let parent_started = peers[0].sio_control & 0x80 != 0;
            for (index, peer) in peers.iter_mut().enumerate() {
                peer.sio_control =
                    (peer.sio_control & !0x000c) | 0x0008 | if index == 0 { 0 } else { 0x0004 };
                if index != 0 && !parent_started {
                    peer.sio_control &= !0x80;
                }
            }
            if !parent_started {
                return Ok(false);
            }
            if peers[0].sio_completion_cycle.is_some() {
                return Ok(true);
            }
            let mut received = [0xffff; 4];
            for (index, peer) in peers.iter().enumerate() {
                received[index] = peer.sio_send;
            }
            const DURATIONS: [[Cycle; 3]; 4] = [
                [73_003, 107_680, 142_356],
                [18_251, 26_920, 35_589],
                [12_167, 17_947, 23_726],
                [6_075, 8_973, 11_863],
            ];
            let Some(started_at) = peers[0].sio_start_cycle else { return Ok(false) };
            let deadline =
                started_at + DURATIONS[(peers[0].sio_control & 3) as usize][peers.len() - 2];
            if peers.iter().any(|peer| peer.current_cycle > deadline) {
                return Err(SioLinkError::MissedDeadline);
            }
            for (index, peer) in peers.iter_mut().enumerate() {
                peer.sio_data = [0xffff; 4];
                peer.sio_multiplayer_receive = received;
                peer.sio_multiplayer_id = index as u8;
                peer.sio_control |= 0x80;
                arm_sio_completion(peer, deadline);
            }
            Ok(true)
        }
        3 => {
            if connection != SioConnection::Cable2p {
                return Err(SioLinkError::UnsupportedMode);
            }
            let mut progressed = false;
            for index in 0..2 {
                let (sender, receiver) = if index == 0 {
                    let (left, right) = peers.split_at_mut(1);
                    (&mut *left[0], &mut *right[0])
                } else {
                    let (left, right) = peers.split_at_mut(1);
                    (&mut *right[0], &mut *left[0])
                };
                let receiver_ready = receiver.sio_control & 0x800 != 0
                    && receiver.uart_rx_fifo.count
                        < if receiver.sio_control & 0x100 != 0 { 4 } else { 1 }
                    && receiver.uart_rx_arrival_cycle.is_none();
                sender.uart_cts_ready = receiver_ready;
                let was_active = sender.uart_tx_active.is_some();
                sender.maybe_start_uart_transmit();
                if !was_active && sender.uart_tx_active.is_some() {
                    progressed = true;
                }
                let Some(value) = sender.uart_tx_active else { continue };
                if sender.uart_tx_peer_scheduled {
                    continue;
                }
                let deadline =
                    sender.uart_tx_completion_cycle.expect("active UART transmit has a deadline");
                if receiver.current_cycle > deadline {
                    return Err(SioLinkError::MissedDeadline);
                }
                if receiver.sio_control & 0x800 != 0 {
                    receiver.uart_rx_arrival_value = value;
                    receiver.uart_rx_arrival_cycle = Some(deadline);
                    if receiver.current_cycle == deadline {
                        receiver.complete_uart_receive();
                    }
                }
                sender.uart_tx_peer_scheduled = true;
                progressed = true;
            }
            Ok(progressed)
        }
        _ => unreachable!(),
    }
}

fn arm_sio_completion(peer: &mut Bus, deadline: Cycle) {
    peer.sio_completion_cycle = Some(deadline);
    if peer.current_cycle == deadline {
        peer.complete_sio_transfer();
    }
}

fn prefetch_duty_valid_until(address: u32, opcode_bytes: u8) -> u32 {
    if address & 0x1ffff == 0 {
        address.wrapping_add(u32::from(opcode_bytes))
    } else {
        address.wrapping_add(0x20000) & !0x1ffff
    }
}

/// The nonsequential and sequential halfword cycles of the three Game Pak
/// windows under `waitcnt`.
fn game_pak_timing(waitcnt: u16) -> ([u8; 3], [u8; 3]) {
    let n =
        std::array::from_fn(|window| [5, 4, 3, 9][usize::from((waitcnt >> (2 + window * 3)) & 3)]);
    let s = std::array::from_fn(|window| {
        if waitcnt & (1 << (4 + window * 3)) != 0 { 2 } else { [3, 5, 9][window] }
    });
    (n, s)
}

/// The cycles of each word access to an HLE IRQ stack whose six words
/// start at `stack`, when all of them are in IWRAM or all in EWRAM.
fn hle_stack_word_cycles(stack: u32) -> Option<Cycle> {
    match (stack >> 24, stack.wrapping_add(20) >> 24) {
        (3, 3) => Some(1),
        (2, 2) => Some(ewram_cycles(AccessWidth::Word)),
        _ => None,
    }
}

fn ewram_cycles(width: AccessWidth) -> Cycle {
    if width == AccessWidth::Word { 6 } else { 3 }
}

fn video_cycles(width: AccessWidth) -> Cycle {
    if width == AccessWidth::Word { 2 } else { 1 }
}

/// The clock and PPU position of a bulk DMA, which meets no event.
struct BeatClock {
    cycle: Cycle,
    line: u16,
    dot: u16,
}

impl BeatClock {
    /// [`Self::elapse`] by any number of cycles.
    fn elapse_many(&mut self, cycles: Cycle) {
        let dot = Cycle::from(self.dot) + cycles;
        self.cycle += cycles;
        self.line += (dot / 1232) as u16;
        self.dot = (dot % 1232) as u16;
    }

    #[inline(always)]
    fn elapse(&mut self, cycles: Cycle) {
        self.cycle += cycles;
        self.dot += cycles as u16;
        if self.dot >= 1232 {
            self.dot -= 1232;
            self.line += 1;
        }
    }
}

/// How many beats of `unit_bytes` from `address`, moving by address
/// control `control`, stay in the address's 16 MiB region.
fn dma_beats_in_region(address: u32, unit_bytes: u32, control: u8) -> u32 {
    let offset = address & 0x00ff_ffff;
    match control {
        0 | 3 => (0x00ff_ffff - offset) / unit_bytes + 1,
        1 => offset / unit_bytes + 1,
        _ => u32::MAX,
    }
}

fn adjusted_dma_address(address: u32, unit_bytes: u32, control: u8) -> u32 {
    match control {
        0 | 3 => address.wrapping_add(unit_bytes),
        1 => address.wrapping_sub(unit_bytes),
        2 => address,
        _ => unreachable!(),
    }
}

impl AccessWidth {
    pub(crate) const fn bytes(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
        }
    }
}

/// Reads little-endian bytes of a power-of-two memory, wrapping within it.
///
/// Bus accesses are aligned to their width, so the common case is one load:
/// masking the index to a width-aligned position inside the array lets the
/// compiler prove the bounds. A misaligned index keeps the byte-wise wrap.
#[inline(always)]
pub(crate) fn read_ram<const N: usize>(ram: &[u8; N], index: usize, width: AccessWidth) -> u32 {
    const { assert!(N.is_power_of_two() && N >= 4) };
    let index = index & (N - 1);
    match width {
        AccessWidth::Byte => u32::from(ram[index]),
        AccessWidth::Half if index & 1 == 0 => {
            let at = index & (N - 2);
            u32::from(u16::from_le_bytes([ram[at], ram[at + 1]]))
        }
        AccessWidth::Word if index & 3 == 0 => {
            let at = index & (N - 4);
            u32::from_le_bytes([ram[at], ram[at + 1], ram[at + 2], ram[at + 3]])
        }
        _ => read_ram_wrapping(ram, index, width),
    }
}

#[inline(always)]
pub(crate) fn write_ram<const N: usize>(
    ram: &mut [u8; N],
    index: usize,
    width: AccessWidth,
    value: u32,
) {
    const { assert!(N.is_power_of_two() && N >= 4) };
    let index = index & (N - 1);
    match width {
        AccessWidth::Byte => ram[index] = value as u8,
        AccessWidth::Half if index & 1 == 0 => {
            let at = index & (N - 2);
            let [low, high] = (value as u16).to_le_bytes();
            ram[at] = low;
            ram[at + 1] = high;
        }
        AccessWidth::Word if index & 3 == 0 => {
            let at = index & (N - 4);
            let [b0, b1, b2, b3] = value.to_le_bytes();
            ram[at] = b0;
            ram[at + 1] = b1;
            ram[at + 2] = b2;
            ram[at + 3] = b3;
        }
        _ => write_ram_wrapping(ram, index, width, value),
    }
}

/// The byte-wise reference for [`read_ram`], also its misaligned path.
#[cold]
fn read_ram_wrapping<const N: usize>(ram: &[u8; N], index: usize, width: AccessWidth) -> u32 {
    (0..width.bytes())
        .map(|offset| u32::from(ram[(index + offset) & (N - 1)]) << (offset * 8))
        .fold(0, |a, b| a | b)
}

/// The byte-wise reference for [`write_ram`], also its misaligned path.
#[cold]
fn write_ram_wrapping<const N: usize>(
    ram: &mut [u8; N],
    index: usize,
    width: AccessWidth,
    value: u32,
) {
    for offset in 0..width.bytes() {
        ram[(index + offset) & (N - 1)] = (value >> (offset * 8)) as u8;
    }
}

fn boxed_array<T: Clone, const N: usize>(value: T) -> Box<[T; N]> {
    vec![value; N].into_boxed_slice().try_into().ok().expect("fixed-size heap array")
}

fn vram_index(address: u32, dispcnt: u16) -> Option<usize> {
    let mut offset = address.wrapping_sub(VRAM_BASE) & 0x1ffff;
    if offset >= 0x18000 {
        offset -= 0x8000;
        if dispcnt & 7 >= 3 && offset < 0x14000 {
            return None;
        }
    }
    Some(offset as usize)
}

fn vram_obj_boundary(dispcnt: u16) -> usize {
    if dispcnt & 7 >= 3 { 0x14000 } else { 0x10000 }
}

/// A halfword on a 16-bit bus, as the 32-bit data bus carries it: on both
/// halves.
#[inline(always)]
pub(crate) fn half_on_both_lanes(half: u32) -> u32 {
    half.wrapping_mul(0x0001_0001)
}

/// The data bus after IWRAM, which drives only the addressed half, presents
/// `half` at `address`: the other half keeps `latch`'s.
#[inline(always)]
pub(crate) fn iwram_half_latch(latch: u32, address: u32, half: u32) -> u32 {
    if address & 2 == 0 { (latch & 0xffff_0000) | half } else { (latch & 0xffff) | (half << 16) }
}

pub fn is_byte_wide_save_address(address: u32) -> bool {
    matches!(address >> 24, 0x0e | 0x0f)
}

pub fn is_arm_iwram_slice_fetch(address: u32) -> bool {
    address & 3 == 0 && address >> 24 == 3
}

fn encode_scroll_segments(
    bus: &Bus,
    sink: &mut evenfall_state::Sink<'_>,
) -> evenfall_state::Result<()> {
    let mut canonical = bus.scroll_segments;
    for (segments, &count) in canonical.iter_mut().zip(&bus.scroll_segment_counts) {
        segments[usize::from(count)..].fill(ScrollSegment::default());
    }
    canonical.encode(sink)
}

fn encode_backdrop_segments(
    bus: &Bus,
    sink: &mut evenfall_state::Sink<'_>,
) -> evenfall_state::Result<()> {
    let mut canonical = bus.backdrop_segments;
    canonical[usize::from(bus.backdrop_segment_count)..].fill(BackdropSegment::default());
    canonical.encode(sink)
}

fn encode_win_segments(
    bus: &Bus,
    sink: &mut evenfall_state::Sink<'_>,
) -> evenfall_state::Result<()> {
    let mut canonical = bus.win_segments;
    canonical[usize::from(bus.win_segment_count)..].fill(WindowSegment::default());
    canonical.encode(sink)
}

fn encode_blend_segments(
    bus: &Bus,
    sink: &mut evenfall_state::Sink<'_>,
) -> evenfall_state::Result<()> {
    let mut canonical = bus.blend_segments;
    canonical[usize::from(bus.blend_segment_count)..].fill(BlendSegment::default());
    canonical.encode(sink)
}

fn encode_affine_segments(
    bus: &Bus,
    sink: &mut evenfall_state::Sink<'_>,
) -> evenfall_state::Result<()> {
    let mut canonical = bus.affine_segments;
    for (segments, &count) in canonical.iter_mut().zip(&bus.affine_segment_counts) {
        segments[usize::from(count)..].fill(AffineSegment::default());
    }
    canonical.encode(sink)
}

fn encode_mosaic_segments(
    bus: &Bus,
    sink: &mut evenfall_state::Sink<'_>,
) -> evenfall_state::Result<()> {
    let mut canonical = bus.mosaic_segments;
    canonical[usize::from(bus.mosaic_segment_count)..].fill(MosaicSegment::default());
    canonical.encode(sink)
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Whether DMA may run beats in bulk; the per-beat loop is its
        /// reference.
        pub(super) static BULK_DMA: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
        /// Beats DMA ran in bulk on this thread, and of them to VRAM a page
        /// at a time.
        pub(super) static BULK_BEATS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        pub(super) static VRAM_RUN_BEATS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        /// Whether the HLE IRQ trampoline may run fused; the per-access
        /// sequence is its reference.
        pub(super) static FUSED_HLE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
        /// Fused trampoline sequences run and declined on this thread.
        pub(super) static FUSED_HLE_TAKEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        pub(super) static FUSED_HLE_DECLINED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        /// The bus after each HLE IRQ entry's and return's accesses.
        pub(super) static HLE_TRACE: std::cell::RefCell<Vec<String>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn dma_half(bus: &mut Bus, address: u32, value: u16) -> Cycle {
        bus.write(
            Access {
                address,
                width: AccessWidth::Half,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            },
            u32::from(value),
        )
        .cycles
    }

    #[test]
    fn shared_media_constructor_keeps_rom_and_bios_allocations() {
        let mut image = vec![0u8; 256];
        image[0xd0..0xd2].copy_from_slice(&0x4321u16.to_le_bytes());
        let upload_bytes = image.as_ptr();
        let rom = Media::from_vec(image);
        let bios = Media::from_slice(&vec![0x5a; BIOS_SIZE]);
        let bus = Box::new(Bus::init_with_media(rom.clone(), bios.clone(), save::Type::None, None));
        assert_eq!(bus.rom().as_ptr(), upload_bytes);
        assert_eq!(bus.bios().as_ptr(), bios.as_ptr());
        assert_eq!(
            bus.instruction_fetch_region(0x0800_00d0).bytes,
            Some(upload_bytes.wrapping_add(0xca))
        );
        assert_eq!(bus.read_game_pak_rom(0x0800_00d0, AccessWidth::Half), 0x4321);
        assert_eq!(bus.read_bios(0, AccessWidth::Byte), 0x5a);
        drop(bus);
        assert_eq!(rom.as_ptr(), upload_bytes);
    }

    /// An immediate DMA3 starts two cycles after its enabling write, copies
    /// halfwords, clears its enable bit and requests an IRQ.
    #[test]
    fn dma3_immediate_starts_two_cycles_after_enabling_write() {
        let mut bus = Bus::init(&[]);
        dma_half(&mut bus, 0x0200_0000, 0x1122);
        dma_half(&mut bus, 0x0200_0002, 0x3344);
        for (address, value) in [
            (0x0400_00d4, 0),
            (0x0400_00d6, 0x0200),
            (0x0400_00d8, 0),
            (0x0400_00da, 0x0300),
            (0x0400_00dc, 2),
        ] {
            dma_half(&mut bus, address, value);
        }
        assert_eq!(dma_half(&mut bus, 0x0400_00de, 0xc000), 1);
        assert_eq!(&bus.iwram[..4], &[0; 4]);
        bus.elapse(1);
        assert_eq!(&bus.iwram[..4], &[0; 4]);
        let before_start = bus.now();
        bus.elapse(1);
        assert_eq!(bus.now() - before_start, 11);
        assert_eq!(&bus.iwram[..4], &[0x22, 0x11, 0x44, 0x33]);
        assert!(!bus.dma_channels[3].enabled());
        assert_eq!(bus.irq.interrupt_request_flags & (1 << 11), 1 << 11);
    }

    /// A VBlank DMA3 uploads a solid tile while the VBlank IRQ stays disabled.
    #[test]
    fn vblank_dma_starts_without_vblank_irq_enable() {
        let mut bus = Bus::init(&[]);
        bus.ppu_timing.line = 159;
        bus.ppu_timing.cycle_in_line = 1231;
        bus.current_cycle = 159 * 1232 + 1231;
        bus.apu_cycle = bus.current_cycle;
        bus.timer_cycle = bus.current_cycle;
        bus.ewram[..2].copy_from_slice(&0x3333_u16.to_le_bytes());
        bus.dma_channels[3] = DmaChannel {
            source: EWRAM_BASE,
            destination: VRAM_BASE + 0xa0,
            word_count: 1,
            control: 0x9000,
            latched_source: EWRAM_BASE,
            latched_destination: VRAM_BASE + 0xa0,
            latched_word_count: 1,
        };

        bus.elapse(1);
        assert_eq!(bus.ppu_timing.line, 160);
        assert_eq!(bus.dma_start_cycles[3], Some(bus.now() + 3));
        bus.elapse(2);
        assert_eq!(&bus.vram[0xa0..0xa2], &[0, 0]);
        bus.elapse(1);
        assert_eq!(&bus.vram[0xa0..0xa2], &[0x33, 0x33]);
        assert!(!bus.dma_channels[3].enabled());
    }

    /// DMA started at a batch deadline must not close the caller's deferred-device batch.
    #[test]
    fn dma_at_lcd_boundary_preserves_deferred_batch() {
        let mut bus = Bus::init(&[]);
        bus.ppu_timing.line = 159;
        bus.ppu_timing.cycle_in_line = 1231;
        bus.current_cycle = 159 * 1232 + 1231;
        bus.apu_cycle = bus.current_cycle;
        bus.timer_cycle = bus.current_cycle;
        bus.ewram[..2].copy_from_slice(&0x3333_u16.to_le_bytes());
        bus.dma_channels[3] = DmaChannel {
            source: EWRAM_BASE,
            destination: VRAM_BASE + 0xa0,
            word_count: 1,
            control: 0x9000,
            latched_source: EWRAM_BASE,
            latched_destination: VRAM_BASE + 0xa0,
            latched_word_count: 1,
        };
        bus.begin_device_batch();
        bus.elapse(4);
        assert_eq!(&bus.vram[0xa0..0xa2], &[0x33, 0x33]);
        assert!(bus.batch_active);
        assert!(bus.devices_deferred);
        assert!(bus.apu_cycle < bus.current_cycle);
        bus.end_device_batch();
        assert_eq!(bus.apu_cycle, bus.current_cycle);
    }

    /// A DMA from source zero reuses the shared transfer latch without
    /// advancing the source.
    #[test]
    fn dma_source_zero_reuses_shared_transfer_latch() {
        let mut bus = Bus::init(&[]);
        dma_half(&mut bus, 0x0300_0100, 0xbeef);
        for (address, value) in [
            (0x0400_00bc, 0x0100),
            (0x0400_00be, 0x0300),
            (0x0400_00c0, 0x0200),
            (0x0400_00c2, 0x0300),
            (0x0400_00c4, 1),
            (0x0400_00c6, 0x8000),
        ] {
            dma_half(&mut bus, address, value);
        }
        bus.elapse(3);
        assert_eq!(bus.dma_transfer_latch, 0xbeef_beef);
        for (address, value) in [
            (0x0400_00d4, 0),
            (0x0400_00d6, 0),
            (0x0400_00d8, 0x0300),
            (0x0400_00da, 0x0300),
            (0x0400_00dc, 2),
            (0x0400_00de, 0x8400),
        ] {
            dma_half(&mut bus, address, value);
        }
        bus.elapse(3);
        assert_eq!(&bus.iwram[0x300..0x308], &[0xef, 0xbe, 0xef, 0xbe, 0xef, 0xbe, 0xef, 0xbe]);
        assert_eq!(bus.dma_channels[3].latched_source, 0);
    }

    /// Normal-mode SIO waits for both peers and rejects one that arrives past
    /// the deadline.
    #[test]
    fn sio_peer_arriving_after_master_deadline_is_rejected() {
        let mut master = Bus::init(&[]);
        let mut slave = Bus::init(&[]);
        dma_half(&mut master, MMIO_BASE + 0x134, 0);
        dma_half(&mut slave, MMIO_BASE + 0x134, 0);
        dma_half(&mut master, MMIO_BASE + 0x128, 0x0083);
        assert_eq!(
            synchronize_sio(&mut [&mut master, &mut slave], SioConnection::Cable2p),
            Ok(false)
        );
        dma_half(&mut slave, MMIO_BASE + 0x128, 0x0080);
        slave.elapse(65);
        assert_eq!(
            synchronize_sio(&mut [&mut master, &mut slave], SioConnection::Cable2p),
            Err(SioLinkError::MissedDeadline)
        );
    }

    /// Four-player SIO exchanges every multiplayer word at the parent's baud
    /// deadline.
    #[test]
    fn multiplayer_sio_delivers_all_four_words_at_parent_deadline() {
        let mut buses = std::array::from_fn::<Bus, 4, _>(|_| Bus::init(&[]));
        let sent = [0x0123, 0x4567, 0x89ab, 0xcdef];
        for (index, bus) in buses.iter_mut().enumerate() {
            dma_half(bus, MMIO_BASE + 0x134, 0);
            dma_half(bus, MMIO_BASE + 0x128, 0x2003);
            dma_half(bus, MMIO_BASE + 0x12a, sent[index]);
            dma_half(bus, MMIO_BASE + 0x128, if index == 0 { 0x6083 } else { 0x6003 });
        }
        let [a, b, c, d] = &mut buses;
        assert_eq!(synchronize_sio(&mut [a, b, c, d], SioConnection::CableMulti), Ok(true));
        for bus in &mut buses {
            assert_eq!(bus.sio_completion_cycle, Some(bus.now() + 11_863));
            bus.elapse(11_862);
            assert_ne!(bus.sio_control & 0x80, 0);
            bus.elapse(1);
        }
        for (index, bus) in buses.iter().enumerate() {
            assert_eq!(bus.sio_data, sent);
            assert_eq!(bus.sio_control & 0x80, 0);
            assert_eq!(bus.sio_control & 0x30, (index as u16) << 4);
            assert_ne!(bus.irq.interrupt_request_flags & (1 << 7), 0);
        }
    }

    /// UART CTS holds the sender until the peer receiver has capacity.
    #[test]
    fn uart_cts_waits_for_receiver_capacity() {
        let mut sender = Bus::init(&[]);
        let mut receiver = Bus::init(&[]);
        dma_half(&mut sender, MMIO_BASE + 0x134, 0);
        dma_half(&mut receiver, MMIO_BASE + 0x134, 0);
        dma_half(&mut sender, MMIO_BASE + 0x128, 0x3487);
        dma_half(&mut receiver, MMIO_BASE + 0x128, 0x3083);
        dma_half(&mut sender, MMIO_BASE + 0x12a, 0x3c);
        assert_eq!(sender.uart_tx_completion_cycle, None);
        assert_eq!(
            synchronize_sio(&mut [&mut sender, &mut receiver], SioConnection::Cable2p),
            Ok(false)
        );
        dma_half(&mut receiver, MMIO_BASE + 0x128, 0x3883);
        assert_eq!(
            synchronize_sio(&mut [&mut sender, &mut receiver], SioConnection::Cable2p),
            Ok(true)
        );
        assert!(sender.uart_tx_completion_cycle.is_some());
    }

    #[test]
    fn bios_hard_reset_preserves_host_audio_suppression() {
        let mut bus = Bus::init(&[]);
        let mut samples = [StereoSample::default(); 16];
        bus.set_audio_output(false);
        bus.hard_reset();
        bus.elapse_cpu_internal(1024);
        assert_eq!(bus.drain_audio_samples(&mut samples), 0);
        bus.set_audio_output(true);
        bus.elapse_cpu_internal(1024);
        assert_eq!(bus.drain_audio_samples(&mut samples), 2);
    }

    /// Deferred device clocks catch up, without any event due, before a
    /// register reset or a hard reset changes them.
    #[test]
    fn deferred_clock_materializes_before_register_and_hard_reset() {
        let mut bus = Bus::init(&[]);
        bus.timers[0].control = 0x80;
        bus.begin_device_batch();
        bus.elapse(7);
        bus.register_ram_reset(0);
        assert_eq!(bus.now(), 7);
        assert_eq!(bus.ppu_timing.cycle_in_line, 7);
        assert_eq!(bus.timers[0].counter, 7);
        bus.elapse(5);
        bus.hard_reset();
        assert_eq!(bus.now(), 12);
        assert_eq!(bus.ppu_timing.cycle_in_line, 0);
        bus.elapse(1);
        assert_eq!(bus.now(), 13);
        assert_eq!(bus.ppu_timing.cycle_in_line, 1);
        assert_eq!(bus.timers[0].counter, 0);
        bus.end_device_batch();
    }

    /// The event horizon defers APU and timer clocks across LCD events.
    /// HALT still delivers the visible HBlank edges at their exact cycles.
    #[test]
    fn halted_lcd_events_do_not_materialize_unobserved_devices() {
        let mut batched = Bus::init(&[]);
        let mut eager = Bus::init(&[]);
        batched.begin_device_batch();
        batched.enter_halt();
        eager.enter_halt();
        let deadline = 1232 * 2;
        assert!(!batched.idle_halt_until(deadline));
        assert!(!eager.idle_halt_until(deadline));
        assert_eq!(batched.now(), deadline);
        assert_eq!(batched.apu_cycle, 0, "APU has no observer before batch end");
        assert_eq!(batched.timer_cycle, 0, "inactive timers have no event");
        batched.end_device_batch();
        assert_eq!(batched.apu_cycle, deadline);
        assert_eq!(batched.timer_cycle, deadline);
        assert_eq!(batched.ppu_timing, eager.ppu_timing);
        assert_eq!(batched.apu, eager.apu);
        for (actual, expected) in batched.timers.iter().zip(&eager.timers) {
            assert_eq!(actual.read_counter(), expected.read_counter());
            assert_eq!(actual.prescaler_phase, expected.prescaler_phase);
        }
    }

    /// Timer overflows delegated to the sound FIFOs match eager delivery
    /// through refills and schedule changes.
    #[test]
    fn fifo_only_timer_overflows_match_eager_refill_and_irq_schedule() {
        let mut eager = Bus::init(&[]);
        let mut batched = Bus::init(&[]);
        for bus in [&mut eager, &mut batched] {
            dma_half(bus, MMIO_BASE + 0x84, 1 << 7);
            dma_half(bus, MMIO_BASE + 0x82, 1 << 8);
            for word in 0..7 {
                bus.write(
                    Access {
                        address: MMIO_BASE + 0xa0,
                        width: AccessWidth::Word,
                        kind: AccessKind::Nonsequential,
                        source: AccessSource::CpuData,
                    },
                    word * 0x0101_0101,
                );
            }
            dma_half(bus, MMIO_BASE + 0x100, 0xfff0);
            dma_half(bus, MMIO_BASE + 0x102, 0x0080);
        }
        batched.begin_device_batch();
        assert!(batched.fifo_clock.delegated());
        eager.elapse(64); // Four overflows, but no refill request yet.
        batched.elapse(64);
        assert!(batched.timer_cycle < batched.now());
        assert!(batched.apu_cycle < batched.now());
        assert_ne!(batched.apu.fifo_a, eager.apu.fifo_a);
        // The first refill request, a FIFO append and then an IRQ-enabled
        // timer each change the schedule without changing the guest result.
        eager.elapse(260);
        batched.elapse(260);
        for bus in [&mut eager, &mut batched] {
            bus.write(
                Access {
                    address: MMIO_BASE + 0xa0,
                    width: AccessWidth::Word,
                    kind: AccessKind::Nonsequential,
                    source: AccessSource::CpuData,
                },
                0x1122_3344,
            );
            dma_half(bus, MMIO_BASE + 0x102, 0x00c0);
            bus.elapse(80);
        }
        batched.end_device_batch();
        assert_eq!(batched.now(), eager.now());
        assert_eq!(batched.irq.interrupt_request_flags, eager.irq.interrupt_request_flags);
        for (actual, expected) in batched.timers.iter().zip(&eager.timers) {
            assert_eq!(actual.reload, expected.reload);
            assert_eq!(actual.counter, expected.counter);
            assert_eq!(actual.control, expected.control);
            assert_eq!(actual.prescaler_phase, expected.prescaler_phase);
        }
        assert_eq!(batched.apu, eager.apu);
    }

    /// FIFO ring-buffer unit check; full UART integration belongs to the serial bus tests.
    #[test]
    fn uart_fifo_capacity_and_wraparound() {
        let mut fifo = UartFifo::default();
        assert!(fifo.push(0x23, 1));
        assert!(!fifo.push(0x99, 1));
        assert_eq!(fifo.pop(), Some(0x23));
        assert_eq!(fifo.pop(), None);
        for value in [0x41, 0x52, 0x63, 0x74] {
            assert!(fifo.push(value, 4));
        }
        assert!(!fifo.push(0x88, 4));
        assert_eq!(fifo.pop(), Some(0x41));
        assert_eq!(fifo.pop(), Some(0x52));
        assert!(fifo.push(0x85, 4));
        assert!(fifo.push(0x96, 4));
        assert_eq!(
            [fifo.pop(), fifo.pop(), fifo.pop(), fifo.pop()],
            [Some(0x63), Some(0x74), Some(0x85), Some(0x96)]
        );
        fifo.clear();
        assert_eq!(fifo.pop(), None);
        assert!(fifo.push(0xa7, 1));
        assert_eq!(fifo.pop(), Some(0xa7));
    }

    /// RAM accesses preserve byte order, unaligned mirror wrapping and bus timing.
    #[test]
    fn ram_mirror_crossing_preserves_little_endian_byte_order() {
        let mut ram = [0u8; 8];
        write_ram(&mut ram, 7, AccessWidth::Word, 0xddcc_bbaa);
        assert_eq!(ram, [0xbb, 0xcc, 0xdd, 0, 0, 0, 0, 0xaa]);
        assert_eq!(read_ram(&ram, 7, AccessWidth::Word), 0xddcc_bbaa);
        assert_eq!(read_ram(&ram, 1, AccessWidth::Half), 0xddcc);
        assert_eq!(read_ram(&ram, 0, AccessWidth::Byte), 0xbb);
    }

    /// The single-load RAM paths match the byte-wise reference at every index,
    /// width and alignment, including indexes past the end of the memory.
    #[test]
    fn ram_fast_paths_match_the_byte_wise_reference() {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut fast = [0u8; 64];
        for byte in &mut fast {
            *byte = next() as u8;
        }
        let mut reference = fast;
        for _ in 0..20_000 {
            let index = (next() % 200) as usize;
            let width =
                [AccessWidth::Byte, AccessWidth::Half, AccessWidth::Word][next() as usize % 3];
            assert_eq!(
                read_ram(&fast, index, width),
                read_ram_wrapping(&reference, index & 63, width),
                "read index {index} width {width:?}"
            );
            let value = next();
            write_ram(&mut fast, index, width, value);
            write_ram_wrapping(&mut reference, index & 63, width, value);
            assert_eq!(fast, reference, "write index {index} width {width:?}");
        }
    }

    /// Bitmap modes leave the lower half of mirrored VRAM unmapped.
    #[test]
    fn bitmap_modes_leave_lower_mirrored_vram_unmapped() {
        assert_eq!(vram_index(0x0601_8000, 0), Some(0x1_0000));
        assert_eq!(vram_index(0x0601_8000, 3), None);
        assert_eq!(vram_index(0x0601_c000, 3), Some(0x1_4000));
        assert_eq!(vram_index(0x0603_c000, 3), Some(0x1_4000));
        assert_eq!(vram_obj_boundary(2), 0x1_0000);
        assert_eq!(vram_obj_boundary(4), 0x1_4000);
    }

    #[test]
    fn game_pak_prefetch_batching_matches_single_cycle_advancement_through_capacity() {
        let mut batched = Bus::init(&[0; 64]);
        let mut scalar = Bus::init(&[0; 64]);
        for bus in [&mut batched, &mut scalar] {
            bus.waitcnt = 1 << 14;
            bus.fetch_kind = AccessKind::Nonsequential;
            bus.fetch_instruction(GAME_PAK_BASE, AccessWidth::Half);
        }
        batched.elapse(24);
        for _ in 0..24 {
            scalar.elapse(1);
        }
        assert_eq!(scalar.now(), batched.now());
        assert_eq!(scalar.game_pak_prefetch.active, batched.game_pak_prefetch.active);
        assert_eq!(scalar.game_pak_prefetch.head_address, batched.game_pak_prefetch.head_address);
        assert_eq!(scalar.game_pak_prefetch.last_address, batched.game_pak_prefetch.last_address);
        assert_eq!(scalar.game_pak_prefetch.countdown, batched.game_pak_prefetch.countdown);
        assert_eq!(scalar.game_pak_prefetch.count, batched.game_pak_prefetch.count);
        assert_eq!(batched.game_pak_prefetch.count, 8);
        assert_eq!(batched.game_pak_prefetch.countdown, 0);
    }

    /// A fused Thumb Game Pak refill matches two ordinary fetches.
    #[test]
    fn fused_thumb_refill_matches_two_scalar_fetches() {
        let rom = std::array::from_fn::<u8, 256, _>(|i| (i as u8).wrapping_mul(53));
        for wait in [0x4000, 0x4014, 0x43a8] {
            for target in [0x0800_0020, 0x0a00_0042, 0x0c00_00d0] {
                let mut fused = Bus::init(&rom);
                let mut scalar = Bus::init(&rom);
                for bus in [&mut fused, &mut scalar] {
                    bus.write_waitcnt(wait);
                    bus.refresh_game_pak_timing();
                    bus.begin_device_batch();
                }
                let mut fused_region = InstructionFetchRegion::default();
                let actual = fused
                    .fused_thumb_game_pak_refill(&mut fused_region, target)
                    .expect("ordinary ROM miss should admit a fused refill");
                let mut scalar_region = InstructionFetchRegion::default();
                scalar.fetch_kind = AccessKind::Nonsequential;
                let first = scalar.fetch_instruction_in_region(
                    &mut scalar_region,
                    target,
                    AccessWidth::Half,
                );
                scalar.fetch_kind = AccessKind::Sequential;
                let second = scalar.fetch_instruction_in_region(
                    &mut scalar_region,
                    target + 2,
                    AccessWidth::Half,
                );
                assert_eq!(actual, [first, second], "wait {wait:x}, target {target:x}");
                assert_eq!(fused.now(), scalar.now(), "wait {wait:x}, target {target:x}");
                assert_eq!(fused.open_bus_value, scalar.open_bus_value);
                assert_eq!(fused.fetch_kind, scalar.fetch_kind);
                assert_eq!(
                    (
                        fused.game_pak_prefetch.head_address,
                        fused.game_pak_prefetch.last_address,
                        fused.game_pak_prefetch.count,
                        fused.game_pak_prefetch.countdown,
                        fused.game_pak_prefetch.duty,
                        fused.duty_valid_until
                    ),
                    (
                        scalar.game_pak_prefetch.head_address,
                        scalar.game_pak_prefetch.last_address,
                        scalar.game_pak_prefetch.count,
                        scalar.game_pak_prefetch.countdown,
                        scalar.game_pak_prefetch.duty,
                        scalar.duty_valid_until
                    ),
                    "wait {wait:x}, target {target:x}",
                );
                fused.end_device_batch();
                scalar.end_device_batch();
            }
        }
    }

    /// A straddling GPIO fetch or a refill reaching an event must fall back.
    #[test]
    fn fused_thumb_refill_rejects_gpio_aperture_and_event_boundary() {
        let mut bus = Bus::init(&[0; 256]);
        bus.write_waitcnt(0x4000);
        bus.refresh_game_pak_timing();
        bus.begin_device_batch();
        let mut region = InstructionFetchRegion::default();
        assert_eq!(bus.fused_thumb_game_pak_refill(&mut region, 0x0800_00c2), None);
        assert_eq!(region.region_class, InstructionFetchClass::Other);
        bus.batch_deadline = bus.now() + 6;
        let before = bus.now();
        assert_eq!(bus.fused_thumb_game_pak_refill(&mut region, 0x0800_00d0), None);
        assert_eq!(bus.now(), before);
        assert_eq!(bus.game_pak_prefetch.count, 0);
        bus.end_device_batch();
    }

    #[test]
    fn cached_irq_line_follows_ie_if_ime_writes_and_keypad_edges() {
        let mut bus = Bus::init(&[]);
        let half = |address| Access {
            address,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        };
        // A request that rises reaches the CPU IRQ_LATENCY cycles after the
        // cycle it rose in, and not one cycle sooner.
        let arrives = |bus: &mut Bus| {
            assert!(!bus.irq.irq_line);
            assert_eq!(bus.irq.irq_line_rises_at, Some(bus.now() + IRQ_LATENCY));
            bus.elapse(IRQ_LATENCY - 1);
            assert!(!bus.irq.irq_line);
            bus.elapse(1);
            assert!(bus.irq.irq_line);
            assert_eq!(bus.irq.irq_line_rises_at, None);
        };
        bus.write(half(MMIO_BASE + 0x132), 0x4001);
        bus.write(half(MMIO_BASE + 0x200), 1 << 12);
        bus.write(half(MMIO_BASE + 0x208), 1);
        assert!(!bus.irq.irq_line);
        bus.set_keys(KeyState { a: true, ..KeyState::default() });
        arrives(&mut bus);
        let byte = Access {
            address: MMIO_BASE + 0x203,
            width: AccessWidth::Byte,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        };
        // A request that falls reaches the CPU at once.
        bus.write(byte, 1 << 4);
        assert!(!bus.irq.irq_line);
        bus.set_keys(KeyState::default());
        bus.set_keys(KeyState { a: true, ..KeyState::default() });
        arrives(&mut bus);
        bus.write(half(MMIO_BASE + 0x200), 0);
        assert!(!bus.irq.irq_line);
        bus.write(half(MMIO_BASE + 0x200), 1 << 12);
        arrives(&mut bus);
        bus.write(half(MMIO_BASE + 0x208), 0);
        assert!(!bus.irq.irq_line);
        bus.write(half(MMIO_BASE + 0x208), 1);
        arrives(&mut bus);
        bus.write(half(MMIO_BASE + 0x202), 1 << 12);
        assert!(!bus.irq.irq_line);
        // A request that falls before it arrives never reaches the CPU.
        bus.set_keys(KeyState::default());
        bus.set_keys(KeyState { a: true, ..KeyState::default() });
        bus.elapse(1);
        bus.write(half(MMIO_BASE + 0x202), 1 << 12);
        assert_eq!(bus.irq.irq_line_rises_at, None);
        bus.elapse(IRQ_LATENCY);
        assert!(!bus.irq.irq_line);
    }

    #[test]
    fn stable_bus_state_retains_only_strictly_future_explicit_deadlines() {
        let mut bus = Bus::init(&[]);
        assert!(bus.validate_state().is_ok());
        bus.sio_completion_cycle = Some(bus.current_cycle);
        assert!(bus.validate_state().is_err());
        bus.sio_completion_cycle = None;
        bus.dma_start_cycles[0] = Some(bus.current_cycle);
        assert!(bus.validate_state().is_err());
        bus.dma_start_cycles[0] = Some(bus.current_cycle + 1);
        assert!(bus.validate_state().is_ok());
    }

    #[test]
    fn normal_mode_internal_clock_sio_transfers_complete_at_bit_clock_deadline() {
        for (control, cycles, normal32) in [(0x4083, 8 * 8, false), (0x5081, 32 * 64, true)] {
            let mut bus = Bus::init(&[]);
            let half = |address| Access {
                address,
                width: AccessWidth::Half,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            };
            bus.write(half(MMIO_BASE + 0x134), 0);
            bus.write(half(MMIO_BASE + 0x128), control & !0x80);
            if normal32 {
                bus.write(
                    Access {
                        address: MMIO_BASE + 0x120,
                        width: AccessWidth::Word,
                        kind: AccessKind::Nonsequential,
                        source: AccessSource::CpuData,
                    },
                    0x1234_5678,
                );
            } else {
                bus.write(half(MMIO_BASE + 0x12a), 0x1200);
            }
            bus.write(half(MMIO_BASE + 0x128), control);
            bus.elapse(cycles - 2);
            assert_ne!(bus.read(half(MMIO_BASE + 0x128)).value & 0x80, 0);
            assert_eq!(bus.irq.interrupt_request_flags & (1 << 7), 0);
            bus.elapse(1);
            assert_eq!(bus.read(half(MMIO_BASE + 0x128)).value & 0x80, 0);
            assert_eq!(bus.irq.interrupt_request_flags & (1 << 7), 1 << 7);
            if normal32 {
                assert_eq!(
                    bus.read(Access {
                        address: MMIO_BASE + 0x120,
                        width: AccessWidth::Word,
                        kind: AccessKind::Nonsequential,
                        source: AccessSource::CpuData
                    })
                    .value,
                    u32::MAX
                );
            } else {
                assert_eq!(bus.read(half(MMIO_BASE + 0x12a)).value, 0x12ff);
            }
        }
    }

    #[test]
    fn joy_bus_status_and_reset_commands_return_gba_type_and_raise_reset_irq() {
        let mut bus = Bus::init(&[]);
        assert_eq!(
            bus.schedule_joy_bus(JoyBusCommand::Status, bus.now()),
            Err(JoyBusError::NotInJoyBusMode)
        );
        let half = |address| Access {
            address,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        };
        bus.write(half(MMIO_BASE + 0x134), 0xc000);
        bus.write(half(MMIO_BASE + 0x140), 0x40);
        bus.schedule_joy_bus(JoyBusCommand::Status, bus.now()).unwrap();
        let status = bus.take_joy_bus_response().unwrap();
        assert_eq!(&status.data[..usize::from(status.len)], &[0, 4, 0]);
        assert_eq!(bus.irq.interrupt_request_flags & (1 << 7), 0);
        bus.write(half(MMIO_BASE + 0x200), 1 << 7);
        let deadline = bus.now() + 37;
        bus.schedule_joy_bus(JoyBusCommand::Reset, deadline).unwrap();
        assert_eq!(bus.take_joy_bus_response(), None);
        bus.enter_halt();
        assert!(bus.idle_halt_until(deadline));
        assert_eq!(bus.now(), deadline);
        assert_eq!(bus.take_joy_bus_response().unwrap().data[..3], [0, 4, 0]);
        assert_eq!(bus.read(half(MMIO_BASE + 0x140)).value, 0x41);
        assert_eq!(bus.irq.interrupt_request_flags & (1 << 7), 1 << 7);
    }

    /// Timed key changes apply at their exact cycle, before serial and LCD events.
    #[test]
    fn timed_key_changes_apply_at_their_exact_cycle() {
        let mut bus = Bus::init(&[]);
        bus.keypad_control = 0x4001;
        let inputs = [TimedInput {
            at: 73,
            change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() }),
        }];
        bus.begin_external_inputs(&inputs);
        bus.begin_device_batch();
        bus.elapse(72);
        assert_eq!(bus.irq.interrupt_request_flags & (1 << 12), 0);
        bus.elapse(1);
        assert_eq!(bus.irq.interrupt_request_flags & (1 << 12), 1 << 12);
        assert_eq!(bus.external_inputs_consumed(), 1);
        bus.end_device_batch();
        bus.end_external_inputs();
    }

    /// VRAM writes during one visible line past the point where its history
    /// is full record exactly what snapshotting and recording every write
    /// records, in bitmap and tiled modes, with writes that change one byte,
    /// both or none and writes into the bitmap-mode hole.
    #[test]
    fn vram_writes_skip_snapshots_only_when_history_is_full() {
        let mut seed = 0x0bad_5eed_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut skipped = 0;
        for trial in 0..200 {
            let (mut fast, mut reference) = (Bus::init(&[]), Bus::init(&[]));
            for bus in [&mut fast, &mut reference] {
                dma_half(bus, 0x0400_0000, [3, 4, 0][trial % 3]);
                bus.elapse(5 * 1232 + 40);
            }
            for _ in 0..60 {
                let address = 0x0600_0000 + (next() & 0x1_fffe);
                let width = [AccessWidth::Byte, AccessWidth::Half, AccessWidth::Word]
                    [(next() % 3) as usize];
                let value = if next() & 1 == 0 { 0 } else { next() };
                skipped += usize::from(fast.vram_history_full());
                fast.write_vram_at_current_dot(address, width, value);
                let snapshot = reference.snapshot_vram_write(address, width);
                reference.write_vram(address, width, value);
                reference.record_vram_write(snapshot);
                let history = |bus: &Bus| {
                    let count = usize::from(bus.backdrop_segment_count);
                    let segments: Vec<_> = bus.backdrop_segments[..count]
                        .iter()
                        .map(|segment| (segment.start_x, segment.color))
                        .collect();
                    (bus.backdrop_segment_line, segments)
                };
                assert_eq!(history(&fast), history(&reference), "trial {trial}");
                assert!(fast.vram[..] == reference.vram[..], "trial {trial}");
            }
        }
        assert!(skipped > 2_000, "only {skipped} writes found the history full");
    }

    /// One DMA through channel 3 with and without bulk beats, from `lead`
    /// cycles into the frame, over RAM filled from `fill` and VRAM filled
    /// by `vram`: the scanline memo's marks when it ends, and after the
    /// following lines every durable state byte, the raster history and the
    /// pixels, agree. Returns the VRAM it left.
    #[allow(clippy::too_many_arguments)]
    fn check_bulk_dma(
        context: &str,
        rom: &[u8],
        source: u32,
        destination: u32,
        count: u32,
        control: u16,
        dispcnt: u16,
        lead: Cycle,
        fill: u32,
        vram: impl Fn(usize) -> u8,
    ) -> Vec<u8> {
        use evenfall_state::Codec as _;
        let run = |bulk: bool| {
            BULK_DMA.set(bulk);
            let mut bus = Bus::init(rom);
            for (i, byte) in bus.iwram.iter_mut().enumerate() {
                *byte = (i as u32 ^ fill) as u8;
            }
            for (i, byte) in bus.ewram.iter_mut().enumerate() {
                *byte = (i as u32).wrapping_mul(fill) as u8;
            }
            for (i, byte) in bus.vram.iter_mut().enumerate() {
                *byte = vram(i);
            }
            dma_half(&mut bus, 0x0400_0000, dispcnt);
            bus.begin_device_batch();
            bus.elapse(lead);
            for (offset, value) in [
                (0xd4, source as u16),
                (0xd6, (source >> 16) as u16),
                (0xd8, destination as u16),
                (0xda, (destination >> 16) as u16),
                (0xdc, count as u16),
                (0xde, control | 1 << 15),
            ] {
                dma_half(&mut bus, 0x0400_0000 + offset, value);
            }
            while bus.dma_channels[3].enabled() && bus.now() < lead + 100_000 {
                bus.elapse(1);
            }
            BULK_DMA.set(true);
            bus
        };
        let (mut bulk, mut reference) = (run(true), run(false));
        let history = |bus: &Bus| {
            let count = usize::from(bus.backdrop_segment_count);
            let segments: Vec<_> =
                bus.backdrop_segments[..count].iter().map(|s| (s.start_x, s.color)).collect();
            (bus.backdrop_segment_line, segments)
        };
        assert_eq!(bulk.now(), reference.now(), "{context}: end");
        assert!(bulk.scanline_memo.same_changes(&reference.scanline_memo), "{context}: marks");
        assert_eq!(history(&bulk), history(&reference), "{context}: history at the end");
        for bus in [&mut bulk, &mut reference] {
            bus.elapse(5000);
            bus.end_device_batch();
        }
        let bytes = |bus: &Bus| {
            let mut bytes = Vec::new();
            bus.encode(&mut evenfall_state::Sink::new(Some(&mut bytes), None)).unwrap();
            bytes
        };
        assert!(bytes(&bulk) == bytes(&reference), "{context}: state");
        assert!(bulk.scanline_memo.same_changes(&reference.scanline_memo), "{context}: marks");
        assert_eq!(history(&bulk), history(&reference), "{context}: history");
        assert!(bulk.frame_pixels[..] == reference.frame_pixels[..], "{context}: pixels");
        bulk.vram.to_vec()
    }

    fn xorshift(seed: u32) -> impl FnMut() -> u32 {
        let mut seed = seed;
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        }
    }

    /// A DMA whose beats run in bulk leaves the machine as the per-beat loop
    /// does: every durable state byte, the scanline memo's marks and the
    /// raster history, for both widths, every address control, sources in
    /// IWRAM, EWRAM, the latch and ROM, destinations in IWRAM, EWRAM, VRAM
    /// and OAM, tiled and bitmap display modes, and starts in HDraw, HBlank
    /// and VBlank.
    #[test]
    fn bulk_dma_beats_match_the_per_beat_loop() {
        let mut next = xorshift(0x0d3a_b01c);
        let rom: Vec<u8> = (0..0x1000).map(|i| (i * 7) as u8).collect();
        BULK_BEATS.set(0);
        for trial in 0..400 {
            let word = next() & 1 != 0;
            let source = match next() % 8 {
                0 => 0,
                1 => 0x0800_0100 + (next() & 0x7fc),
                2..=4 => 0x0300_0000 + (next() & 0x7ffc),
                _ => 0x0200_0000 + (next() & 0x3_fffc),
            };
            let destination = match next() % 5 {
                0 => 0x0300_4000 + (next() & 0x3ffc),
                1 => 0x0203_0000 + (next() & 0xfffc),
                2 => 0x0700_0000 + (next() & 0x3fc),
                _ => 0x0600_0000 + (next() & 0x1_fffc),
            };
            let count = 1 + next() % 400;
            let control = ((next() % 4) << 5) as u16
                | ((next() % 3) << 7) as u16
                | if word { 1 << 10 } else { 0 };
            // Tiled mode 0 with BG0, bitmap modes 3 and 4 with BG2.
            let dispcnt = [0x0100, 0x0403, 0x0404, 0x0101][(next() % 4) as usize];
            let lead = 1 + u64::from(next() % 280_000);
            let fill = next();
            let context = format!(
                "trial {trial}: {source:08x} -> {destination:08x} x{count} control {control:04x} \
                 mode {dispcnt} at {lead}"
            );
            let vram = |i: usize| (i as u32 >> 3 ^ fill >> 8) as u8;
            let _ = check_bulk_dma(
                &context,
                &rom,
                source,
                destination,
                count,
                control,
                dispcnt,
                lead,
                fill,
                vram,
            );
        }
        let beats = BULK_BEATS.get();
        assert!(beats > 20_000, "only {beats} beats ran in bulk");
    }

    /// Bulk DMA to VRAM that starts in HBlank or VBlank, or on a line whose
    /// raster history is full, runs a page at a time as the per-beat loop
    /// does, including its memo marks for pages it leaves unchanged: fills
    /// from the latch and fixed sources, copies from IWRAM and EWRAM, both
    /// widths, halfword destinations off word alignment, into the upper
    /// mirrored 32 KiB and across page ends.
    #[test]
    fn vram_dma_runs_match_the_per_beat_loop() {
        let mut next = xorshift(0x51de_a11e);
        let rom = vec![0u8; 0x1000];
        VRAM_RUN_BEATS.set(0);
        for trial in 0..300 {
            let word = next() & 1 != 0;
            let source = match next() % 4 {
                0 => 0,
                1 => 0x0300_0000 + (next() & 0x7ffc),
                _ => 0x0200_0000 + (next() & 0x3_fffc),
            };
            let destination =
                0x0600_0000 + (next() & if next().is_multiple_of(4) { 0x1_fffe } else { 0x1_7ffe });
            let destination = if word { destination & !3 } else { destination };
            // Short ones end on the line they start on.
            let count = 1 + next() % if next().is_multiple_of(2) { 40 } else { 2000 };
            let source_control = [0, 0, 2, 1][(next() % 4) as usize];
            let control = source_control << 7 | if word { 1 << 10 } else { 0 };
            let dispcnt = [0x0100, 0x0403, 0x0404][(next() % 3) as usize];
            // VBlank, HBlank of a line, HDraw or its end.
            let line = u64::from(next() % 228);
            let dot = match next() % 3 {
                0 => 960 + u64::from(next() % 272),
                1 => u64::from(next() % 960),
                _ => 880 + u64::from(next() % 80),
            };
            let lead = line * 1232 + dot + 280_896;
            let fill = next();
            let context = format!(
                "trial {trial}: {source:08x} -> {destination:08x} x{count} control {control:04x} \
                 mode {dispcnt} at line {line} dot {dot}"
            );
            let vram = |i: usize| (i as u32 >> 3 ^ fill >> 8) as u8;
            let after = check_bulk_dma(
                &context,
                &rom,
                source,
                destination,
                count,
                control,
                dispcnt,
                lead,
                fill,
                vram,
            );
            // Again over VRAM that already holds what the DMA writes in two
            // pages of three, which it leaves unmarked.
            let vram = |i: usize| if (i / 1024).is_multiple_of(3) { vram(i) } else { after[i] };
            let context = format!("{context} again");
            let _ = check_bulk_dma(
                &context,
                &rom,
                source,
                destination,
                count,
                control,
                dispcnt,
                lead,
                fill,
                vram,
            );
        }
        let beats = VRAM_RUN_BEATS.get();
        assert!(beats > 50_000, "only {beats} beats ran to VRAM a page at a time");
    }

    /// State bytes, frame and audio samples at a host slice end.
    type SliceSnapshot = (Vec<u8>, Vec<u16>, Vec<u32>);

    /// A machine under HLE that takes IRQs from HBlank, VCount, VBlank and
    /// a 301-cycle timer while its ARM ROM loop divides through the BIOS,
    /// multiplies and halts. HBlank DMA copies the six words below the IRQ
    /// stack pointer into the palette and sound FIFO DMA plays them, so
    /// both see the stack between the trampoline's stores. Its snapshots at
    /// host slice ends (state bytes, the frame, the audio samples), and the
    /// bus after each entry's and return's accesses.
    fn hle_irq_machine_snapshots(irq_stack: u32) -> (Vec<SliceSnapshot>, Vec<String>) {
        use crate::machine::{Config, Machine};
        HLE_TRACE.with_borrow_mut(Vec::clear);
        // `add r4, r4, #1; mov r0, r4, lsl #9; and r1, r4, #1f; add r1, r1,
        // #3; swi 60000h; muls r5, r0, r4; tst r4, #7; swieq 20000h; b rom`.
        let program = [
            0xe284_4001,
            0xe1a0_0484,
            0xe204_101f,
            0xe281_1003,
            0xef06_0000,
            0xe015_0490,
            0xe314_0007,
            0x0f02_0000,
            0xeaff_fff6,
        ];
        // At 3001000h: `mov r0, #4000000h; ldr r1, [r0, #200h]; and r1, r1,
        // r1, lsr #16; add r2, r0, #200h; strh r1, [r2, #2]; ldrh r3, [r0,
        // #-8]; orr r3, r3, r1; strh r3, [r0, #-8]; bx lr`: acknowledge IF,
        // and mirror it for the BIOS.
        let handler = [
            0xe3a0_0301,
            0xe590_1200,
            0xe001_1821,
            0xe280_2c02,
            0xe1c2_10b2,
            0xe150_30b8,
            0xe183_3001,
            0xe140_30b8,
            0xe12f_ff1e,
        ];
        let rom: Vec<u8> = program.iter().flat_map(|word: &u32| word.to_le_bytes()).collect();
        let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
        machine.cpu.irq_r13 = irq_stack;
        let mut next = xorshift(0x1a2b_3c4d);
        machine.bus.vram.iter_mut().for_each(|byte| *byte = next() as u8);
        machine.bus.palette.iter_mut().for_each(|byte| *byte = next() as u8);
        machine.bus.scanline_memo.invalidate();
        for (index, word) in handler.into_iter().enumerate() {
            machine.bus.write_iwram(0x0300_1000 + 4 * index as u32, AccessWidth::Word, word);
        }
        machine.bus.write_iwram(0x0300_7ffc, AccessWidth::Word, 0x0300_1000);
        let stack = irq_stack.wrapping_sub(24);
        for (address, width, value) in [
            (0x0400_0204, AccessWidth::Half, 0x4317), // WAITCNT: Game Pak prefetch on
            (0x0400_0000, AccessWidth::Half, 0x0100), // DISPCNT: mode 0, BG0
            (0x0400_0004, AccessWidth::Half, 0x6438), // DISPSTAT: all IRQs, LYC 100
            (0x0400_0084, AccessWidth::Half, 0x0080), // Sound on, FIFO A on timer 0
            (0x0400_0082, AccessWidth::Half, 0x0b04),
            (0x0400_0100, AccessWidth::Half, 0xfe0c), // TM0: 500 cycles
            (0x0400_0102, AccessWidth::Half, 0x0080),
            (0x0400_0108, AccessWidth::Half, 0xfed3), // TM2: 301 cycles, IRQ
            (0x0400_010a, AccessWidth::Half, 0x00c0),
            // DMA0 copies the stack words into the palette every HBlank.
            (0x0400_00b0, AccessWidth::Word, stack),
            (0x0400_00b4, AccessWidth::Word, 0x0500_0000),
            (0x0400_00b8, AccessWidth::Half, 6),
            (0x0400_00ba, AccessWidth::Half, 0xa660),
            // DMA1 feeds them to FIFO A.
            (0x0400_00bc, AccessWidth::Word, stack),
            (0x0400_00c0, AccessWidth::Word, 0x0400_00a0),
            (0x0400_00c6, AccessWidth::Half, 0xb640),
            (0x0400_0200, AccessWidth::Half, 0x0027), // IE: LCD and timer 2
            (0x0400_0208, AccessWidth::Half, 1),
        ] {
            let access = Access {
                address,
                width,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            };
            machine.bus.write(access, value);
        }
        let mut snapshots = Vec::new();
        let mut deadline = 0;
        for slice in 0..14 {
            deadline += 61_111 + slice * 13_337;
            machine.run_until(deadline).unwrap();
            let mut samples = vec![evenfall_audio::StereoSample::default(); 8192];
            let count = machine.drain_audio_samples(&mut samples);
            let samples = samples[..count]
                .iter()
                .map(|sample| u32::from(sample.left as u16) | u32::from(sample.right as u16) << 16)
                .collect();
            snapshots.push((
                machine.write_state_alloc().unwrap(),
                machine.framebuffer().to_vec(),
                samples,
            ));
        }
        (snapshots, HLE_TRACE.take())
    }

    /// The fused HLE IRQ entry and return leave the machine as their
    /// per-access reference does: the bus where each sequence ends (clock,
    /// open bus, BIOS latch, fetch and prefetch state, device batch), and at
    /// every host slice end the state bytes, frame and audio. The stack is
    /// in IWRAM, in EWRAM, and across the two, where nothing fuses. Timer,
    /// HBlank and FIFO DMA events land inside the sequences, and those
    /// sequences take the reference.
    #[test]
    fn fused_hle_irq_trampolines_match_the_per_access_sequence() {
        for (irq_stack, fuses) in [(0x0300_7fa0, true), (0x0200_0200, true), (0x0300_0010, false)] {
            FUSED_HLE_TAKEN.set(0);
            FUSED_HLE_DECLINED.set(0);
            let (fused, fused_trace) = hle_irq_machine_snapshots(irq_stack);
            let (taken, declined) = (FUSED_HLE_TAKEN.get(), FUSED_HLE_DECLINED.get());
            FUSED_HLE.set(false);
            let (reference, reference_trace) = hle_irq_machine_snapshots(irq_stack);
            FUSED_HLE.set(true);
            assert!(reference_trace.len() > 10_000, "{irq_stack:08x}: {}", reference_trace.len());
            for (index, (fused, reference)) in fused_trace.iter().zip(&reference_trace).enumerate()
            {
                assert_eq!(fused, reference, "{irq_stack:08x}: sequence {index}");
            }
            assert_eq!(fused_trace.len(), reference_trace.len(), "{irq_stack:08x}");
            for (slice, (fused, reference)) in fused.iter().zip(&reference).enumerate() {
                let first = fused.0.iter().zip(&reference.0).position(|(a, b)| a != b);
                assert!(
                    fused.0 == reference.0,
                    "{irq_stack:08x} slice {slice}: state byte {first:?}"
                );
                assert!(fused.1 == reference.1, "{irq_stack:08x} slice {slice}: frame");
                assert!(fused.2 == reference.2, "{irq_stack:08x} slice {slice}: audio");
            }
            if fuses {
                assert!(taken > 5_000, "{irq_stack:08x}: only {taken} fused sequences");
                assert!(declined > 200, "{irq_stack:08x}: only {declined} declined sequences");
            } else {
                assert_eq!(taken + declined, 0, "{irq_stack:08x}");
            }
        }
    }
}
