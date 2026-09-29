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
use crate::video::{AffineTransition, scanline_memo, tile_row_cache, timing::PpuTiming};
use evenfall_state::Codec;

mod kernel;
pub use kernel::{
    ArmIwramExecutionRegion, RamWords, ThumbRomRun, begin_arm_iwram_slice, fetch_arm_iwram_in_slice,
};

pub const SCREEN_WIDTH: usize = 240;
pub const SCREEN_HEIGHT: usize = 160;
const BIOS_SIZE: usize = 16 * 1024;
const EWRAM_SIZE: usize = 256 * 1024;
const IWRAM_SIZE: usize = 32 * 1024;
const VRAM_SIZE: usize = 96 * 1024;
const PALETTE_SIZE: usize = 1024;
const OAM_SIZE: usize = 1024;
const MAX_SCROLL_SEGMENTS: usize = 64;
const EWRAM_BASE: u32 = 0x0200_0000;
const IWRAM_BASE: u32 = 0x0300_0000;
const MMIO_BASE: u32 = 0x0400_0000;
const VRAM_BASE: u32 = 0x0600_0000;
const GAME_PAK_BASE: u32 = 0x0800_0000;
const GAME_PAK_WINDOW_SIZE: u32 = 32 * 1024 * 1024;

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

#[derive(Clone, Copy, Debug)]
pub struct TimedInput {
    pub at: Cycle,
    pub change: TimedInputChange,
}

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
#[codec(tag_bits = 2)]
pub enum PowerState {
    #[default]
    Running,
    Halted,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaLivelock {
    pub started_at: Cycle,
    pub channel: u8,
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
#[codec(module = "zigba.bus.state.v6")]
pub struct Bus {
    pub ewram: Box<[u8; EWRAM_SIZE]>,
    pub iwram: Box<[u8; IWRAM_SIZE]>,
    pub palette: [u8; PALETTE_SIZE],
    pub vram: Box<[u8; VRAM_SIZE]>,
    pub oam: [u8; OAM_SIZE],
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
    pub line_window_masks: [u8; SCREEN_WIDTH],
    #[codec(skip)]
    pub line_unmasked_layers: u8,
    #[codec(skip)]
    pub line_window_key: u64,
    #[codec(skip)]
    pub tile_row_cache: Box<tile_row_cache::Cache>,
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
    pub rom: Box<[u8]>,
    #[codec(skip)]
    pub bios: Box<[u8]>,
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
    pub interrupt_enable_mask: u16,
    pub interrupt_request_flags: u16,
    pub interrupt_master_enabled: bool,
    #[codec(skip)]
    pub irq_line: bool,
    pub post_boot: u8,
    pub power_state: PowerState,
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
    #[codec(skip)]
    pub external_input_pointer: Option<*const TimedInput>,
    #[codec(skip)]
    pub external_input_count: usize,
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
    #[codec(skip)]
    pub dma_livelock: Option<DmaLivelock>,
    #[codec(skip)]
    pub active_dma_channel: Option<u8>,
    pub dma_transfer_latch: u32,
    pub open_bus_value: u32,
    pub bios_latch: u32,
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
        assert!(
            bios.is_empty() || bios.len() == BIOS_SIZE,
            "external GBA BIOS must be exactly 16 KiB"
        );
        Self {
            ewram: boxed_array(0),
            iwram: boxed_array(0),
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
            line_window_masks: [0x3f; SCREEN_WIDTH],
            line_unmasked_layers: 0x3f,
            line_window_key: 1 << 63,
            tile_row_cache: Box::default(),
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
            rom: rom.into(),
            bios: bios.into(),
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
            interrupt_enable_mask: 0,
            interrupt_request_flags: 0,
            interrupt_master_enabled: false,
            irq_line: false,
            post_boot: 0,
            power_state: PowerState::Running,
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
            external_input_pointer: None,
            external_input_count: 0,
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
            dma_livelock: None,
            active_dma_channel: None,
            dma_transfer_latch: 0,
            open_bus_value: 0,
            bios_latch: 0xe129_f000,
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

    /// Initialize a Bus in caller-provided uninitialized storage.
    ///
    /// # Safety
    /// `ptr` must be aligned, writable storage for one Bus, with no live Bus
    /// value at that address. The caller owns and drops the initialized value.
    pub unsafe fn init_in_place(
        ptr: *mut Self,
        rom: &[u8],
        bios: &[u8],
        save_type: save::Type,
        save_data: Option<&[u8]>,
    ) {
        let bus = if let Some(data) = save_data {
            Self::init_with_bios_and_save_data(rom, bios, save_type, data)
        } else {
            Self::init_with_bios_and_save(rom, bios, save_type)
        };
        // SAFETY: guaranteed by the caller; the constructor owns its ROM,
        // BIOS and boxed large arrays, and does not borrow the input slices.
        unsafe { ptr.write(bus) };
    }

    pub fn write_state(&self, sink: &mut evenfall_state::Sink<'_>) -> evenfall_state::Result<()> {
        use evenfall_state::Error;
        if self.servicing_dma
            || self.active_dma_channel.is_some()
            || self.dma_livelock.is_some()
            || self.external_input_pointer.is_some()
            || self.external_input_count != 0
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
        self.external_input_pointer = None;
        self.external_input_count = 0;
        self.external_input_index = 0;
        self.servicing_dma = false;
        self.dma_livelock = None;
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
        self.update_irq_line();
        self.validate_state()?;
        self.apu.write_state(&mut evenfall_state::Sink::new(None, None))?;
        Ok(())
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
        };
        zero_without_count_up.validate_state(prescaler).map_err(|_| InvalidSaveState)?;
        for timer in &self.timers[1..] {
            timer.validate_state(prescaler).map_err(|_| InvalidSaveState)?;
        }
        if let Some(gpio) = &self.cartridge_gpio {
            gpio.validate_state(self.current_cycle).map_err(|_| InvalidSaveState)?;
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
            AccessSource::CpuCode => self.fetch_kind = AccessKind::Sequential,
            AccessSource::CpuData => {
                self.fetch_kind = if access.address >> 24 == 4 {
                    AccessKind::Sequential
                } else {
                    AccessKind::Nonsequential
                };
            }
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
                self.complete_read(self.read_bios(address, width), 1)
            }
            2 => self.complete_read(self.read_ewram(address, width), ewram_cycles(width)),
            3 => self.complete_read(self.read_iwram(address, width), 1),
            4 => {
                let value = self.read_mmio(address, width);
                self.complete_read(value, 1)
            }
            5 => self.complete_read(self.read_palette(address, width), video_cycles(width)),
            6 => self.complete_read(self.read_vram(address, width), video_cycles(width)),
            7 => self.complete_read(self.read_oam(address, width), 1),
            8..=13
                if width == AccessWidth::Half
                    && self.save.is_eeprom()
                    && self.is_eeprom_address(address) =>
            {
                let started_at = self.current_cycle;
                self.stop_game_pak_prefetch(true);
                let value = u32::from(self.save.read_eeprom_bit(self.current_cycle));
                let cycles = self.game_pak_cycles(access);
                let mut result = self.complete_read(value, cycles);
                result.cycles = self.current_cycle - started_at;
                result
            }
            8..=13 => self.read_game_pak(access),
            14..=15 => {
                let started_at = self.current_cycle;
                self.stop_game_pak_prefetch(true);
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

    pub fn write(&mut self, access: Access, value: u32) -> WriteResult {
        let started_at = self.current_cycle;
        let address = access.address;
        let width = access.width;
        let region = address >> 24;
        if access.source == AccessSource::CpuData {
            self.fetch_kind =
                if region == 4 { AccessKind::Sequential } else { AccessKind::Nonsequential };
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
            6 => {
                if width == AccessWidth::Word {
                    for beat in 0..2 {
                        let at = address.wrapping_add(beat * 2);
                        self.advance(1);
                        self.flush_device_batch();
                        let snapshot = self.snapshot_vram_write(at, AccessWidth::Half);
                        self.write_vram(at, AccessWidth::Half, value >> (beat * 16));
                        self.record_vram_write(snapshot);
                    }
                } else {
                    self.advance(video_cycles(width));
                    self.flush_device_batch();
                    let snapshot = self.snapshot_vram_write(address, width);
                    self.write_vram(address, width, value);
                    self.record_vram_write(snapshot);
                }
            }
            7 => {
                self.advance(1);
                self.write_oam(address, width, value);
            }
            8..=13 if width == AccessWidth::Half && self.is_eeprom_access(address) => {
                self.stop_game_pak_prefetch(true);
                let cycles = self.game_pak_cycles(access);
                self.advance(cycles);
                self.save.write_eeprom_bit(value as u8, self.current_cycle);
            }
            8..=13 => {
                self.stop_game_pak_prefetch(true);
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
                self.stop_game_pak_prefetch(true);
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
        self.game_pak_code_active = true;
        self.prefetch_disabled_bug_pending = false;
        self.prefetch_buffer_was_disabled = false;
        self.start_game_pak_prefetch(address, 2);
        debug_assert_eq!(self.game_pak_prefetch.countdown, duty);
        self.continue_game_pak_prefetch(2);
        self.game_pak_prefetch.head_address = next + 2;
        self.open_bus_value = second;
        self.current_cycle = end;
        Some([first, second])
    }

    pub fn begin_device_batch(&mut self) {
        debug_assert!(!self.batch_active);
        debug_assert_eq!(self.apu_cycle, self.current_cycle);
        debug_assert_eq!(self.timer_cycle, self.current_cycle);
        self.batch_device_cycle = self.current_cycle;
        self.batch_active = true;
        self.batch_deadline = self.next_device_boundary();
    }

    pub fn end_device_batch(&mut self) {
        self.synchronize_devices();
        self.batch_active = false;
        self.batch_device_cycle = 0;
        self.batch_deadline = 0;
    }

    pub fn enter_halt(&mut self) {
        self.flush_device_batch();
        self.batch_deadline = 0;
        self.power_state = PowerState::Halted;
    }

    pub fn enter_stop(&mut self) {
        self.synchronize_devices();
        self.batch_deadline = 0;
        self.power_state = PowerState::Stopped;
    }

    pub fn idle_halt_until(&mut self, deadline: Cycle) -> bool {
        debug_assert_eq!(self.power_state, PowerState::Halted);
        debug_assert!(deadline >= self.current_cycle);
        self.apply_due_external_inputs();
        while !self.halt_wake_pending()
            && self.current_cycle < deadline
            && self.dma_livelock.is_none()
        {
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
        debug_assert_eq!(self.power_state, PowerState::Stopped);
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
        }
        if flags & 0x80 != 0 {
            self.reset_general_registers();
        }
    }

    pub fn hard_reset(&mut self) {
        self.register_ram_reset(0xff);
        self.iwram[0x7e00..].fill(0);
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
        self.post_boot = 1;
        self.power_state = PowerState::Running;
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
        self.dma_start_cycles = [None; 4];
        self.pending_dma = 0;
        self.servicing_dma = false;
        self.active_dma_channel = None;
        self.dma_transfer_latch = 0;
        self.interrupt_enable_mask = 0;
        self.interrupt_request_flags = 0;
        self.interrupt_master_enabled = false;
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

    /// Install caller-owned inputs for one synchronous run. The caller must
    /// keep the slice alive and immobile until `end_external_inputs`.
    ///
    /// # Safety
    /// `inputs` must remain allocated and immutable through that call.
    pub unsafe fn begin_external_inputs(&mut self, inputs: &[TimedInput]) {
        debug_assert!(self.external_input_pointer.is_none());
        if inputs.is_empty() {
            return;
        }
        self.external_input_pointer = Some(inputs.as_ptr());
        self.external_input_count = inputs.len();
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
        self.external_input_pointer = None;
        self.external_input_count = 0;
        self.external_input_index = 0;
        if self.batch_active {
            self.flush_device_batch();
            self.batch_deadline = self.next_device_boundary();
        }
    }

    fn next_external_input_cycle(&self) -> Option<Cycle> {
        if self.external_input_index == self.external_input_count {
            return None;
        }
        // SAFETY: begin_external_inputs requires this slice to remain alive
        // and immutable until end_external_inputs; index is strictly in range.
        Some(unsafe { (*self.external_input_pointer.unwrap().add(self.external_input_index)).at })
    }

    fn apply_due_external_inputs(&mut self) {
        while self.external_input_index < self.external_input_count {
            // SAFETY: the synchronous run holds the input slice alive, and
            // the loop bounds the index by the supplied slice length.
            let input =
                unsafe { *self.external_input_pointer.unwrap().add(self.external_input_index) };
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
            self.interrupt_request_flags |= 1 << 12;
            self.update_irq_line();
        }
        self.keypad_condition_met = condition;
    }

    pub fn irq_pending(&self) -> bool {
        self.interrupt_master_enabled
            && self.interrupt_enable_mask & self.interrupt_request_flags != 0
    }

    pub fn update_irq_line(&mut self) {
        self.irq_line = self.irq_pending();
    }

    pub fn power_state(&self) -> PowerState {
        self.power_state
    }

    pub fn resume_power(&mut self) {
        self.power_state = PowerState::Running;
    }

    pub fn halt_wake_pending(&self) -> bool {
        self.interrupt_enable_mask & self.interrupt_request_flags != 0
    }

    pub fn stop_wake_pending(&self) -> bool {
        self.interrupt_enable_mask
            & self.interrupt_request_flags
            & ((1 << 7) | (1 << 12) | (1 << 13))
            != 0
    }

    pub fn completed_frames(&self) -> Cycle {
        self.completed_frame_count
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

    pub fn prescaler_counter(&self) -> u16 {
        (self.timer_cycle.wrapping_sub(u64::from(self.prescaler_origin)) as u16) & 1023
    }

    fn read_bios(&self, address: u32, width: AccessWidth) -> u32 {
        if self.bios.len() != BIOS_SIZE {
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
            return (self.bios_latch >> shift) & mask;
        }
        (0..width.bytes())
            .map(|offset| u32::from(self.bios[address as usize + offset]) << (offset * 8))
            .fold(0, |a, b| a | b)
    }

    fn read_game_pak_rom(&self, address: u32, width: AccessWidth) -> u32 {
        let first = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        let end = first + width.bytes();
        if end <= self.rom.len()
            && end <= GAME_PAK_WINDOW_SIZE as usize
            && (self.cartridge_gpio.is_none() || first > 0xc9 || end <= 0xc4)
        {
            return (0..width.bytes()).fold(0u32, |value, offset| {
                value | (u32::from(self.rom[first + offset]) << (offset * 8))
            });
        }
        (0..width.bytes()).fold(0u32, |value, offset| {
            let byte_address = address.wrapping_add(offset as u32);
            let index = ((byte_address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
            let port = self.cartridge_gpio.as_ref().and_then(|gpio| gpio.read_byte(index as u32));
            let byte = port.unwrap_or_else(|| {
                self.rom.get(index).copied().unwrap_or_else(|| {
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

    fn write_ewram(&mut self, address: u32, width: AccessWidth, value: u32) {
        write_ram(
            &mut self.ewram,
            (address.wrapping_sub(EWRAM_BASE) as usize) & (EWRAM_SIZE - 1),
            width,
            value,
        );
    }

    fn read_iwram(&self, address: u32, width: AccessWidth) -> u32 {
        read_ram(&self.iwram, (address.wrapping_sub(IWRAM_BASE) as usize) & (IWRAM_SIZE - 1), width)
    }

    fn write_iwram(&mut self, address: u32, width: AccessWidth, value: u32) {
        write_ram(
            &mut self.iwram,
            (address.wrapping_sub(IWRAM_BASE) as usize) & (IWRAM_SIZE - 1),
            width,
            value,
        );
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
        for window in 0..3 {
            self.game_pak_n_cycles[window] =
                [5, 4, 3, 9][usize::from((self.waitcnt >> (2 + window * 3)) & 3)];
            self.game_pak_s_cycles[window] =
                if self.waitcnt & (1 << (4 + window * 3)) != 0 { 2 } else { [3, 5, 9][window] };
        }
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

    fn write_cartridge_gpio_half(&mut self, address: u32, value: u16) {
        let offset = (address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1);
        if self
            .cartridge_gpio
            .as_mut()
            .is_some_and(|gpio| gpio.write_half(offset, value, self.current_cycle))
        {
            self.interrupt_request_flags |= 1 << 13;
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

    fn peek_mmio_half(&self, address: u32) -> Option<u16> {
        if (MMIO_BASE + 0xb0..MMIO_BASE + 0xe0).contains(&address) && address & 1 == 0 {
            let offset = address - (MMIO_BASE + 0xb0);
            let channel = &self.dma_channels[(offset / 12) as usize];
            return Some(if offset % 12 == 10 { channel.control } else { 0 });
        }
        if (MMIO_BASE + 0x100..=MMIO_BASE + 0x10e).contains(&address) && address & 1 == 0 {
            let offset = address - (MMIO_BASE + 0x100);
            let timer = &self.timers[(offset / 4) as usize];
            return Some(if offset & 2 != 0 { timer.read_control() } else { timer.read_counter() });
        }
        let offset = address.wrapping_sub(MMIO_BASE);
        if let Some(index) = Self::background_control_index(address) {
            return Some(self.bg_controls[index]);
        }
        if Self::background_offset_register(address).is_some() {
            return Some(0);
        }
        if let Some((bg, parameter)) = Self::affine_parameter_register(address) {
            return Some(self.bg_affine_parameters[bg][parameter] as u16);
        }
        if Self::affine_reference_register(address).is_some() {
            return Some(0);
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
            0x040 => self.win0h,
            0x042 => self.win1h,
            0x044 => self.win0v,
            0x046 => self.win1v,
            0x048 => self.winin,
            0x04a => self.winout,
            0x04c => self.mosaic,
            0x050 => self.bldcnt,
            0x052 => self.bldalpha,
            0x054 => self.bldy,
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
            0x200 => self.interrupt_enable_mask,
            0x202 => self.interrupt_request_flags,
            0x204 => self.waitcnt,
            0x208 => u16::from(self.interrupt_master_enabled),
            0x300 => u16::from(self.post_boot),
            _ => return None,
        })
    }

    fn write_mmio(&mut self, address: u32, width: AccessWidth, value: u32) {
        if Apu::accepts_fifo_write(address, width.bytes() as u8) {
            self.apu.write_fifo(address, value, width.bytes() as u8);
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
            self.post_boot = value & 1;
            return;
        }
        if address == MMIO_BASE + 0x301 {
            self.power_state =
                if value & 0x80 != 0 { PowerState::Stopped } else { PowerState::Halted };
            return;
        }
        if (MMIO_BASE + 0x60..MMIO_BASE + 0xa0).contains(&address) {
            self.sync_apu_to(self.current_cycle);
        }
        if self.apu.write_byte(address, value) {
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
            self.interrupt_request_flags &= !(u16::from(value) << ((address & 1) * 8));
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
        if offset == 0x154 {
            return Some(self.joy_transmit as u16);
        }
        if offset == 0x156 {
            return Some((self.joy_transmit >> 16) as u16);
        }
        self.peek_mmio_half(address)
    }

    fn write_mmio_half(&mut self, address: u32, value: u16) {
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
            } else {
                self.timers[index].write_reload(value);
            }
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
                self.interrupt_enable_mask = value & 0x3fff;
                self.update_irq_line();
            }
            0x202 => {
                self.interrupt_request_flags &= !value;
                self.update_irq_line();
            }
            0x204 => self.write_waitcnt(value),
            0x208 => {
                self.interrupt_master_enabled = value & 1 != 0;
                self.update_irq_line();
            }
            0x300 => {
                self.post_boot = (value & 1) as u8;
                self.power_state =
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
            self.interrupt_request_flags |= 1 << 7;
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
            self.interrupt_request_flags |= 1 << 7;
            self.update_irq_line();
        }
    }

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
            self.interrupt_request_flags |= 1 << 7;
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
        self.sync_apu_to(self.current_cycle);
        self.sync_timers_to(self.current_cycle);
    }

    fn flush_device_batch(&mut self) {
        if !self.batch_active || self.current_cycle == self.batch_device_cycle {
            return;
        }
        let cycles = self.current_cycle - self.batch_device_cycle;
        debug_assert!(self.current_cycle < self.batch_deadline);
        self.ppu_timing.elapse_before_boundary(cycles);
        self.sync_apu_to(self.current_cycle);
        self.sync_timers_to(self.current_cycle);
        self.batch_device_cycle = self.current_cycle;
    }

    fn sync_apu_to(&mut self, target: Cycle) {
        let cycles = target - self.apu_cycle;
        self.apu_cycle = target;
        if cycles != 0 {
            self.apu.elapse(cycles);
        }
    }

    fn sync_timers_to(&mut self, target: Cycle) {
        let cycles = target - self.timer_cycle;
        self.timer_cycle = target;
        for (index, timer) in self.timers.iter_mut().enumerate() {
            if (index == 0 || !timer.count_up_enabled()) && cycles != 0 {
                timer.elapse_before_overflow(cycles);
            }
        }
    }

    fn next_timer_overflow(&self) -> Cycle {
        self.timers
            .iter()
            .enumerate()
            .filter_map(|(index, timer)| {
                if index != 0 && timer.count_up_enabled() {
                    None
                } else {
                    timer
                        .cycles_until_overflow()
                        .map(|cycles| self.timer_cycle.saturating_add(cycles))
                }
            })
            .min()
            .unwrap_or(Cycle::MAX)
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
        if let Some(at) = self.next_external_input_cycle() {
            boundary = boundary.min(at);
        }
        boundary
    }

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

    fn advance_step(&mut self, cycles: Cycle) {
        self.advance_game_pak_prefetch(cycles);
        let events = self.ppu_timing.elapse(cycles);
        self.sync_apu_to(self.current_cycle + cycles);
        let elapsed = self.current_cycle + cycles - self.timer_cycle;
        self.timer_cycle += elapsed;
        let mut previous = 0;
        let mut audio_requests = 0;
        for (index, timer) in self.timers.iter_mut().enumerate() {
            let overflows = if index != 0 && timer.count_up_enabled() {
                timer.cascade(previous)
            } else {
                timer.elapse(elapsed)
            };
            if overflows != 0 && timer.irq_enabled() {
                self.interrupt_request_flags |= 1 << (index + 3);
            }
            if index < 2 && overflows != 0 {
                audio_requests |= self.apu.timer_overflow(index, overflows);
            }
            previous = overflows;
        }
        self.update_irq_line();
        self.current_cycle += cycles;
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
            self.deliver_lcd_events(events);
        }
        if audio_requests != 0 {
            self.queue_audio_dma(audio_requests);
        }
    }

    fn deliver_lcd_events(&mut self, events: crate::video::timing::Events) {
        for (line, &completed) in events.hblank_lines.iter().enumerate() {
            if !completed {
                continue;
            }
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
            self.interrupt_request_flags |= 1;
        }
        if events.hblank_irq {
            self.interrupt_request_flags |= 1 << 1;
        }
        if events.vcount_irq {
            self.interrupt_request_flags |= 1 << 2;
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
        if self.dma_livelock.is_some() {
            self.pending_dma = 0;
            return;
        }
        self.servicing_dma = true;
        self.dma_burst_start = self.current_cycle;
        let sub_batch = !self.batch_active;
        if sub_batch {
            self.begin_device_batch();
        }
        while self.pending_dma != 0 {
            let index = self.pending_dma.trailing_zeros() as usize;
            self.pending_dma &= !(1 << index);
            if self.dma_channels[index].enabled() {
                if self.dma_burst_exhausted(index) {
                    break;
                }
                self.run_dma(index);
            }
        }
        if sub_batch {
            self.end_device_batch();
        }
        self.servicing_dma = false;
    }

    pub const MAX_DMA_BURST_CYCLES: Cycle = 1 << 24;

    fn dma_burst_exhausted(&mut self, index: usize) -> bool {
        if self.current_cycle - self.dma_burst_start < Self::MAX_DMA_BURST_CYCLES {
            return false;
        }
        self.dma_livelock =
            Some(DmaLivelock { started_at: self.dma_burst_start, channel: index as u8 });
        self.pending_dma = 0;
        true
    }

    pub fn dma_livelock(&self) -> Option<DmaLivelock> {
        self.dma_livelock
    }

    fn run_dma(&mut self, index: usize) {
        let previous_active = self.active_dma_channel;
        self.active_dma_channel = Some(index as u8);
        self.dma_epoch = self.dma_epoch.wrapping_add(1);
        let mut source = self.dma_channels[index].latched_source;
        let mut destination = self.dma_channels[index].latched_destination;
        let control = self.dma_channels[index].control;
        let is_audio_fifo = (1..=2).contains(&index)
            && (control >> 12) & 3 == 3
            && Apu::fifo_for_address(destination).is_some();
        let width = if is_audio_fifo || control & (1 << 10) != 0 {
            AccessWidth::Word
        } else {
            AccessWidth::Half
        };
        let unit_bytes = width.bytes() as u32;
        let transfer_words =
            if is_audio_fifo { 4 } else { self.dma_channels[index].transfer_words(index) };
        if index == 3
            && width == AccessWidth::Half
            && self.is_eeprom_address(destination)
            && self.save.is_auto()
        {
            self.save.resolve_eeprom_transfer(transfer_words);
        }
        let mut did_access_rom = false;
        for transferred in 0..transfer_words {
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
                self.service_higher_priority_dma(index);
                if self.dma_livelock.is_some() {
                    self.active_dma_channel = previous_active;
                    return;
                }
            }
        }
        self.advance(2);
        self.dma_channels[index].latched_source = source;
        let repeat = control & (1 << 9) != 0 && (control >> 12) & 3 != 0;
        if repeat && (control >> 5) & 3 == 3 {
            self.dma_channels[index].reload_latched_destination();
        } else {
            self.dma_channels[index].latched_destination = destination;
        }
        if !repeat {
            self.dma_channels[index].control &= !(1 << 15);
        }
        if control & (1 << 14) != 0 {
            self.interrupt_request_flags |= 1 << (index + 8);
            self.update_irq_line();
        }
        self.active_dma_channel = previous_active;
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
            _ if Apu::accepts_fifo_write(address, width.bytes() as u8) => {
                self.advance(1);
                self.apu.write_fifo(address, value, width.bytes() as u8);
            }
            _ => {
                self.write(Access { address, width, kind, source: AccessSource::Dma }, value);
                return;
            }
        }
        self.service_pending_dma();
    }

    fn service_higher_priority_dma(&mut self, current_index: usize) {
        if current_index == 0 {
            return;
        }
        let higher_mask = (1u8 << current_index) - 1;
        while self.pending_dma & higher_mask != 0 {
            let index = (self.pending_dma & higher_mask).trailing_zeros() as usize;
            self.pending_dma &= !(1 << index);
            if self.dma_channels[index].enabled() {
                if self.dma_burst_exhausted(index) {
                    return;
                }
                self.run_dma(index);
            }
        }
    }

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
        read_ram(&self.palette, address as usize & (PALETTE_SIZE - 1), width)
    }

    fn write_palette(&mut self, address: u32, width: AccessWidth, value: u32) {
        let address = (if width == AccessWidth::Byte { address & !1 } else { address }) as usize
            & (PALETTE_SIZE - 1);
        let mut changed = false;
        for offset in 0..width.bytes().max(2) {
            let byte = if width == AccessWidth::Byte {
                value as u8
            } else {
                (value >> (offset * 8)) as u8
            };
            let index = (address + offset) & (PALETTE_SIZE - 1);
            changed |= self.palette[index] != byte;
            self.palette[index] = byte;
        }
        if changed {
            self.scanline_memo.mark_palette();
        }
    }

    fn read_vram(&self, address: u32, width: AccessWidth) -> u32 {
        (0..width.bytes()).fold(0, |value, offset| {
            let byte = vram_index(address.wrapping_add(offset as u32), self.dispcnt)
                .map_or(0, |index| self.vram[index]);
            value | (u32::from(byte) << (offset * 8))
        })
    }

    fn write_vram(&mut self, address: u32, width: AccessWidth, value: u32) {
        if width == AccessWidth::Byte {
            let Some(index) = vram_index(address & !1, self.dispcnt) else {
                return;
            };
            if index >= vram_obj_boundary(self.dispcnt) {
                return;
            }
            for index in index..index + 2 {
                self.write_vram_byte(index, value as u8);
            }
            return;
        }
        for offset in 0..width.bytes() {
            if let Some(index) = vram_index(address.wrapping_add(offset as u32), self.dispcnt) {
                self.write_vram_byte(index, (value >> (offset * 8)) as u8);
            }
        }
    }

    fn write_vram_byte(&mut self, index: usize, value: u8) {
        if self.vram[index] != value {
            self.scanline_memo.mark_vram(index);
        }
        self.vram[index] = value;
    }

    fn read_oam(&self, address: u32, width: AccessWidth) -> u32 {
        read_ram(&self.oam, address as usize & (OAM_SIZE - 1), width)
    }

    fn write_oam(&mut self, address: u32, width: AccessWidth, value: u32) {
        if width == AccessWidth::Byte {
            return;
        }
        write_ram(&mut self.oam, address as usize & (OAM_SIZE - 1), width, value);
        self.scanline_memo.mark_oam();
        self.oam_compact_valid = false;
        self.object_line_index = 0xffff;
    }

    fn complete_read(&mut self, value: u32, cycles: Cycle) -> ReadResult {
        let started_at = self.current_cycle;
        self.open_bus_value = value;
        self.advance(cycles);
        self.service_pending_dma();
        ReadResult { value, cycles: self.current_cycle - started_at }
    }

    fn read_game_pak(&mut self, mut access: Access) -> ReadResult {
        let started_at = self.current_cycle;
        let value = self.read_game_pak_rom(access.address, access.width);
        if access.source != AccessSource::CpuCode {
            self.stop_game_pak_prefetch(true);
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
                self.complete_read(value, 1);
                return ReadResult { value, cycles: self.current_cycle - started_at };
            }
            if self.game_pak_prefetch.countdown != 0
                && aligned == self.game_pak_prefetch.last_address
            {
                let remaining = self.game_pak_prefetch.countdown;
                self.complete_read(value, remaining);
                self.game_pak_prefetch.head_address = aligned.wrapping_add(u32::from(opcode_bytes));
                self.game_pak_prefetch.count = 0;
                return ReadResult { value, cycles: self.current_cycle - started_at };
            }
        }
        self.stop_game_pak_prefetch(false);
        if self.prefetch_buffer_was_disabled {
            access.kind = AccessKind::Nonsequential;
            self.prefetch_buffer_was_disabled = false;
        }
        let fetch_started = self.current_cycle;
        let epoch = self.dma_epoch;
        let cycles = self.game_pak_cycles(access);
        self.complete_read(value, cycles);
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

    fn stop_game_pak_prefetch(&mut self, collision: bool) {
        if self.game_pak_prefetch.active && collision && self.game_pak_prefetch.countdown != 0 {
            let halfway = self.game_pak_prefetch.duty / 2 + 1;
            if self.game_pak_prefetch.countdown == 1
                || (self.game_pak_prefetch.opcode_bytes == 4
                    && self.game_pak_prefetch.countdown == halfway)
            {
                self.advance(1);
            }
        }
        self.game_pak_prefetch = GamePakPrefetch::default();
    }

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

fn ewram_cycles(width: AccessWidth) -> Cycle {
    if width == AccessWidth::Word { 6 } else { 3 }
}

fn video_cycles(width: AccessWidth) -> Cycle {
    if width == AccessWidth::Word { 2 } else { 1 }
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
    const fn bytes(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
        }
    }
}

fn read_ram<const N: usize>(ram: &[u8; N], index: usize, width: AccessWidth) -> u32 {
    (0..width.bytes())
        .map(|offset| u32::from(ram[(index + offset) & (N - 1)]) << (offset * 8))
        .fold(0, |a, b| a | b)
}

fn write_ram<const N: usize>(ram: &mut [u8; N], index: usize, width: AccessWidth, value: u32) {
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

    /// Zig: "DMA3 immediate starts two cycles after its enabling write, copies halfwords, clears enable, and requests IRQ".
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
        assert_eq!(bus.interrupt_request_flags & (1 << 11), 1 << 11);
    }

    /// ZiGBA probe-vdma: VBlank DMA3 uploads a solid tile without enabling VBlank IRQ.
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

    /// Zig: "DMA source zero reuses the shared transfer latch without advancing".
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

    /// Zig: "SIO synchronization waits for both normal peers and rejects one past the deadline".
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

    /// Zig: "four-player SIO exchanges every multiplayer word at the parent baud deadline".
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
            assert_ne!(bus.interrupt_request_flags & (1 << 7), 0);
        }
    }

    /// Zig: "UART CTS waits until the peer receiver has capacity".
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

    /// Zig: "BIOS hard reset preserves host audio suppression".
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

    /// Zig: "no-event materialization precedes register reset and hard reset clock changes".
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

    /// Zig: RAM accesses preserve byte order, unaligned mirror wrapping and bus timing.
    #[test]
    fn ram_mirror_crossing_preserves_little_endian_byte_order() {
        let mut ram = [0u8; 8];
        write_ram(&mut ram, 7, AccessWidth::Word, 0xddcc_bbaa);
        assert_eq!(ram, [0xbb, 0xcc, 0xdd, 0, 0, 0, 0, 0xaa]);
        assert_eq!(read_ram(&ram, 7, AccessWidth::Word), 0xddcc_bbaa);
        assert_eq!(read_ram(&ram, 1, AccessWidth::Half), 0xddcc);
        assert_eq!(read_ram(&ram, 0, AccessWidth::Byte), 0xbb);
    }

    /// Zig: bitmap modes leave the lower half of mirrored VRAM unmapped.
    #[test]
    fn bitmap_modes_leave_lower_mirrored_vram_unmapped() {
        assert_eq!(vram_index(0x0601_8000, 0), Some(0x1_0000));
        assert_eq!(vram_index(0x0601_8000, 3), None);
        assert_eq!(vram_index(0x0601_c000, 3), Some(0x1_4000));
        assert_eq!(vram_index(0x0603_c000, 3), Some(0x1_4000));
        assert_eq!(vram_obj_boundary(2), 0x1_0000);
        assert_eq!(vram_obj_boundary(4), 0x1_4000);
    }

    /// Zig: "Game Pak prefetch batching matches single-cycle advancement through capacity".
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

    /// Zig: "fused Thumb Game Pak refill matches two ordinary fetches".
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

    /// Zig: "cached IRQ line follows IE IF IME writes and keypad edges".
    #[test]
    fn cached_irq_line_follows_ie_if_ime_writes_and_keypad_edges() {
        let mut bus = Bus::init(&[]);
        let half = |address| Access {
            address,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        };
        bus.write(half(MMIO_BASE + 0x132), 0x4001);
        bus.write(half(MMIO_BASE + 0x200), 1 << 12);
        bus.write(half(MMIO_BASE + 0x208), 1);
        assert!(!bus.irq_line);
        bus.set_keys(KeyState { a: true, ..KeyState::default() });
        assert!(bus.irq_line);
        let byte = Access {
            address: MMIO_BASE + 0x203,
            width: AccessWidth::Byte,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        };
        bus.write(byte, 1 << 4);
        assert!(!bus.irq_line);
        bus.set_keys(KeyState::default());
        bus.set_keys(KeyState { a: true, ..KeyState::default() });
        assert!(bus.irq_line);
        bus.write(half(MMIO_BASE + 0x200), 0);
        assert!(!bus.irq_line);
        bus.write(half(MMIO_BASE + 0x200), 1 << 12);
        assert!(bus.irq_line);
        bus.write(half(MMIO_BASE + 0x208), 0);
        assert!(!bus.irq_line);
        bus.write(half(MMIO_BASE + 0x208), 1);
        assert!(bus.irq_line);
        bus.write(half(MMIO_BASE + 0x202), 1 << 12);
        assert!(!bus.irq_line);
    }

    /// Zig: "stable bus state retains only strictly future explicit deadlines".
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

    /// Zig: "normal-mode internal-clock SIO transfers complete at their bit-clock deadline".
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
            assert_eq!(bus.interrupt_request_flags & (1 << 7), 0);
            bus.elapse(1);
            assert_eq!(bus.read(half(MMIO_BASE + 0x128)).value & 0x80, 0);
            assert_eq!(bus.interrupt_request_flags & (1 << 7), 1 << 7);
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

    /// Zig: "JoyBus status and reset commands return the GBA type and raise reset IRQ".
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
        assert_eq!(bus.interrupt_request_flags & (1 << 7), 0);
        bus.write(half(MMIO_BASE + 0x200), 1 << 7);
        let deadline = bus.now() + 37;
        bus.schedule_joy_bus(JoyBusCommand::Reset, deadline).unwrap();
        assert_eq!(bus.take_joy_bus_response(), None);
        bus.enter_halt();
        assert!(bus.idle_halt_until(deadline));
        assert_eq!(bus.now(), deadline);
        assert_eq!(bus.take_joy_bus_response().unwrap().data[..3], [0, 4, 0]);
        assert_eq!(bus.read(half(MMIO_BASE + 0x140)).value, 0x41);
        assert_eq!(bus.interrupt_request_flags & (1 << 7), 1 << 7);
    }

    /// Zig: timed key changes apply at their exact cycle, before serial and LCD events.
    #[test]
    fn timed_key_changes_apply_at_their_exact_cycle() {
        let mut bus = Bus::init(&[]);
        bus.keypad_control = 0x4001;
        let inputs = [TimedInput {
            at: 73,
            change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() }),
        }];
        // SAFETY: `inputs` stays alive and immutable until end_external_inputs.
        unsafe {
            bus.begin_external_inputs(&inputs);
        }
        bus.begin_device_batch();
        bus.elapse(72);
        assert_eq!(bus.interrupt_request_flags & (1 << 12), 0);
        bus.elapse(1);
        assert_eq!(bus.interrupt_request_flags & (1 << 12), 1 << 12);
        assert_eq!(bus.external_inputs_consumed(), 1);
        bus.end_device_batch();
        bus.end_external_inputs();
    }
}
