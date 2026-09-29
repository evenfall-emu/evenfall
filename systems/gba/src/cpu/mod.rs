//! GBA CPU state and the scheduling seam around the ARM7TDMI core.

use evenfall_cpu_arm::Core;
use evenfall_state::Codec;
use std::ops::{Deref, DerefMut};

pub use evenfall_cpu_arm::InstructionSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleIrqFrame {
    pub return_address: u32,
    pub saved_spsr_irq: u32,
    pub saved_r0_r3: [u32; 4],
    pub saved_r12: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleSoundChannelClearContinuation {
    pub sound_area: u32,
    pub identifier: u32,
    pub extension_cursor: u32,
    pub channel: u8,
    pub return_address: u32,
    pub return_cpsr: u32,
    pub return_lr: u32,
    pub saved_r4_r7: [u32; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
#[codec(tag_bits = 2)]
pub enum HleSoundDriverMainPhase {
    #[default]
    AfterPlayerCallback,
    AfterMp2kCallback,
    AfterCgbCallback,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleSoundDriverMainContinuation {
    pub sound_area: u32,
    pub scanline_limit: u32,
    pub phase: HleSoundDriverMainPhase,
    pub return_address: u32,
    pub return_cpsr: u32,
    pub return_lr: u32,
    pub saved_r4_r11: [u32; 8],
    pub pending_players: [u32; 16],
    #[codec(bits = 5)]
    pub pending_player_count: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
#[codec(tag_bits = 4)]
pub enum HleMultiBootPhase {
    #[default]
    InitialDelay,
    LaunchLength,
    WaitLength,
    LaunchData,
    WaitData,
    LaunchCrcRequest,
    WaitCrcRequest,
    LaunchCrcSignal,
    WaitCrcSignal,
    LaunchCrc,
    WaitCrc,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleMultiBootContinuation {
    pub parameters: MultiBootParameters,
    pub mode: MultiBootMode,
    pub encoder: MultiBootEncoder,
    pub phase: HleMultiBootPhase,
    pub source_offset: u32,
    pub encoded_word: u32,
    pub multiplayer_high_half: bool,
    pub random_data: [u8; 3],
    pub final_checksum: u16,
    pub wait_started_at: u64,
    pub instruction_address: u32,
    pub instruction_set: InstructionSet,
}

// These nested protocol layouts are ZiGBA multiboot.Parameters/Mode/Encoder.
// Once the devices crate is merged, they are replaced by its canonical types.
#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct MultiBootParameters {
    pub handshake_data: u8,
    pub client_data: [u8; 3],
    pub palette_data: u8,
    pub client_bit: u8,
    pub source: u32,
    pub end: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
#[codec(tag_bits = 2)]
pub enum MultiBootMode {
    #[default]
    #[codec(rename = "normal_256_khz")]
    Normal256Khz,
    #[codec(rename = "multiplayer_115_khz")]
    Multiplayer115Khz,
    #[codec(rename = "normal_2_mhz")]
    Normal2Mhz,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct MultiBootEncoder {
    pub checksum: u32,
    pub key_state: u32,
    pub checksum_polynomial: u32,
    pub key_xor: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleCpuSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub fill_value: u32,
    pub fill: bool,
    pub word: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleCpuFastSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining_blocks: u32,
    pub last_word: u32,
    pub fill: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleBgAffineSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleObjAffineSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub stride: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleBitUnpackWork {
    pub source: u32,
    pub destination: u32,
    pub source_length: u32,
    pub bias: u32,
    pub input: u32,
    pub output: u32,
    pub source_width: u8,
    pub destination_width: u8,
    pub input_bits_remaining: u8,
    pub output_bits: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleLz77Work {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub pending_halfword: u16,
    pub block_header: u8,
    #[codec(bits = 4)]
    pub blocks_remaining: u8,
    pub vram_output: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleHuffmanWork {
    pub tree_base: u32,
    pub bitstream_address: u32,
    pub destination: u32,
    pub remaining: u32,
    pub node_address: u32,
    pub output: u32,
    pub node: u8,
    #[codec(bits = 6)]
    pub symbol_bits: u8,
    #[codec(bits = 6)]
    pub output_bits: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleRunLengthWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub padding: u32,
    pub pending_halfword: u16,
    pub vram_output: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleDifferentialFilterWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub previous: u16,
    pub pending_halfword: u16,
    pub input_halfword: bool,
    pub output_halfword: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Codec)]
pub enum HleBulkWork {
    CpuSet(HleCpuSetWork),
    CpuFastSet(HleCpuFastSetWork),
    BgAffineSet(HleBgAffineSetWork),
    ObjAffineSet(HleObjAffineSetWork),
    BitUnpack(HleBitUnpackWork),
    Lz77(HleLz77Work),
    Huffman(HleHuffmanWork),
    RunLength(HleRunLengthWork),
    DifferentialFilter(HleDifferentialFilterWork),
}

impl Default for HleBulkWork {
    fn default() -> Self {
        Self::CpuSet(HleCpuSetWork::default())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
pub struct HleBulkService {
    pub instruction_address: u32,
    pub instruction: u32,
    pub instruction_set: InstructionSet,
    pub work: HleBulkWork,
}

#[derive(Clone, Debug, PartialEq, Eq, Codec)]
pub enum HleService {
    #[codec(rename = "multiboot")]
    MultiBoot(HleMultiBootContinuation),
    Bulk(HleBulkService),
}

impl Default for HleService {
    fn default() -> Self {
        Self::MultiBoot(HleMultiBootContinuation::default())
    }
}

/// ZiGBA's single flat `Cpu`: the core prefix precedes all HLE fields.
#[derive(Clone, Debug, PartialEq, Eq, Codec)]
pub struct Cpu {
    #[codec(flatten)]
    pub core: Core,
    pub hle_bios: bool,
    pub hle_irq_stack: [HleIrqFrame; 8],
    pub hle_irq_depth: u8,
    pub hle_interrupt_wait_mask: Option<u16>,
    pub hle_sound_channel_clear: Option<HleSoundChannelClearContinuation>,
    pub hle_sound_driver_main: Option<HleSoundDriverMainContinuation>,
    pub hle_service: Option<HleService>,
}

impl Cpu {
    pub fn write_state_schema(sink: &mut evenfall_state::Sink<'_>) -> evenfall_state::Result<()> {
        evenfall_state::schema_name(sink, "zigba.cpu.state.v6")?;
        Self::schema(sink)
    }

    pub fn init(entry_address: u32, initial_cpsr: u32, hle_bios: bool) -> Self {
        Self {
            core: Core::new(entry_address, initial_cpsr),
            hle_bios,
            hle_irq_stack: [HleIrqFrame::default(); 8],
            hle_irq_depth: 0,
            hle_interrupt_wait_mask: None,
            hle_sound_channel_clear: None,
            hle_sound_driver_main: None,
            hle_service: None,
        }
    }
}

impl Deref for Cpu {
    type Target = Core;
    fn deref(&self) -> &Core {
        &self.core
    }
}

impl DerefMut for Cpu {
    fn deref_mut(&mut self) -> &mut Core {
        &mut self.core
    }
}
