//! Hardware-visible memory seam shared by CPU and DMA masters.

use crate::apu::Apu;
use crate::cartridge::{
    gpio::Gpio as CartridgeGpio,
    save::{self, Device as SaveDevice},
    tilt,
};
use crate::dma::Channel as DmaChannel;
use crate::timer::Timer;
use crate::types::Cycle;
use crate::video::{scanline_memo, tile_row_cache, timing::PpuTiming};
use evenfall_state::Codec;

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
const PALETTE_BASE: u32 = 0x0500_0000;
const VRAM_BASE: u32 = 0x0600_0000;
const OAM_BASE: u32 = 0x0700_0000;
const GAME_PAK_BASE: u32 = 0x0800_0000;
const GAME_PAK_END: u32 = 0x0e00_0000;
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

#[derive(Clone, Copy, Debug, Default, Codec)]
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
pub(crate) struct ScrollSegment {
    pub start_x: u16,
    pub hofs: u16,
    pub vofs: u16,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub(crate) struct BackdropSegment {
    pub start_x: u16,
    pub color: u16,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub(crate) struct WindowSegment {
    pub start_x: u16,
    pub win0h: u16,
    pub win0v: u16,
    pub win1h: u16,
    pub win1v: u16,
    pub winin: u16,
    pub winout: u16,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub(crate) struct BlendSegment {
    pub start_x: u16,
    pub bldcnt: u16,
    pub bldalpha: u16,
    pub bldy: u16,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub(crate) struct AffineSegment {
    pub start_x: u16,
    pub pa: i16,
    pub pc: i16,
    pub ref_x: i32,
    pub ref_y: i32,
}

#[derive(Clone, Copy, Debug, Default, Codec)]
pub(crate) struct MosaicSegment {
    pub start_x: u16,
    pub mosaic: u16,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum LineScan {
    #[default]
    Unknown,
    Absent,
    Present,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CompactObject {
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
pub(crate) struct ObjectLineData {
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
pub(crate) struct ScanlineRegisters {
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
                let offset = address as u32 & 0x00ff_ffff;
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
        _region: &mut InstructionFetchRegion,
        address: u32,
        width: AccessWidth,
    ) -> u32 {
        self.fetch_instruction(address, width)
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

    /// No fused shortcut is admitted yet; the caller uses two ordinary fetches.
    pub fn fused_thumb_game_pak_refill(
        &mut self,
        _region: &mut InstructionFetchRegion,
        _address: u32,
    ) -> Option<[u32; 2]> {
        None
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

    pub fn soft_reset_entry_address(&mut self) -> u32 {
        let reenter_ewram = self.iwram[0x7ffa] != 0;
        self.iwram[0x7e00..].fill(0);
        if reenter_ewram { EWRAM_BASE } else { GAME_PAK_BASE }
    }

    pub fn register_ram_reset(&mut self, _flags: u8) {
        todo!("synchronize devices and reset selected hardware groups")
    }

    pub fn hard_reset(&mut self) {
        todo!("BIOS register reset and console power-on state")
    }

    pub fn inspect_byte(&self, address: u32) -> u8 {
        match address >> 24 {
            0 if address < BIOS_SIZE as u32 => self.read_bios(address, AccessWidth::Byte) as u8,
            2 => self.read_ewram(address, AccessWidth::Byte) as u8,
            3 => self.read_iwram(address, AccessWidth::Byte) as u8,
            4 => todo!("side-effect-free MMIO peek"),
            5 => self.palette[(address as usize) & (PALETTE_SIZE - 1)],
            6 => vram_index(address, self.dispcnt).map_or(0, |index| self.vram[index]),
            7 => self.oam[(address as usize) & (OAM_SIZE - 1)],
            8..=13 => self.read_game_pak_rom(address, AccessWidth::Byte) as u8,
            14..=15 => self.save.peek_byte(address as u16, self.current_cycle),
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
        if (0x08..=0x0e).contains(&offset) && offset & 1 == 0 {
            return Some(self.bg_controls[((offset - 8) / 2) as usize]);
        }
        if (0x10..0x20).contains(&offset) && offset & 1 == 0 {
            return Some(0);
        }
        if (0x20..0x40).contains(&offset) && offset & 1 == 0 {
            let bg = ((offset - 0x20) / 0x10) as usize;
            let reg = (offset - 0x20) % 0x10;
            if reg < 8 {
                return Some(self.bg_affine_parameters[bg][(reg / 2) as usize] as u16);
            }
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
        if (0x10..0x20).contains(&offset) {
            let bg = ((offset - 0x10) / 4) as usize;
            return Some(if offset & 2 == 0 {
                self.bg_horizontal_offsets[bg]
            } else {
                self.bg_vertical_offsets[bg]
            });
        }
        if (0x28..0x40).contains(&offset) && (offset - 0x20) % 0x10 >= 8 {
            let bg = ((offset - 0x20) / 0x10) as usize;
            let coord = (((offset - 0x20) % 0x10 - 8) / 4) as usize;
            let raw = self.bg_reference_points[bg][coord] as u32;
            return Some(if offset & 2 != 0 { (raw >> 16) as u16 } else { raw as u16 });
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
        if (0x08..=0x0e).contains(&offset) && offset & 1 == 0 {
            let index = ((offset - 8) / 2) as usize;
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
        if (0x10..0x20).contains(&offset) && offset & 1 == 0 {
            let index = ((offset - 0x10) / 4) as usize;
            self.begin_scroll_segment(index);
            if offset & 2 != 0 {
                self.bg_vertical_offsets[index] = value & 0x1ff;
            } else {
                self.bg_horizontal_offsets[index] = value & 0x1ff;
            }
            self.end_scroll_segment(index);
            return;
        }
        if (0x20..0x40).contains(&offset) && offset & 1 == 0 {
            todo!("affine MMIO raster history");
        }
        if (0x60..0xa0).contains(&offset) {
            self.sync_apu_to(self.current_cycle);
        }
        if self.apu.write_half(address, value) {
            return;
        }
        if (0x120..=0x126).contains(&offset) && offset & 1 == 0 {
            if self.sio_control >> 12 == 1 && offset < 0x124 {
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
                } else if self.sio_control >> 12 != 3 {
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
        match self.sio_control >> 12 {
            1 if index < 2 => self.sio_data[index],
            2 => self.sio_data[index],
            _ => 0,
        }
    }

    fn is_uart_mode(&self) -> bool {
        self.rcnt & 0x8000 == 0 && self.sio_control >> 12 == 3
    }

    fn read_sio_control(&self) -> u16 {
        if self.sio_control >> 12 == 3 {
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
                Some(self.current_cycle + (bits * 16_777_216 + rate - 1) / rate);
        }
    }

    fn write_sio_control(&mut self, _value: u16) {
        todo!("SIO start and UART mode transitions")
    }
    fn write_rcnt(&mut self, _value: u16) {
        todo!("RCNT transitions")
    }
    fn read_rcnt(&self) -> u16 {
        let serial_lines = if self.sio_control >> 12 <= 1 { 5 } else { 15 };
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
            if index == 0 || !timer.count_up_enabled() {
                if cycles != 0 {
                    timer.elapse_before_overflow(cycles);
                }
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
        ] {
            if let Some(at) = at {
                boundary = boundary.min(at);
            }
        }
        for &at in &self.dma_start_cycles {
            if let Some(at) = at {
                boundary = boundary.min(at);
            }
        }
        if self.external_input_index < self.external_input_count {
            todo!("external input deadline");
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
        if self.external_input_index < self.external_input_count {
            todo!("deliver timed external inputs");
        }
        for (index, at) in self.dma_start_cycles.iter_mut().enumerate() {
            if *at == Some(self.current_cycle) {
                *at = None;
                self.pending_dma |= 1 << index;
            }
        }
        if [
            self.sio_completion_cycle,
            self.uart_tx_completion_cycle,
            self.uart_rx_arrival_cycle,
            self.joy_completion_cycle,
        ]
        .into_iter()
        .flatten()
        .any(|at| at == self.current_cycle)
        {
            todo!("deliver serial completion");
        }
        if events.hblank_any
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
        if self.pending_dma != 0 && !self.servicing_dma {
            todo!("DMA arbitration and beat dispatch");
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
            if let Some(fifo) = Apu::fifo_for_address(channel.latched_destination) {
                if requests & (1 << fifo as u8) != 0 {
                    self.pending_dma |= 1 << index;
                }
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
        let address = address as usize & (PALETTE_SIZE - 1);
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
            if index >= if self.dispcnt & 7 >= 3 { 0x14000 } else { 0x10000 } {
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
}
