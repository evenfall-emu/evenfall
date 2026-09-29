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

    pub fn read(&mut self, _access: Access) -> ReadResult {
        todo!("timed region dispatch and device event delivery")
    }

    pub fn write(&mut self, _access: Access, _value: u32) -> WriteResult {
        todo!("timed region dispatch and device event delivery")
    }

    pub fn elapse(&mut self, _cycles: Cycle) {
        todo!("timed event delivery and DMA arbitration")
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
        todo!("advance Game Pak prefetch through admitted block")
    }

    pub fn finish_cpu_ram_store_block(&mut self, _cycles: Cycle) {
        todo!("advance Game Pak prefetch through admitted block")
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
        todo!("arm the device event horizon")
    }

    pub fn end_device_batch(&mut self) {
        todo!("materialize the deferred devices")
    }

    pub fn enter_halt(&mut self) {
        todo!("flush the device batch before halting")
    }

    pub fn enter_stop(&mut self) {
        todo!("freeze clocked devices before stopping")
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
        assert_eq!(read_ram(&ram, 1, AccessWidth::Half), 0xccbb);
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
