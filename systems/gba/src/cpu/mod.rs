//! GBA CPU state and the scheduling seam around the ARM7TDMI core.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus, InstructionFetchRegion};
use evenfall_cpu_arm::{Core, Pipeline};
use evenfall_state::Codec;
use std::ops::{Deref, DerefMut};

pub use evenfall_cpu_arm::{ExecuteResult, InstructionSet};

impl From<evenfall_cpu_arm::AccessWidth> for AccessWidth {
    fn from(width: evenfall_cpu_arm::AccessWidth) -> Self {
        match width {
            evenfall_cpu_arm::AccessWidth::Byte => Self::Byte,
            evenfall_cpu_arm::AccessWidth::Half => Self::Half,
            evenfall_cpu_arm::AccessWidth::Word => Self::Word,
        }
    }
}

impl From<evenfall_cpu_arm::AccessKind> for AccessKind {
    fn from(kind: evenfall_cpu_arm::AccessKind) -> Self {
        match kind {
            evenfall_cpu_arm::AccessKind::Nonsequential => Self::Nonsequential,
            evenfall_cpu_arm::AccessKind::Sequential => Self::Sequential,
        }
    }
}

impl evenfall_cpu_arm::Bus for Bus {
    #[inline]
    fn read_cpu_data(
        &mut self,
        address: u32,
        width: evenfall_cpu_arm::AccessWidth,
        kind: evenfall_cpu_arm::AccessKind,
    ) -> u32 {
        self.read_cpu_data(address, width.into(), kind.into())
    }

    #[inline]
    fn write_cpu_data(
        &mut self,
        address: u32,
        width: evenfall_cpu_arm::AccessWidth,
        kind: evenfall_cpu_arm::AccessKind,
        value: u32,
    ) {
        self.write_cpu_data(
            Access {
                address,
                width: width.into(),
                kind: kind.into(),
                source: AccessSource::CpuData,
            },
            value,
        );
    }

    #[inline]
    fn elapse_cpu_internal(&mut self, cycles: evenfall_cpu_arm::Cycle) {
        self.elapse_cpu_internal(cycles);
    }

    #[inline]
    fn cpu_ram_block_cycles(
        &self,
        first: u32,
        beats: u32,
        internal: evenfall_cpu_arm::Cycle,
    ) -> Option<evenfall_cpu_arm::Cycle> {
        self.cpu_ram_block_cycles(first, beats, internal)
    }

    #[inline]
    fn cpu_ram_word(&self, address: u32) -> u32 {
        self.cpu_ram_word(address)
    }

    #[inline]
    fn set_cpu_ram_word(&mut self, address: u32, value: u32) {
        self.set_cpu_ram_word(address, value);
    }

    #[inline]
    fn finish_cpu_ram_load_block(&mut self, cycles: evenfall_cpu_arm::Cycle, last_word: u32) {
        self.finish_cpu_ram_load_block(cycles, last_word);
    }

    #[inline]
    fn finish_cpu_ram_store_block(&mut self, cycles: evenfall_cpu_arm::Cycle) {
        self.finish_cpu_ram_store_block(cycles);
    }
}

pub const HLE_IRQ_RETURN_STUB: u32 = 0x0000_0138;
pub const HLE_CALLBACK_RETURN_STUB: u32 = 0x0000_013c;
pub const HLE_SOUND_JUMP_BASE: u32 = 0x0000_3e00;
pub const HLE_SOUND_JUMP_COUNT: u32 = 36;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepResult {
    pub instruction_address: u32,
    pub instruction: u32,
    pub instruction_set: InstructionSet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitReason {
    Halted,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HleMemoryOperation {
    Read,
    Write,
}

/// A typed stop for guest-controlled behavior ZiGBA cannot safely emulate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultReason {
    Isa(evenfall_cpu_arm::FaultReason),
    UnsupportedHleSwi {
        service: u8,
    },
    InvalidHleMemoryAccess {
        service: u8,
        operation: HleMemoryOperation,
        address: u32,
        width: AccessWidth,
    },
    HleIrqNestingLimit {
        depth: u8,
    },
    SampleFreqSetModeZero {
        mode: u8,
    },
    SoundChannelClearInternalCallback {
        target: u32,
    },
    TrackStopCgbCallback {
        target: u32,
    },
    Mp2kPlayerChainLimit {
        player: u32,
    },
    Mp2kMissingRunningStatus {
        track: u32,
        event: u8,
    },
    Mp2kInternalCallback {
        target: u32,
    },
    Mp2kReplacementCallback {
        command: u8,
        target: u32,
    },
    SoundDriverMainInternalCallback {
        target: u32,
    },
    DmaLivelock {
        channel: u8,
        started_at: u64,
    },
}

impl From<evenfall_cpu_arm::FaultReason> for FaultReason {
    fn from(reason: evenfall_cpu_arm::FaultReason) -> Self {
        Self::Isa(reason)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultStep {
    pub instruction_address: u32,
    pub instruction: u32,
    pub instruction_set: InstructionSet,
    pub reason: FaultReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    Executed(StepResult),
    Waiting(WaitReason),
    Faulted(FaultStep),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HleBranchResult {
    pub branched: bool,
    pub fault: Option<FaultReason>,
}

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

    pub fn read_register(&self, index: usize) -> u32 {
        self.registers[index]
    }

    pub fn next_instruction_address(&self) -> u32 {
        self.core.next_instruction_address
    }

    pub fn read_cpsr(&self) -> u32 {
        self.cpsr
    }

    pub(crate) fn refill_pipeline(&mut self, bus: &mut Bus) {
        self.refill_pipeline_in_region(bus, &mut InstructionFetchRegion::default());
    }

    pub(crate) fn refill_thumb_pipeline_in_region(
        &mut self,
        bus: &mut Bus,
        region: &mut InstructionFetchRegion,
    ) {
        if self.is_thumb() {
            if let Some(words) =
                bus.fused_thumb_game_pak_refill(region, self.core.next_instruction_address)
            {
                self.pipeline =
                    Some(Pipeline { instructions: words, instruction_set: InstructionSet::Thumb });
                self.registers[15] = self.core.next_instruction_address.wrapping_add(4);
                return;
            }
        }
        self.refill_pipeline_in_region(bus, region);
    }

    pub(crate) fn refill_pipeline_in_region(
        &mut self,
        bus: &mut Bus,
        region: &mut InstructionFetchRegion,
    ) {
        if self.is_thumb() {
            self.pipeline = Some(Pipeline {
                instructions: bus
                    .fetch_thumb_refill_in_region(region, self.core.next_instruction_address),
                instruction_set: InstructionSet::Thumb,
            });
            self.registers[15] = self.core.next_instruction_address.wrapping_add(4);
            return;
        }
        bus.fetch_kind = AccessKind::Nonsequential;
        let first = bus.fetch_instruction_in_region(
            region,
            self.core.next_instruction_address,
            AccessWidth::Word,
        );
        bus.fetch_kind = AccessKind::Sequential;
        let second = bus.fetch_instruction_in_region(
            region,
            self.core.next_instruction_address.wrapping_add(4),
            AccessWidth::Word,
        );
        self.pipeline =
            Some(Pipeline { instructions: [first, second], instruction_set: InstructionSet::Arm });
        self.registers[15] = self.core.next_instruction_address.wrapping_add(8);
    }

    pub(crate) fn enter_irq(&mut self, bus: &mut Bus) -> Result<(), FaultReason> {
        if self.hle_bios && usize::from(self.hle_irq_depth) == self.hle_irq_stack.len() {
            return Err(FaultReason::HleIrqNestingLimit { depth: self.hle_irq_depth });
        }
        self.enter_exception(0x12, 0x18, self.core.next_instruction_address.wrapping_add(4));
        if self.hle_bios {
            self.hle_irq_stack[usize::from(self.hle_irq_depth)] = HleIrqFrame {
                return_address: self.registers[14],
                saved_spsr_irq: self.spsr_irq,
                saved_r0_r3: self.registers[..4].try_into().expect("four registers"),
                saved_r12: self.registers[12],
            };
            self.hle_irq_depth += 1;
            let handler = bus
                .read(Access {
                    address: 0x0300_7ffc,
                    width: AccessWidth::Word,
                    kind: AccessKind::Nonsequential,
                    source: AccessSource::CpuData,
                })
                .value;
            self.registers[14] = HLE_IRQ_RETURN_STUB;
            self.irq_r14 = HLE_IRQ_RETURN_STUB;
            if handler & 1 != 0 {
                self.cpsr |= 1 << 5;
                self.core.next_instruction_address = handler & !1;
                self.registers[15] = self.core.next_instruction_address.wrapping_add(4);
            } else {
                self.cpsr &= !(1 << 5);
                self.core.next_instruction_address = handler & !3;
                self.registers[15] = self.core.next_instruction_address.wrapping_add(8);
            }
            bus.set_bios_latch(0xe25e_f004);
        } else {
            debug_assert_eq!(self.core.next_instruction_address, 0x18);
        }
        Ok(())
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
