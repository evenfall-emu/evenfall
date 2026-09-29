//! GBA CPU state and the scheduling seam around the ARM7TDMI core.

use evenfall_cpu_arm::Core;
use std::ops::{Deref, DerefMut};

pub use evenfall_cpu_arm::InstructionSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HleIrqFrame {
    pub return_address: u32,
    pub saved_spsr_irq: u32,
    pub saved_r0_r3: [u32; 4],
    pub saved_r12: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HleSoundDriverMainPhase {
    AfterPlayerCallback,
    AfterMp2kCallback,
    AfterCgbCallback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleSoundDriverMainContinuation {
    pub sound_area: u32,
    pub scanline_limit: u32,
    pub phase: HleSoundDriverMainPhase,
    pub return_address: u32,
    pub return_cpsr: u32,
    pub return_lr: u32,
    pub saved_r4_r11: [u32; 8],
    pub pending_players: [u32; 16],
    pub pending_player_count: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HleMultiBootPhase {
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiBootParameters {
    pub handshake_data: u8,
    pub client_data: [u8; 3],
    pub palette_data: u8,
    pub client_bit: u8,
    pub source: u32,
    pub end: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiBootMode {
    Normal256Khz,
    Multiplayer115Khz,
    Normal2Mhz,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiBootEncoder {
    pub checksum: u32,
    pub key_state: u32,
    pub checksum_polynomial: u32,
    pub key_xor: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleCpuSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub fill_value: u32,
    pub fill: bool,
    pub word: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleCpuFastSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining_blocks: u32,
    pub last_word: u32,
    pub fill: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleBgAffineSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleObjAffineSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub stride: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleLz77Work {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub pending_halfword: u16,
    pub block_header: u8,
    pub blocks_remaining: u8,
    pub vram_output: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleHuffmanWork {
    pub tree_base: u32,
    pub bitstream_address: u32,
    pub destination: u32,
    pub remaining: u32,
    pub node_address: u32,
    pub output: u32,
    pub node: u8,
    pub symbol_bits: u8,
    pub output_bits: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleRunLengthWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub padding: u32,
    pub pending_halfword: u16,
    pub vram_output: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleDifferentialFilterWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub previous: u16,
    pub pending_halfword: u16,
    pub input_halfword: bool,
    pub output_halfword: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HleBulkService {
    pub instruction_address: u32,
    pub instruction: u32,
    pub instruction_set: InstructionSet,
    pub work: HleBulkWork,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HleService {
    MultiBoot(HleMultiBootContinuation),
    Bulk(HleBulkService),
}

/// ZiGBA's single flat `Cpu`: the core prefix precedes all HLE fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpu {
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
