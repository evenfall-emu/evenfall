//! Hardware-visible memory seam shared by CPU and DMA masters.

use crate::apu::Apu;
use crate::cartridge::{gpio::Gpio as CartridgeGpio, save::Device as SaveDevice, tilt};
use crate::dma::Channel as DmaChannel;
use crate::timer::Timer;
use crate::types::Cycle;
use crate::video::{scanline_memo, tile_row_cache, timing::PpuTiming};
use evenfall_state::Codec;

pub const SCREEN_WIDTH: usize = 240;
pub const SCREEN_HEIGHT: usize = 160;
const EWRAM_SIZE: usize = 256 * 1024;
const IWRAM_SIZE: usize = 32 * 1024;
const VRAM_SIZE: usize = 96 * 1024;
const PALETTE_SIZE: usize = 1024;
const OAM_SIZE: usize = 1024;
const MAX_SCROLL_SEGMENTS: usize = 64;

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
