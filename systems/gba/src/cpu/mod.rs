//! GBA CPU state and the scheduling seam around the ARM7TDMI core.

use crate::bus::{
    Access, AccessKind, AccessSource, AccessWidth, ArmIwramExecutionRegion, Bus,
    InstructionFetchRegion, PowerState, ThumbRomRun, ThumbStream, begin_arm_iwram_slice,
    fetch_arm_iwram_in_slice, is_arm_iwram_slice_fetch,
};
pub use crate::multiboot::{
    Encoder as MultiBootEncoder, Mode as MultiBootMode, Parameters as MultiBootParameters,
};
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

impl evenfall_cpu_arm::Bus for ArmIwramExecutionRegion<'_> {
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
        self.write(
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
    fn elapse_cpu_internal(&mut self, cycles: u64) {
        self.elapse_cpu_internal(cycles);
    }

    // The ARM kernel's block preflight admits RAM, but the shared executor
    // uses its ordinary per-beat path rather than a second optimized path.
    fn cpu_ram_word(&self, _address: u32) -> u32 {
        unreachable!("the ARM region does not admit a fast block")
    }
    fn set_cpu_ram_word(&mut self, _address: u32, _value: u32) {
        unreachable!("the ARM region does not admit a fast block")
    }
    fn finish_cpu_ram_load_block(&mut self, _cycles: u64, _last_word: u32) {
        unreachable!("the ARM region does not admit a fast block")
    }
    fn finish_cpu_ram_store_block(&mut self, _cycles: u64) {
        unreachable!("the ARM region does not admit a fast block")
    }
}

impl evenfall_cpu_arm::Bus for ThumbRomRun<'_> {
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
        _kind: evenfall_cpu_arm::AccessKind,
        value: u32,
    ) {
        self.write_cpu_data(
            Access {
                address,
                width: width.into(),
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            },
            value,
        );
    }

    #[inline]
    fn elapse_cpu_internal(&mut self, cycles: u64) {
        self.elapse_cpu_internal(cycles);
    }

    fn cpu_ram_block_cycles(&self, first: u32, beats: u32, internal: u64) -> Option<u64> {
        self.cpu_ram_block_cycles(first, beats, internal)
    }

    fn cpu_ram_word(&self, address: u32) -> u32 {
        self.cpu_ram_word(address)
    }

    fn set_cpu_ram_word(&mut self, address: u32, value: u32) {
        self.set_cpu_ram_word(address, value);
    }

    fn finish_cpu_ram_load_block(&mut self, cycles: u64, last_word: u32) {
        self.finish_cpu_ram_load_block(cycles, last_word);
    }

    fn finish_cpu_ram_store_block(&mut self, cycles: u64) {
        self.finish_cpu_ram_store_block(cycles);
    }
}

struct ArmKernelState<'a> {
    region: ArmIwramExecutionRegion<'a>,
    pc8: u32,
    current: u32,
    next: u32,
    limit: u64,
}

impl ArmKernelState<'_> {
    #[inline]
    fn fits(&self, required: u64) -> bool {
        self.region.cycle <= self.limit && self.limit - self.region.cycle >= required
    }

    #[inline]
    fn bound(&mut self) {
        let words = u64::from((0x0400_0000 - self.pc8) / 4);
        self.limit = self.region.end.min(self.region.cycle + words);
    }

    #[inline]
    fn advance(&mut self, cpu: &mut Cpu, fetched: u32) {
        self.current = self.next;
        self.next = fetched;
        self.pc8 = self.pc8.wrapping_add(4);
        cpu.registers[15] = self.pc8;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkOutcome {
    Deadline,
    Generic,
    Faulted,
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

impl HleBulkWork {
    pub fn service(&self) -> u8 {
        match self {
            Self::CpuSet(_) => 0x0b,
            Self::CpuFastSet(_) => 0x0c,
            Self::BgAffineSet(_) => 0x0e,
            Self::ObjAffineSet(_) => 0x0f,
            Self::BitUnpack(_) => 0x10,
            Self::Lz77(work) => {
                if work.vram_output {
                    0x12
                } else {
                    0x11
                }
            }
            Self::Huffman(_) => 0x13,
            Self::RunLength(work) => {
                if work.vram_output {
                    0x15
                } else {
                    0x14
                }
            }
            Self::DifferentialFilter(work) => {
                if work.input_halfword {
                    0x18
                } else if work.output_halfword {
                    0x17
                } else {
                    0x16
                }
            }
        }
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::CpuSet(work) => {
                work.remaining <= 0x001f_ffff
                    && (work.source | work.destination) & (if work.word { 3 } else { 1 }) == 0
            }
            Self::CpuFastSet(work) => {
                work.remaining_blocks <= 0x0004_0000 && (work.source | work.destination) & 3 == 0
            }
            Self::BgAffineSet(work) => {
                !bg_affine_set_mmio_fault(work.source, work.destination, work.remaining)
            }
            Self::ObjAffineSet(_) => true,
            Self::BitUnpack(work) => {
                matches!(work.source_width, 1 | 2 | 4 | 8)
                    && matches!(work.destination_width, 1 | 2 | 4 | 8 | 16 | 32)
                    && work.input_bits_remaining <= 8
                    && work.input_bits_remaining % work.source_width == 0
                    && work.output_bits < 32
                    && work.output_bits % work.destination_width == 0
            }
            Self::Lz77(work) => work.blocks_remaining <= 8,
            Self::Huffman(work) => {
                matches!(work.symbol_bits, 4 | 8)
                    && work.output_bits < 32
                    && work.output_bits % work.symbol_bits == 0
            }
            Self::RunLength(work) => work.padding <= 3,
            Self::DifferentialFilter(work) => work.output_halfword || !work.input_halfword,
        }
    }
}

fn bg_affine_set_mmio_fault(source: u32, destination: u32, count: u32) -> bool {
    // ZiGBA's four record streams, in per-record bus access order.
    for (base, stride) in [
        (source, 20),
        (source.wrapping_add(4), 20),
        (destination.wrapping_add(8), 16),
        (destination.wrapping_add(12), 16),
    ] {
        if base & 3 == 0 {
            continue;
        }
        let record = if base >> 24 == 4 {
            0
        } else {
            u64::from(0x0400_0000u32.wrapping_sub(base)).div_ceil(stride)
        };
        if record < u64::from(count) {
            return true;
        }
    }
    false
}

fn instruction_address_aligned(address: u32, instruction_set: InstructionSet) -> bool {
    address & (if instruction_set == InstructionSet::Thumb { 1 } else { 3 }) == 0
}

fn valid_hle_return_state(address: u32, cpsr: u32) -> bool {
    evenfall_cpu_arm::ProcessorMode::parse(cpsr).is_some()
        && instruction_address_aligned(
            address,
            if cpsr & (1 << 5) != 0 { InstructionSet::Thumb } else { InstructionSet::Arm },
        )
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

    pub fn write_state(
        &self,
        sink: &mut evenfall_state::Sink<'_>,
        current_cycle: u64,
    ) -> evenfall_state::Result<()> {
        self.validate_state(current_cycle)?;
        let mut canonical = self.clone();
        canonical.hle_irq_stack[usize::from(canonical.hle_irq_depth)..]
            .fill(HleIrqFrame::default());
        if let Some(continuation) = &mut canonical.hle_sound_driver_main {
            continuation.pending_players[usize::from(continuation.pending_player_count)..].fill(0);
        }
        canonical.encode(sink)
    }

    pub fn read_state(
        &mut self,
        source: &mut evenfall_state::Source<'_>,
        current_cycle: u64,
    ) -> evenfall_state::Result<()> {
        self.decode(source)?;
        self.validate_state(current_cycle)
    }

    pub fn validate_state(&self, current_cycle: u64) -> evenfall_state::Result<()> {
        use evenfall_state::Error::InvalidSaveState as Invalid;
        if evenfall_cpu_arm::ProcessorMode::parse(self.cpsr).is_none()
            || usize::from(self.hle_irq_depth) > self.hle_irq_stack.len()
        {
            return Err(Invalid);
        }
        let set = if self.is_thumb() { InstructionSet::Thumb } else { InstructionSet::Arm };
        if !instruction_address_aligned(self.core.next_instruction_address, set) {
            return Err(Invalid);
        }
        if !self.hle_bios
            && (self.hle_irq_depth != 0
                || self.hle_interrupt_wait_mask.is_some()
                || self.hle_sound_channel_clear.is_some()
                || self.hle_sound_driver_main.is_some()
                || self.hle_service.is_some())
        {
            return Err(Invalid);
        }
        for frame in &self.hle_irq_stack[..usize::from(self.hle_irq_depth)] {
            if !valid_hle_return_state(frame.return_address, frame.saved_spsr_irq) {
                return Err(Invalid);
            }
        }
        if let Some(continuation) = &self.hle_sound_channel_clear
            && (!(1..=4).contains(&continuation.channel)
                || continuation.identifier != 0x6873_6d53
                || !valid_hle_return_state(continuation.return_address, continuation.return_cpsr))
        {
            return Err(Invalid);
        }
        if let Some(continuation) = &self.hle_sound_driver_main {
            let count = usize::from(continuation.pending_player_count);
            if count > continuation.pending_players.len()
                || !valid_hle_return_state(continuation.return_address, continuation.return_cpsr)
                || (continuation.phase == HleSoundDriverMainPhase::AfterMp2kCallback)
                    == (count == 0)
            {
                return Err(Invalid);
            }
        }
        match &self.hle_service {
            Some(HleService::MultiBoot(continuation)) => {
                let p = &continuation.parameters;
                p.validate().map_err(|_| Invalid)?;
                let length = p.payload_length();
                if continuation.source_offset > length
                    || continuation.source_offset & 3 != 0
                    || continuation.wait_started_at > current_cycle
                    || !instruction_address_aligned(
                        continuation.instruction_address,
                        continuation.instruction_set,
                    )
                    || (continuation.multiplayer_high_half && !continuation.mode.is_multiplayer())
                {
                    return Err(Invalid);
                }
                let multiplayer = continuation.mode.is_multiplayer();
                if continuation.encoder.checksum_polynomial
                    != if multiplayer { 0xa517 } else { 0xc37b }
                    || continuation.encoder.key_xor
                        != if multiplayer { 0x6465_646f } else { 0x4320_2f2f }
                {
                    return Err(Invalid);
                }
            }
            Some(HleService::Bulk(bulk)) => {
                let width = if bulk.instruction_set == InstructionSet::Thumb { 2 } else { 4 };
                let service = match bulk.instruction_set {
                    InstructionSet::Thumb if bulk.instruction & 0xffff_ff00 == 0xdf00 => {
                        bulk.instruction as u8
                    }
                    InstructionSet::Arm if bulk.instruction & 0x0f00_0000 == 0x0f00_0000 => {
                        (bulk.instruction >> 16) as u8
                    }
                    _ => return Err(Invalid),
                };
                if bulk.instruction_set != set
                    || !instruction_address_aligned(bulk.instruction_address, bulk.instruction_set)
                    || self.core.next_instruction_address
                        != bulk.instruction_address.wrapping_add(width)
                    || service != bulk.work.service()
                    || !bulk.work.valid()
                {
                    return Err(Invalid);
                }
            }
            None => {}
        }
        if self.pipeline.is_some_and(|pipeline| pipeline.instruction_set != set) {
            return Err(Invalid);
        }
        Ok(())
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
        if self.is_thumb()
            && let Some(words) =
                bus.fused_thumb_game_pak_refill(region, self.core.next_instruction_address)
        {
            self.pipeline =
                Some(Pipeline { instructions: words, instruction_set: InstructionSet::Thumb });
            self.registers[15] = self.core.next_instruction_address.wrapping_add(4);
            return;
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
        let return_address = self.core.next_instruction_address.wrapping_add(4);
        self.enter_exception(0x12, 0x18, return_address);
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

    pub fn inside_hle_bulk_service(&self) -> bool {
        matches!(self.hle_service, Some(HleService::Bulk(_)))
    }

    pub fn step(&mut self, bus: &mut Bus) -> StepOutcome {
        self.step_impl(bus, true).expect("the reporting step always yields an outcome")
    }

    pub fn run_step(
        &mut self,
        bus: &mut Bus,
        execution_fault: &mut Option<FaultReason>,
    ) -> Option<StepOutcome> {
        let outcome = self.step_impl(bus, false);
        if let Some(StepOutcome::Faulted(fault)) = outcome {
            *execution_fault = Some(fault.reason);
        }
        outcome
    }

    fn step_impl(&mut self, bus: &mut Bus, report_instruction: bool) -> Option<StepOutcome> {
        // Bulk continuations ignore HALTCNT writes made by their own accesses.
        if !self.inside_hle_bulk_service() {
            let wait = match bus.power_state() {
                PowerState::Running => None,
                PowerState::Halted if !bus.halt_wake_pending() => Some(WaitReason::Halted),
                PowerState::Stopped if !bus.stop_wake_pending() => Some(WaitReason::Stopped),
                _ => {
                    bus.resume_power();
                    if bus.irq_line && self.cpsr & (1 << 7) == 0 {
                        if let Err(reason) = self.enter_irq(bus) {
                            return Some(StepOutcome::Faulted(self.boundary_fault(bus, reason)));
                        }
                        self.refill_pipeline(bus);
                    }
                    None
                }
            };
            if let Some(wait) = wait {
                return Some(StepOutcome::Waiting(wait));
            }
        }

        if self.hle_service.is_some() || self.core.next_instruction_address < 0x4000 {
            if self.inside_hle_bulk_service() {
                return self.continue_hle_bulk_service(bus, report_instruction);
            }
            if self.hle_irq_depth != 0 && self.core.next_instruction_address == HLE_IRQ_RETURN_STUB
            {
                return Some(StepOutcome::Executed(self.complete_hle_irq(bus)));
            }
            if self.hle_service.is_some() {
                return Some(StepOutcome::Executed(self.continue_hle_multi_boot(bus)));
            }
            if self.core.next_instruction_address == HLE_CALLBACK_RETURN_STUB {
                if self.hle_sound_channel_clear.is_some() {
                    return Some(self.continue_hle_sound_channel_clear(bus));
                }
                if self.hle_sound_driver_main.is_some() {
                    return Some(self.continue_hle_sound_driver_main(bus));
                }
            }
            if self.hle_bios
                && let Some(index) = self.hle_sound_jump_index()
            {
                return Some(self.execute_hle_sound_jump(bus, index));
            }
        }

        let address = self.core.next_instruction_address;
        let set = if self.is_thumb() { InstructionSet::Thumb } else { InstructionSet::Arm };
        let width = if self.is_thumb() { AccessWidth::Half } else { AccessWidth::Word };
        let instruction_width: u32 = if self.is_thumb() { 2 } else { 4 };
        let instruction = if let Some(pipeline) = self.pipeline.as_mut() {
            debug_assert_eq!(pipeline.instruction_set, set);
            let instruction = pipeline.instructions[0];
            pipeline.instructions[0] = pipeline.instructions[1];
            pipeline.instructions[1] =
                bus.fetch_instruction(address.wrapping_add(2 * instruction_width), width);
            instruction
        } else {
            bus.fetch_instruction(address, width)
        };
        let execution: Result<ExecuteResult, FaultReason> = if self.is_thumb() {
            Ok(self.core.execute_thumb(bus, instruction as u16))
        } else {
            self.core.execute_arm(bus, instruction).map_err(FaultReason::from)
        };
        let execution = execution.and_then(|mut result| {
            if let Some(service) = result.software_interrupt {
                if self.hle_bios {
                    result = self.execute_hle_software_interrupt(bus, instruction, service)?;
                } else {
                    self.enter_software_interrupt(address.wrapping_add(instruction_width));
                    result.branched = true;
                }
            }
            Ok(result)
        });
        let execution = match execution {
            Ok(result) => result,
            Err(reason) => {
                return Some(StepOutcome::Faulted(FaultStep {
                    instruction_address: address,
                    instruction,
                    instruction_set: set,
                    reason,
                }));
            }
        };
        if !execution.branched {
            self.core.next_instruction_address =
                self.core.next_instruction_address.wrapping_add(instruction_width);
        }

        if execution.branched {
            self.refill_pipeline(bus);
        } else if self.hle_service.is_none() && bus.irq_line && self.cpsr & (1 << 7) == 0 {
            bus.resume_power();
            if let Err(reason) = self.enter_irq(bus) {
                self.registers[15] = self
                    .core
                    .next_instruction_address
                    .wrapping_add(if self.is_thumb() { 4 } else { 8 });
                return Some(StepOutcome::Faulted(FaultStep {
                    instruction_address: address,
                    instruction,
                    instruction_set: set,
                    reason,
                }));
            }
            self.refill_pipeline(bus);
        } else {
            self.registers[15] = self
                .core
                .next_instruction_address
                .wrapping_add(if self.is_thumb() { 4 } else { 8 });
        }
        report_instruction.then_some(StepOutcome::Executed(StepResult {
            instruction_address: address,
            instruction,
            instruction_set: set,
        }))
    }

    /// Names the instruction at a fault boundary without charging a bus access.
    pub fn boundary_fault(&self, bus: &Bus, reason: FaultReason) -> FaultStep {
        if let Some(HleService::Bulk(bulk)) = &self.hle_service {
            return FaultStep {
                instruction_address: bulk.instruction_address,
                instruction: bulk.instruction,
                instruction_set: bulk.instruction_set,
                reason,
            };
        }
        let instruction_set =
            if self.is_thumb() { InstructionSet::Thumb } else { InstructionSet::Arm };
        let instruction = self.pipeline.map_or_else(
            || {
                let mut value = 0;
                for offset in 0..if self.is_thumb() { 2 } else { 4 } {
                    value |= u32::from(
                        bus.inspect_byte(self.core.next_instruction_address.wrapping_add(offset)),
                    ) << (offset * 8);
                }
                value
            },
            |pipeline| pipeline.instructions[0],
        );
        FaultStep {
            instruction_address: self.core.next_instruction_address,
            instruction,
            instruction_set,
            reason,
        }
    }

    /// A whole-instruction preflight: fetch first, then all data and internal
    /// cycles, and finally any refill. A failed condition still consumes its
    /// fetch, even if the encoding would otherwise leave the kernel.
    fn arm_kernel_cycles(
        &self,
        state: &mut ArmKernelState<'_>,
        instruction: u32,
        condition_passed: bool,
    ) -> Option<u64> {
        if instruction & 0x0f00_0000 == 0x0f00_0000 {
            return None; // SWIs always leave through step_impl.
        }
        if !condition_passed {
            return Some(1);
        }
        let rn = ((instruction >> 16) & 15) as usize;
        let rd = ((instruction >> 12) & 15) as usize;
        let load = instruction & (1 << 20) != 0;
        match (instruction >> 25) & 7 {
            0 if instruction & 0x0fc0_00f0 == 0x0000_0090 => {
                let rm = (instruction & 15) as usize;
                let rs = ((instruction >> 8) & 15) as usize;
                let accumulate = instruction & (1 << 21) != 0;
                // MUL's Rd is bits 19..16 and its accumulator Rn is 15..12;
                // the transfer decoder's `rd`/`rn` names are reversed here.
                if rn == 15 || rs == 15 || rm == 15 || (accumulate && rd == 15) {
                    return None;
                }
                Some(
                    1 + Core::multiply_iterations(self.registers[rs], true) + u64::from(accumulate),
                )
            }
            0 if instruction & 0x0e00_0090 == 0x0000_0090 && (instruction >> 5) & 3 != 0 => {
                let operation = (instruction >> 5) & 3;
                if rd == 15
                    || (rn == 15 && (instruction & (1 << 24) == 0 || instruction & (1 << 21) != 0))
                    || (instruction & (1 << 22) == 0 && instruction & 15 == 15)
                    || (!load && operation != 1)
                {
                    return None;
                }
                let base = self.registers[rn];
                let offset = if instruction & (1 << 22) != 0 {
                    ((instruction >> 8) & 15) << 4 | (instruction & 15)
                } else {
                    self.registers[(instruction & 15) as usize]
                };
                let adjusted = if instruction & (1 << 23) != 0 {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let address = if instruction & (1 << 24) != 0 { adjusted } else { base };
                let width = if operation == 2 { AccessWidth::Byte } else { AccessWidth::Half };
                let bus_address = if width == AccessWidth::Byte { address } else { address & !1 };
                let cycles = if load {
                    state.region.load_cycles(bus_address, width)
                } else {
                    ArmIwramExecutionRegion::ram_cycles(bus_address, width)
                }?;
                Some(1 + cycles + u64::from(load))
            }
            0 | 1
                if rd != 15
                    && (instruction & 0x90 != 0x90 || instruction & (1 << 25) != 0)
                    && (instruction & (1 << 25) != 0 || instruction & 0x0e00_0010 == 0)
                    && (!((8..=11).contains(&((instruction >> 21) & 15))) || load)
                    && instruction & 0x0fbf_0000 != 0x010f_0000 =>
            {
                Some(1)
            }
            2 | 3 => {
                let pre = instruction & (1 << 24) != 0;
                if rd == 15
                    || (rn == 15 && (!pre || instruction & (1 << 21) != 0))
                    || instruction & 0x0e00_0010 == 0x0600_0010
                {
                    return None;
                }
                let base = self.registers[rn];
                let offset = if instruction & (1 << 25) != 0 {
                    self.core.decode_operand2(instruction, false).value
                } else {
                    instruction & 0xfff
                };
                let adjusted = if instruction & (1 << 23) != 0 {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let address = if pre { adjusted } else { base };
                let byte = instruction & (1 << 22) != 0;
                let width = if byte { AccessWidth::Byte } else { AccessWidth::Word };
                let bus_address = if byte { address } else { address & !3 };
                let cycles = if load {
                    state.region.load_cycles(bus_address, width)
                } else {
                    ArmIwramExecutionRegion::ram_cycles(bus_address, width)
                }?;
                Some(1 + cycles + u64::from(load))
            }
            4 => {
                let list = instruction as u16;
                if list == 0 || list & 0x8000 != 0 || rn == 15 {
                    return None;
                }
                let count = list.count_ones();
                let base = self.registers[rn];
                let up = instruction & (1 << 23) != 0;
                let pre = instruction & (1 << 24) != 0;
                let first = if up {
                    base.wrapping_add(if pre { 4 } else { 0 })
                } else {
                    base.wrapping_sub(count * 4).wrapping_add(if pre { 0 } else { 4 })
                } & !3;
                let last = first.wrapping_add((count - 1) * 4);
                if first >> 24 != last >> 24 {
                    return None;
                }
                Some(
                    1 + u64::from(count)
                        * ArmIwramExecutionRegion::ram_cycles(first, AccessWidth::Word)?
                        + u64::from(load),
                )
            }
            5 => {
                let displacement = (((instruction & 0x00ff_ffff) << 8) as i32 >> 6) as u32;
                let target = state.pc8.wrapping_add(displacement);
                (target >= 0x0300_0000 && is_arm_iwram_slice_fetch(target.wrapping_add(8)))
                    .then_some(3)
            }
            _ => None,
        }
    }

    /// Local IWRAM supply and clock stay inside one event-free device batch.
    /// The refused instruction remains untouched for the ordinary runner.
    fn run_arm_iwram_kernel(&mut self, bus: &mut Bus, yield_at: u64) -> bool {
        if bus.irq_line && self.cpsr & (1 << 7) == 0 {
            return false;
        }
        let Some(region) = ArmIwramExecutionRegion::begin(bus, yield_at) else {
            return false;
        };
        let pipeline = self.pipeline.expect("ordinary ARM runner has a pipeline");
        let mut state = ArmKernelState {
            region,
            pc8: self.core.next_instruction_address.wrapping_add(8),
            current: pipeline.instructions[0],
            next: pipeline.instructions[1],
            limit: 0,
        };
        let started = state.region.cycle;
        if is_arm_iwram_slice_fetch(state.pc8) {
            state.bound();
            loop {
                let instruction = state.current;
                let condition_passed = self.core.condition_passed(instruction >> 28);
                let Some(required) =
                    self.arm_kernel_cycles(&mut state, instruction, condition_passed)
                else {
                    break;
                };
                if !state.fits(required) {
                    break;
                }
                let fetched = state.region.fetch(state.pc8);
                if !condition_passed {
                    state.advance(self, fetched);
                    continue;
                }
                // The ordinary step exposes the waiting instruction address
                // throughout execution. In particular BL derives LR from it;
                // after a fall-through the kernel's Core otherwise still held
                // the address from the prior instruction.
                self.core.next_instruction_address = state.pc8.wrapping_sub(8);
                let execution = self
                    .core
                    .execute_arm_condition_passed(&mut state.region, instruction)
                    .expect("an admitted ARM instruction cannot fault");
                if execution.branched {
                    let target = self.core.next_instruction_address;
                    state.pc8 = target;
                    state.current = state.region.fetch(state.pc8);
                    state.pc8 = state.pc8.wrapping_add(4);
                    state.next = state.region.fetch(state.pc8);
                    state.pc8 = state.pc8.wrapping_add(4);
                    self.registers[15] = state.pc8;
                    state.bound();
                } else {
                    debug_assert!(execution.software_interrupt.is_none());
                    if (instruction >> 25) & 7 == 4
                        || (instruction >> 25) & 6 == 2
                        || (instruction & 0x0e00_0090 == 0x0000_0090 && (instruction >> 5) & 3 != 0)
                    {
                        state.region.complete_data();
                    }
                    state.advance(self, fetched);
                }
            }
        }
        self.core.next_instruction_address = state.pc8.wrapping_sub(8);
        self.pipeline = Some(Pipeline {
            instructions: [state.current, state.next],
            instruction_set: InstructionSet::Arm,
        });
        let ran = state.region.cycle != started;
        state.region.finish(state.next);
        ran
    }

    /// The next Thumb instruction's work after its successor fetch. Refuse
    /// accesses whose timing or target cannot be represented by ThumbRomRun.
    /// In particular, an empty register list and BX to ARM stay with the
    /// ordinary interpreter rather than relaxing the ROM run's admission.
    fn thumb_kernel_work(
        &self,
        run: &ThumbRomRun<'_>,
        instruction: u16,
    ) -> Option<(u64, Option<u32>, bool)> {
        let high = instruction >> 8;
        let pc = self.core.next_instruction_address;
        let mut branch = None;
        let mut data = false;
        let cycles = match high {
            0x00..=0x3f | 0xa0..=0xb0 | 0xf0..=0xf7 => 0,
            0x40..=0x43 => {
                let opcode = (instruction >> 6) & 15;
                match opcode {
                    2 | 3 | 4 | 7 => 1,
                    13 => {
                        Core::multiply_iterations(self.registers[(instruction & 7) as usize], true)
                    }
                    _ => 0,
                }
            }
            0x44..=0x47 => {
                let operation = (instruction >> 8) & 3;
                let rd = ((instruction & 7) | ((instruction >> 4) & 8)) as usize;
                let source = self.registers[((instruction >> 3) & 15) as usize];
                if operation == 3 {
                    if source & 1 == 0 {
                        return None;
                    }
                    branch = Some(source & !1);
                } else if rd == 15 && operation != 1 {
                    branch = Some(
                        if operation == 0 {
                            pc.wrapping_add(4).wrapping_add(source)
                        } else {
                            source
                        } & !1,
                    );
                }
                0
            }
            0x48..=0x9f => {
                let (address, width, load) = match high {
                    0x48..=0x4f => (
                        (pc.wrapping_add(4) & !3).wrapping_add(u32::from(instruction & 255) << 2),
                        AccessWidth::Word,
                        true,
                    ),
                    0x50..=0x5f => {
                        let operation = (instruction >> 9) & 7;
                        let address = self.registers[((instruction >> 3) & 7) as usize]
                            .wrapping_add(self.registers[((instruction >> 6) & 7) as usize]);
                        let width = match operation {
                            0 | 4 => AccessWidth::Word,
                            1 | 5 | 7 => AccessWidth::Half,
                            _ => AccessWidth::Byte,
                        };
                        (address, width, operation >= 3)
                    }
                    0x60..=0x7f => {
                        let byte = instruction & (1 << 12) != 0;
                        let address = self.registers[((instruction >> 3) & 7) as usize]
                            .wrapping_add(
                                u32::from((instruction >> 6) & 31) << if byte { 0 } else { 2 },
                            );
                        (
                            address,
                            if byte { AccessWidth::Byte } else { AccessWidth::Word },
                            instruction & (1 << 11) != 0,
                        )
                    }
                    0x80..=0x8f => (
                        self.registers[((instruction >> 3) & 7) as usize]
                            .wrapping_add(u32::from((instruction >> 6) & 31) << 1),
                        AccessWidth::Half,
                        instruction & (1 << 11) != 0,
                    ),
                    _ => (
                        self.registers[13].wrapping_add(u32::from(instruction & 255) << 2),
                        AccessWidth::Word,
                        instruction & (1 << 11) != 0,
                    ),
                };
                let aligned = match width {
                    AccessWidth::Byte => address,
                    AccessWidth::Half => address & !1,
                    AccessWidth::Word => address & !3,
                };
                data = true;
                if load {
                    run.load_cycles(aligned, width)? + 1
                } else {
                    ThumbRomRun::store_cycles(aligned, width)?
                }
            }
            0xb4 | 0xb5 | 0xbc | 0xbd | 0xc0..=0xcf => {
                let pop = instruction & (1 << 11) != 0;
                let stack = high < 0xc0;
                let count = (instruction as u8).count_ones()
                    + u32::from(stack && instruction & (1 << 8) != 0);
                if count == 0 {
                    return None;
                }
                let base =
                    self.registers[if stack { 13 } else { ((instruction >> 8) & 7) as usize }];
                let start = if stack && !pop { base.wrapping_sub(count * 4) } else { base };
                let cycles = ThumbRomRun::block_cycles(start & !3, count, u64::from(pop))?;
                if stack && pop && instruction & (1 << 8) != 0 {
                    branch = Some(run.cpu_ram_word(start.wrapping_add((count - 1) * 4) & !3) & !1);
                }
                data = true;
                cycles
            }
            0xd0..=0xdd => {
                if self.core.condition_passed(u32::from(high & 15)) {
                    let offset = ((i32::from(instruction as u8 as i8)) << 1) as u32;
                    branch = Some(pc.wrapping_add(4).wrapping_add(offset));
                }
                0
            }
            0xe0..=0xe7 => {
                let offset = (((u32::from(instruction & 0x7ff) << 21) as i32) >> 20) as u32;
                branch = Some(pc.wrapping_add(4).wrapping_add(offset));
                0
            }
            0xf8..=0xff => {
                branch =
                    Some(self.registers[14].wrapping_add(u32::from(instruction & 0x7ff) << 1) & !1);
                0
            }
            // SWI and ARMv4T undefined instructions must enter via the
            // ordinary step (which also owns HLE and exception refills).
            _ => return None,
        };
        Some((cycles, branch, data))
    }

    /// Bounded ROM execution with a runtime high-byte dispatch instead of
    /// Zig's 49 comptime-labelled handler classes. The shared ISA executor
    /// remains the semantics source of truth; this loop only admits work.
    fn run_thumb_rom_kernel(
        &mut self,
        bus: &mut Bus,
        view: &mut InstructionFetchRegion,
        yield_at: u64,
    ) -> usize {
        if bus.irq_line && self.cpsr & (1 << 7) == 0 {
            return 0;
        }
        let pipeline = self.pipeline.expect("ordinary Thumb runner has a pipeline");
        if pipeline.instruction_set != InstructionSet::Thumb
            || pipeline.instructions.iter().any(|&word| word > 0xffff)
        {
            return 0;
        }
        let mut pc = self.core.next_instruction_address;
        let Some(mut run) = ThumbRomRun::begin(bus, view, pc, yield_at) else {
            return 0;
        };
        let started = run.cycle;
        let mut executed = 0;
        let mut waiting = pipeline.instructions[0] as u16;
        let mut next = pipeline.instructions[1] as u16;
        loop {
            let saved_cycle = run.cycle;
            let saved_ready = run.head_ready;
            let fetched = run.fetch(pc.wrapping_add(4)) as u16;
            let work = self.thumb_kernel_work(&run, waiting);
            let admitted = work.and_then(|(cycles, branch, data)| {
                if !run.fits(cycles) {
                    return None;
                }
                if let Some(target) = branch {
                    if !run.admits(target) {
                        return None;
                    }
                    // Predict the two refill fetches without touching the
                    // run or CPU; a refusal leaves the waiting opcode intact.
                    let mut clock = run.cycle + cycles;
                    let mut ready = run.head_ready;
                    let offset = target.wrapping_sub(pc.wrapping_add(6));
                    if offset != 0 {
                        let lands = ready.max(clock - run.fill) + u64::from(offset / 2) * run.duty;
                        ready = if offset < 16 && lands > clock && lands - run.duty <= clock {
                            lands
                        } else {
                            clock + run.miss
                        };
                    }
                    for _ in 0..2 {
                        let now = clock;
                        clock = (now + 1).max(ready);
                        ready = ready.max(now - run.fill) + run.duty;
                    }
                    if clock > run.bound {
                        return None;
                    }
                }
                Some((branch, data))
            });
            let Some((branch, data)) = admitted else {
                run.cycle = saved_cycle;
                run.head_ready = saved_ready;
                break;
            };
            self.registers[15] = pc.wrapping_add(4);
            let execution = self.core.execute_thumb(&mut run, waiting);
            executed += 1;
            debug_assert!(execution.software_interrupt.is_none());
            debug_assert_eq!(execution.branched, branch.is_some());
            if data {
                run.complete_data(
                    waiting & (1 << 11) != 0
                        || (waiting >> 8 & 0xf8) == 0x48
                        || ((waiting >> 8) & 0xf0) == 0x50 && (waiting >> 9) & 7 >= 3,
                );
            }
            if let Some(target) = branch {
                debug_assert_eq!(self.core.next_instruction_address, target);
                let words = run.refill(pc.wrapping_add(6), target);
                waiting = words[0] as u16;
                next = words[1] as u16;
                pc = target;
                run.enter(pc);
            } else {
                pc = pc.wrapping_add(2);
                self.core.next_instruction_address = pc;
                waiting = next;
                next = fetched;
            }
            self.registers[15] = pc.wrapping_add(4);
        }
        if executed == 0 {
            debug_assert_eq!(run.cycle, started);
            return 0;
        }
        self.core.next_instruction_address = pc;
        self.pipeline = Some(Pipeline {
            instructions: [u32::from(waiting), u32::from(next)],
            instruction_set: InstructionSet::Thumb,
        });
        run.finish(pc, u32::from(next));
        executed
    }

    /// Ordinary instruction loop; HLE, power transitions and SWIs return to
    /// the generic step without touching the waiting instruction.
    pub fn run_normal_until(
        &mut self,
        bus: &mut Bus,
        yield_at: u64,
        execution_fault: &mut Option<FaultReason>,
        fault_out: &mut Option<FaultStep>,
    ) -> ChunkOutcome {
        if bus.now() >= yield_at {
            return ChunkOutcome::Deadline;
        }
        if self.hle_service.is_some() || self.pipeline.is_none() {
            return ChunkOutcome::Generic;
        }
        let mut region = InstructionFetchRegion::default();
        let mut stream = ThumbStream::default();
        let mut arm_slice = false;
        while bus.now() < yield_at {
            if bus.power_state() != PowerState::Running
                || self.core.next_instruction_address < 0x4000
            {
                return ChunkOutcome::Generic;
            }
            let set = if self.is_thumb() { InstructionSet::Thumb } else { InstructionSet::Arm };
            let width: u32 = if set == InstructionSet::Thumb { 2 } else { 4 };
            let pipeline = self.pipeline.expect("checked before loop");
            if pipeline.instruction_set != set {
                return ChunkOutcome::Generic;
            }
            let address = self.core.next_instruction_address;
            let instruction = pipeline.instructions[0];
            if (set == InstructionSet::Thumb && instruction & 0xff00 == 0xdf00)
                || (set == InstructionSet::Arm && instruction & 0x0f00_0000 == 0x0f00_0000)
            {
                return ChunkOutcome::Generic;
            }
            let arm_iwram =
                set == InstructionSet::Arm && is_arm_iwram_slice_fetch(address.wrapping_add(8));
            if arm_iwram {
                if !arm_slice {
                    begin_arm_iwram_slice(bus);
                    arm_slice = true;
                }
                if self.run_arm_iwram_kernel(bus, yield_at) {
                    if bus.now() >= yield_at {
                        return ChunkOutcome::Deadline;
                    }
                    continue;
                }
            } else if set == InstructionSet::Thumb {
                arm_slice = false;
                if self.run_thumb_rom_kernel(bus, &mut region, yield_at) != 0 {
                    stream.armed = false;
                    if bus.now() >= yield_at {
                        return ChunkOutcome::Deadline;
                    }
                    continue;
                }
            } else {
                arm_slice = false;
            }
            self.pipeline.as_mut().unwrap().instructions[0] = pipeline.instructions[1];
            let fetched = if set == InstructionSet::Thumb {
                bus.fetch_thumb_in_region(&mut region, address.wrapping_add(4), &mut stream)
            } else if arm_iwram {
                fetch_arm_iwram_in_slice(bus, address.wrapping_add(8))
            } else {
                bus.fetch_instruction_in_region(
                    &mut region,
                    address.wrapping_add(8),
                    AccessWidth::Word,
                )
            };
            self.pipeline.as_mut().unwrap().instructions[1] = fetched;
            let execution = if set == InstructionSet::Thumb {
                Ok(self.core.execute_thumb(bus, instruction as u16))
            } else {
                self.core.execute_arm(bus, instruction)
            };
            let execution = match execution {
                Ok(value) => value,
                Err(reason) => {
                    let reason = FaultReason::Isa(reason);
                    *execution_fault = Some(reason);
                    *fault_out = Some(FaultStep {
                        instruction_address: address,
                        instruction,
                        instruction_set: set,
                        reason,
                    });
                    return ChunkOutcome::Faulted;
                }
            };
            debug_assert!(execution.software_interrupt.is_none());
            if !execution.bus_free {
                stream.armed = false;
            }
            if !execution.branched {
                self.core.next_instruction_address = address.wrapping_add(width);
            }
            if execution.branched {
                if set == InstructionSet::Thumb {
                    self.refill_thumb_pipeline_in_region(bus, &mut region);
                } else {
                    self.refill_pipeline_in_region(bus, &mut region);
                }
                stream.armed = false;
                if bus.now() >= yield_at {
                    return ChunkOutcome::Deadline;
                }
                if self.is_thumb() != (set == InstructionSet::Thumb) {
                    return ChunkOutcome::Generic;
                }
            } else if bus.irq_line && self.cpsr & (1 << 7) == 0 {
                bus.resume_power();
                if let Err(reason) = self.enter_irq(bus) {
                    self.registers[15] = self
                        .core
                        .next_instruction_address
                        .wrapping_add(if self.is_thumb() { 4 } else { 8 });
                    *execution_fault = Some(reason);
                    *fault_out = Some(FaultStep {
                        instruction_address: address,
                        instruction,
                        instruction_set: set,
                        reason,
                    });
                    return ChunkOutcome::Faulted;
                }
                self.refill_pipeline_in_region(bus, &mut region);
                return if bus.now() >= yield_at {
                    ChunkOutcome::Deadline
                } else {
                    ChunkOutcome::Generic
                };
            } else {
                self.registers[15] = self.core.next_instruction_address.wrapping_add(2 * width);
            }
        }
        ChunkOutcome::Deadline
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

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_invalid_state(cpu: &Cpu, cycle: u64) {
        use evenfall_state::{Error, Sink, Source};
        assert_eq!(
            cpu.write_state(&mut Sink::new(None, None), cycle),
            Err(Error::InvalidSaveState)
        );
        let mut bytes = Vec::new();
        cpu.encode(&mut Sink::new(Some(&mut bytes), None)).unwrap();
        let mut restored = Cpu::init(0x0800_0000, 0x1f, true);
        assert_eq!(
            restored.read_state(&mut Source::new(&bytes), cycle),
            Err(Error::InvalidSaveState)
        );
    }

    /// ZiGBA `state_codec.writeSchemaName("zigba.cpu.state.v6"); writeSchema(Cpu)`.
    #[test]
    fn cpu_schema_matches_zigba_v6() {
        let mut hash = blake3::Hasher::new();
        Cpu::write_state_schema(&mut evenfall_state::Sink::new(None, Some(&mut hash))).unwrap();
        assert_eq!(
            hash.finalize().to_hex().as_str(),
            "75595acc72ede48e7c4c08b601062ccd2da112163cc71959695a2d85ce93e3c0"
        );
    }

    /// ZiGBA `Cpu.writeState`: an active IRQ frame, fetched opcodes and the
    /// inactive stack tail use the same padding-free bytes as the Zig codec.
    #[test]
    fn cpu_payload_matches_zigba_v6() {
        let mut cpu = Cpu::init(0x0800_0020, 0x1f, true);
        cpu.registers[0] = 0x1234_5678;
        cpu.registers[3] = 0x0200_00a7;
        cpu.pipeline = Some(Pipeline {
            instructions: [0xe3a0_0017, 0xe280_1002],
            instruction_set: InstructionSet::Arm,
        });
        cpu.hle_irq_depth = 1;
        cpu.hle_irq_stack[0] = HleIrqFrame {
            return_address: 0x0800_0018,
            saved_spsr_irq: 0x3f,
            saved_r0_r3: [4, 3, 2, 1],
            saved_r12: 0x55aa_1f00,
        };
        cpu.hle_irq_stack[7] = HleIrqFrame { return_address: 0xdead_beef, ..Default::default() };
        let mut hash = blake3::Hasher::new();
        cpu.write_state(&mut evenfall_state::Sink::new(None, Some(&mut hash)), 64).unwrap();
        assert_eq!(
            hash.finalize().to_hex().as_str(),
            "046107270e9ccccaca6de8dc936670a4dffda7fbe832d5da5f51ca5a21c8bac6"
        );
    }

    /// Zig: CPU state rejects misaligned instruction addresses.
    #[test]
    fn cpu_state_rejects_misaligned_instruction_addresses() {
        let mut arm = Cpu::init(0x0800_0000, 0x1f, true);
        arm.core.next_instruction_address = 0x0800_0002;
        expect_invalid_state(&arm, 0);
        let mut thumb = Cpu::init(0x0800_0000, 0x3f, true);
        thumb.core.next_instruction_address = 0x0800_0001;
        expect_invalid_state(&thumb, 0);
    }

    /// Zig: CPU state rejects HLE continuations under an external BIOS.
    #[test]
    fn cpu_state_rejects_hle_continuations_under_an_external_bios() {
        let mut irq = Cpu::init(0x0800_0000, 0x1f, false);
        irq.hle_irq_depth = 1;
        irq.hle_irq_stack[0] =
            HleIrqFrame { return_address: 0x0800_0004, saved_spsr_irq: 0x1f, ..Default::default() };
        expect_invalid_state(&irq, 0);
        let mut wait = Cpu::init(0x0800_0000, 0x1f, false);
        wait.hle_interrupt_wait_mask = Some(1);
        expect_invalid_state(&wait, 0);
        let mut callback = Cpu::init(0x0800_0000, 0x1f, false);
        callback.hle_sound_channel_clear = Some(HleSoundChannelClearContinuation {
            identifier: 0x6873_6d53,
            channel: 1,
            return_address: 0x0800_0004,
            return_cpsr: 0x1f,
            ..Default::default()
        });
        expect_invalid_state(&callback, 0);
        let mut sound = Cpu::init(0x0800_0000, 0x1f, false);
        sound.hle_sound_driver_main = Some(HleSoundDriverMainContinuation {
            return_address: 0x0800_0004,
            return_cpsr: 0x1f,
            ..Default::default()
        });
        expect_invalid_state(&sound, 0);
        let mut multi_boot = Cpu::init(0x0800_0000, 0x1f, false);
        multi_boot.hle_service = Some(HleService::MultiBoot(multi_boot_continuation()));
        expect_invalid_state(&multi_boot, 0);
    }

    fn multi_boot_continuation() -> HleMultiBootContinuation {
        let parameters = MultiBootParameters {
            handshake_data: 0x9d,
            client_data: [0x12, 0x34, 0x56],
            palette_data: 0x91,
            client_bit: 0x02,
            source: 0x0800_00c0,
            end: 0x0800_01c0,
        };
        HleMultiBootContinuation {
            parameters,
            mode: MultiBootMode::Normal256Khz,
            encoder: MultiBootEncoder::init(
                MultiBootMode::Normal256Khz,
                parameters.palette_data,
                parameters.client_data,
            ),
            instruction_address: 0x0800_0000,
            instruction_set: InstructionSet::Arm,
            ..Default::default()
        }
    }

    /// Zig: CPU state rejects impossible HLE callback continuations.
    #[test]
    fn cpu_state_rejects_impossible_hle_callback_continuations() {
        let channel = HleSoundChannelClearContinuation {
            identifier: 0x6873_6d53,
            channel: 1,
            return_address: 0x0800_0004,
            return_cpsr: 0x1f,
            ..Default::default()
        };
        for bad in [
            HleSoundChannelClearContinuation { channel: 0, ..channel.clone() },
            HleSoundChannelClearContinuation { channel: 5, ..channel.clone() },
            HleSoundChannelClearContinuation { identifier: 0, ..channel.clone() },
            HleSoundChannelClearContinuation { return_cpsr: 0, ..channel },
        ] {
            let mut cpu = Cpu::init(0x0800_0000, 0x1f, true);
            cpu.hle_sound_channel_clear = Some(bad);
            expect_invalid_state(&cpu, 0);
        }
        let sound = HleSoundDriverMainContinuation {
            return_address: 0x0800_0004,
            return_cpsr: 0x1f,
            ..Default::default()
        };
        for bad in [
            HleSoundDriverMainContinuation { return_address: 0x0800_0002, ..sound.clone() },
            HleSoundDriverMainContinuation { pending_player_count: 1, ..sound.clone() },
            HleSoundDriverMainContinuation {
                phase: HleSoundDriverMainPhase::AfterMp2kCallback,
                ..sound.clone()
            },
            HleSoundDriverMainContinuation {
                phase: HleSoundDriverMainPhase::AfterCgbCallback,
                pending_player_count: 1,
                ..sound
            },
        ] {
            let mut cpu = Cpu::init(0x0800_0000, 0x1f, true);
            cpu.hle_sound_driver_main = Some(bad);
            expect_invalid_state(&cpu, 0);
        }
    }

    /// Zig: CPU state rejects impossible HLE IRQ frames.
    #[test]
    fn cpu_state_rejects_impossible_hle_irq_frames() {
        for frame in [
            HleIrqFrame { return_address: 0x0800_0004, ..Default::default() },
            HleIrqFrame { return_address: 0x0800_0002, saved_spsr_irq: 0x1f, ..Default::default() },
        ] {
            let mut cpu = Cpu::init(0x0000_0138, 0x92, true);
            cpu.hle_irq_depth = 1;
            cpu.hle_irq_stack[0] = frame;
            expect_invalid_state(&cpu, 0);
        }
    }

    /// Zig: CPU state rejects impossible HLE MultiBoot progress.
    #[test]
    fn cpu_state_rejects_impossible_hle_multi_boot_progress() {
        let original = multi_boot_continuation();
        let mut cases = Vec::new();
        let mut invalid_parameters = original.clone();
        invalid_parameters.parameters.client_bit = 0;
        cases.push(invalid_parameters);
        let mut past_end = original.clone();
        past_end.source_offset = 0x104;
        cases.push(past_end);
        let mut unaligned_offset = original.clone();
        unaligned_offset.source_offset = 2;
        cases.push(unaligned_offset);
        let mut future_wait = original.clone();
        future_wait.wait_started_at = 101;
        cases.push(future_wait);
        let mut bad_instruction_alignment = original.clone();
        bad_instruction_alignment.instruction_address = 0x0800_0002;
        cases.push(bad_instruction_alignment);
        let mut normal_high_half = original.clone();
        normal_high_half.multiplayer_high_half = true;
        cases.push(normal_high_half);
        let mut bad_encoder = original;
        bad_encoder.encoder.key_xor = 0;
        cases.push(bad_encoder);
        for continuation in cases {
            let mut cpu = Cpu::init(0x0800_0004, 0x1f, true);
            cpu.hle_service = Some(HleService::MultiBoot(continuation));
            expect_invalid_state(&cpu, 100);
        }
    }

    /// Zig: CPU state accepts reachable HLE continuation boundaries.
    #[test]
    fn cpu_state_accepts_reachable_hle_continuation_boundaries() {
        let mut channel = Cpu::init(0x0800_0040, 0x1f, true);
        channel.hle_sound_channel_clear = Some(HleSoundChannelClearContinuation {
            identifier: 0x6873_6d53,
            channel: 4,
            return_address: 0x0800_0004,
            return_cpsr: 0x1f,
            ..Default::default()
        });
        let mut mp2k = Cpu::init(0x0800_0040, 0x1f, true);
        mp2k.hle_sound_driver_main = Some(HleSoundDriverMainContinuation {
            phase: HleSoundDriverMainPhase::AfterMp2kCallback,
            return_address: 0x0800_0004,
            return_cpsr: 0x1f,
            pending_player_count: 16,
            ..Default::default()
        });
        let mut irq = Cpu::init(0x0000_0138, 0x92, true);
        irq.hle_irq_depth = 1;
        irq.hle_irq_stack[0] =
            HleIrqFrame { return_address: 0x0800_0006, saved_spsr_irq: 0x3f, ..Default::default() };
        let mut multi_boot = Cpu::init(0x0800_0004, 0x1f, true);
        multi_boot.hle_service = Some(HleService::MultiBoot(multi_boot_continuation()));
        for cpu in [&channel, &mp2k, &irq, &multi_boot] {
            let mut bytes = Vec::new();
            cpu.write_state(&mut evenfall_state::Sink::new(Some(&mut bytes), None), 100).unwrap();
            let mut decoded = Cpu::init(0x0800_0000, 0x1f, true);
            decoded.read_state(&mut evenfall_state::Source::new(&bytes), 100).unwrap();
        }
    }

    /// ZiGBA `writeState`: unused IRQ frames and pending player slots are canonical.
    #[test]
    fn cpu_state_canonicalizes_unused_hle_slots() {
        let mut cpu = Cpu::init(0x0800_0000, 0x1f, true);
        cpu.hle_sound_driver_main =
            Some(HleSoundDriverMainContinuation { return_cpsr: 0x1f, ..Default::default() });
        let mut expected = Vec::new();
        cpu.write_state(&mut evenfall_state::Sink::new(Some(&mut expected), None), 0).unwrap();

        cpu.hle_irq_stack[7].saved_r12 = 0x8234_abcf;
        cpu.hle_sound_driver_main.as_mut().unwrap().pending_players[15] = 0x0800_0023;
        let mut encoded = Vec::new();
        cpu.write_state(&mut evenfall_state::Sink::new(Some(&mut encoded), None), 0).unwrap();
        assert_eq!(encoded, expected);
        // Serialization does not erase live in-memory values.
        assert_eq!(cpu.hle_irq_stack[7].saved_r12, 0x8234_abcf);
    }

    /// ZiGBA `validateState`: reject a stale resumed SWI whose PC has moved.
    #[test]
    fn bulk_state_requires_return_point() {
        let mut cpu = Cpu::init(0x0800_0004, 0x1f, true);
        cpu.hle_service = Some(HleService::Bulk(HleBulkService {
            instruction_address: 0x0800_0000,
            instruction: 0xef0b_0000,
            instruction_set: InstructionSet::Arm,
            work: HleBulkWork::CpuSet(HleCpuSetWork::default()),
        }));
        assert_eq!(cpu.validate_state(0), Ok(()));
        cpu.core.next_instruction_address = 0x0800_0008;
        assert_eq!(cpu.validate_state(0), Err(evenfall_state::Error::InvalidSaveState));
    }

    fn huffman_work() -> HleBulkWork {
        HleBulkWork::Huffman(HleHuffmanWork {
            tree_base: 0x0300_0005,
            bitstream_address: 0x0300_0008,
            destination: 0x0200_0000,
            remaining: 4,
            node_address: 0x0300_0005,
            symbol_bits: 8,
            ..Default::default()
        })
    }

    fn bulk_service(work: HleBulkWork) -> HleService {
        HleService::Bulk(HleBulkService {
            instruction_address: 0x0800_0000,
            instruction: 0xef00_0000 | u32::from(work.service()) << 16,
            instruction_set: InstructionSet::Arm,
            work,
        })
    }

    /// Zig: CPU state accepts a bulk HLE service at its return point.
    #[test]
    fn cpu_state_accepts_a_bulk_hle_service_at_its_return_point() {
        let mut arm = Cpu::init(0x0800_0004, 0x1f, true);
        arm.hle_service = Some(bulk_service(huffman_work()));
        let mut thumb = Cpu::init(0x0800_0002, 0x3f, true);
        let mut service = match bulk_service(HleBulkWork::CpuSet(HleCpuSetWork {
            source: 0x0200_0002,
            destination: 0x0600_0000,
            remaining: 7,
            ..Default::default()
        })) {
            HleService::Bulk(service) => service,
            HleService::MultiBoot(_) => unreachable!(),
        };
        service.instruction = 0xdf0b;
        service.instruction_set = InstructionSet::Thumb;
        thumb.hle_service = Some(HleService::Bulk(service));
        for cpu in [&arm, &thumb] {
            let mut bytes = Vec::new();
            cpu.write_state(&mut evenfall_state::Sink::new(Some(&mut bytes), None), 0).unwrap();
        }
    }

    /// Zig: CPU state rejects impossible bulk HLE service progress.
    #[test]
    fn cpu_state_rejects_impossible_bulk_hle_service_progress() {
        let mut external = Cpu::init(0x0800_0004, 0x1f, false);
        external.hle_service = Some(bulk_service(huffman_work()));
        expect_invalid_state(&external, 0);
        let mut not_at_return = Cpu::init(0x0800_0008, 0x1f, true);
        not_at_return.hle_service = Some(bulk_service(huffman_work()));
        expect_invalid_state(&not_at_return, 0);

        let mut bad = Vec::new();
        let HleService::Bulk(original) = bulk_service(huffman_work()) else { unreachable!() };
        let mut not_a_swi = original.clone();
        not_a_swi.instruction = 0xe3a0_0000;
        bad.push(not_a_swi);
        let mut wrong_service = original.clone();
        wrong_service.instruction = 0xef11_0000;
        bad.push(wrong_service);
        let mut wrong_instruction_set = original.clone();
        wrong_instruction_set.instruction = 0xdf13;
        wrong_instruction_set.instruction_set = InstructionSet::Thumb;
        bad.push(wrong_instruction_set);
        let mut skipped_symbol = original;
        if let HleBulkWork::Huffman(work) = &mut skipped_symbol.work {
            work.output_bits = 28;
        }
        bad.push(skipped_symbol);
        for service in bad {
            let mut cpu = Cpu::init(0x0800_0004, 0x1f, true);
            cpu.hle_service = Some(HleService::Bulk(service));
            expect_invalid_state(&cpu, 0);
        }
        let mut unaligned_words = Cpu::init(0x0800_0004, 0x1f, true);
        unaligned_words.hle_service =
            Some(bulk_service(HleBulkWork::CpuFastSet(HleCpuFastSetWork {
                source: 0x0200_0000,
                destination: 0x0400_0002,
                remaining_blocks: 1,
                ..Default::default()
            })));
        expect_invalid_state(&unaligned_words, 0);
        let mut bad_width = Cpu::init(0x0800_0004, 0x1f, true);
        bad_width.hle_service = Some(bulk_service(HleBulkWork::BitUnpack(HleBitUnpackWork {
            source: 0x0200_0000,
            destination: 0x0600_0000,
            source_length: 1,
            source_width: 3,
            destination_width: 8,
            ..Default::default()
        })));
        expect_invalid_state(&bad_width, 0);
        let mut unaligned_records = Cpu::init(0x0800_0004, 0x1f, true);
        unaligned_records.hle_service =
            Some(bulk_service(HleBulkWork::BgAffineSet(HleBgAffineSetWork {
                source: 0x03ff_ffed,
                destination: 0x0200_0000,
                remaining: 2,
            })));
        expect_invalid_state(&unaligned_records, 0);
    }

    /// ZiGBA: BgAffineSet analytic preflight matches the record-by-record loop.
    #[test]
    fn bg_affine_set_preflight_crosses_mmio_on_first_reachable_record() {
        let source = 0x03ff_fff9;
        let destination = 0x0500_0000;
        assert!(!bg_affine_set_mmio_fault(source, destination, 0));
        assert!(!bg_affine_set_mmio_fault(source, destination, 1));
        assert!(bg_affine_set_mmio_fault(source, destination, 2));
        assert!(bg_affine_set_mmio_fault(0x0400_0001, destination, 1));
        assert!(!bg_affine_set_mmio_fault(0x0400_0000, 0x0400_0000, u32::MAX));
    }

    /// ZiGBA's ordinary runner differential: the same fetched instructions
    /// must leave CPU, prefetch and bus timing equal to single stepping.
    #[test]
    fn ordinary_runner_matches_single_steps() {
        for thumb in [false, true] {
            let mut rom = [0u8; 256];
            let mut seed = 0x7ac4_0351u32;
            for (i, slot) in rom.chunks_exact_mut(if thumb { 2 } else { 4 }).enumerate() {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let instruction = if thumb {
                    // MOV/ADD/SUB immediate to different low registers.
                    (match i % 3 {
                        0 => 0x2000,
                        1 => 0x3100,
                        _ => 0x3a00,
                    } | ((seed >> 12) & 255)) as u16 as u32
                } else {
                    (match i % 3 {
                        0 => 0xe3a0_0000,
                        1 => 0xe280_1000,
                        _ => 0xe221_2000,
                    }) | ((seed >> 12) & 255)
                };
                slot.copy_from_slice(&instruction.to_le_bytes()[..slot.len()]);
            }
            let mut reference_bus = Bus::init(&rom);
            let mut runner_bus = Bus::init(&rom);
            let cpsr = if thumb { 0x3f } else { 0x1f };
            let mut reference = Cpu::init(0x0800_0000, cpsr, false);
            let mut runner = reference.clone();
            reference.refill_pipeline(&mut reference_bus);
            runner.refill_pipeline(&mut runner_bus);
            for _ in 0..19 {
                assert!(matches!(reference.step(&mut reference_bus), StepOutcome::Executed(_)));
            }
            let mut execution_fault = None;
            let mut fault = None;
            assert_eq!(
                runner.run_normal_until(
                    &mut runner_bus,
                    reference_bus.now(),
                    &mut execution_fault,
                    &mut fault
                ),
                ChunkOutcome::Deadline
            );
            assert_eq!(fault, None);
            assert_eq!(runner, reference);
            assert_eq!(runner_bus.now(), reference_bus.now());
            assert_eq!(runner_bus.fetch_kind, reference_bus.fetch_kind);
            assert_eq!(runner_bus.open_bus_value, reference_bus.open_bus_value);
            assert_eq!(runner_bus.game_pak_prefetch.count, reference_bus.game_pak_prefetch.count);
            assert_eq!(
                runner_bus.game_pak_prefetch.head_address,
                reference_bus.game_pak_prefetch.head_address
            );
        }
    }

    /// ZiGBA: "Thumb ROM kernel matches ordinary stepping for random code, queues and horizons".
    /// The reference takes the exact number of instructions admitted by the
    /// kernel; comparing only final clocks would miss mispredicted branches.
    #[test]
    fn thumb_rom_kernel_matches_ordinary_steps_for_random_code_queues_and_horizons() {
        fn next(seed: &mut u32) -> u32 {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            *seed
        }

        let mut seed = 0x7eed_7b01;
        let mut rom = vec![0u8; 0x30000];
        let mut runs = 0;
        let mut executed = 0;
        let mut refusals = 0;
        for trial in 0..4000 {
            let offset = match next(&mut seed) % 4 {
                0 => 0x1ff80 + next(&mut seed) % 0x80,
                1 => 0x2ff80 + next(&mut seed) % 0x7c,
                2 => 0x80 + next(&mut seed) % 0x40,
                _ => 0x100 + next(&mut seed) % (0x30000 - 0x200),
            } & !1;
            let pc =
                [0x0800_0000, 0x0a00_0000, 0x0c00_0000][(next(&mut seed) % 3) as usize] + offset;
            for at in (offset.saturating_sub(0x40)..(offset + 0xc0).min(0x30000)).step_by(2) {
                let bits = next(&mut seed) as u16;
                let near = (next(&mut seed) % 25) as u16;
                let opcode = match next(&mut seed) % 22 {
                    0 | 1 => bits % 0x1800,
                    2 => 0x1800 | bits & 0x07ff,
                    3 | 4 => 0x2000 | bits & 0x1fff,
                    5 | 6 => 0x4000 | bits & 0x03ff,
                    7 => 0x4400 | bits & 0x03ff,
                    8 => 0x4800 | bits & 0x07ff,
                    9 => 0x5000 | bits & 0x0fff,
                    10 | 11 => 0x6000 | bits & 0x1fff,
                    12 => 0x8000 | bits & 0x0fff,
                    13 => 0x9000 | bits & 0x0fff,
                    14 => {
                        if bits & 1 != 0 {
                            0xa000 | bits & 0x0fff
                        } else {
                            0xb000 | bits & 0xff
                        }
                    }
                    15 => 0xb400 | bits & 0x09ff,
                    16 => 0xc000 | bits & 0x0fff,
                    17 => 0xd000 | (bits % 14) << 8 | near.wrapping_sub(12) & 0xff,
                    18 => 0xe000 | near.wrapping_sub(12) & 0x7ff,
                    19 => {
                        if bits & 1 == 0 {
                            0xf000
                        } else {
                            0xf800 | near
                        }
                    }
                    20 => 0xdf00 | bits & 0xff,
                    _ => 0xde00 | bits & 0xff,
                };
                rom[at as usize..at as usize + 2].copy_from_slice(&opcode.to_le_bytes());
            }
            let wait = [0x4000, 0x4317, 0x43a8][(next(&mut seed) % 3) as usize];
            let mut bus = Bus::init(&rom);
            let mut reference_bus = Bus::init(&rom);
            for bus in [&mut bus, &mut reference_bus] {
                bus.write(
                    Access {
                        address: 0x0400_0204,
                        width: AccessWidth::Half,
                        kind: AccessKind::Nonsequential,
                        source: AccessSource::CpuData,
                    },
                    wait,
                );
                for address in (0x7c00..0x7e00).step_by(4) {
                    let value = (address as u32).wrapping_mul(0x1021_0041);
                    bus.iwram[address..address + 4].copy_from_slice(&value.to_le_bytes());
                }
                bus.begin_device_batch();
                bus.elapse(100);
            }
            let mut cpu = Cpu::init(pc, next(&mut seed) & 0xf000_0000 | 0x3f, true);
            for register in &mut cpu.registers[..15] {
                let value = next(&mut seed);
                *register = match value % 7 {
                    0 | 1 => 0x0200_0000 + (value & 0x3ffff),
                    2 | 3 => 0x0300_0000 + (value & 0x7fff),
                    4 => pc & 0xfe00_0000 | value & 0x1ffff,
                    5 => (pc.wrapping_add((value & 31) * 2)) | 1,
                    _ => value & 255,
                };
            }
            cpu.registers[13] = 0x0300_7c00 + (next(&mut seed) & 0x1fc);
            cpu.refill_pipeline(&mut bus);
            let mut reference = cpu.clone();
            reference.refill_pipeline(&mut reference_bus);
            match next(&mut seed) % 4 {
                0 => {}
                1 => {
                    let pause = 1 + u64::from(next(&mut seed) % 80);
                    bus.elapse(pause);
                    reference_bus.elapse(pause);
                }
                2 => {
                    let pause = 1 + u64::from(next(&mut seed) % 12);
                    bus.elapse(pause);
                    reference_bus.elapse(pause);
                }
                _ => {
                    bus.game_pak_prefetch = Default::default();
                    reference_bus.game_pak_prefetch = Default::default();
                    bus.fetch_kind = AccessKind::Sequential;
                    reference_bus.fetch_kind = AccessKind::Sequential;
                }
            }
            let before = cpu.clone();
            let started = bus.now();
            let yield_at = started + 1 + u64::from(next(&mut seed) % 400);
            let mut view = InstructionFetchRegion::default();
            let count = cpu.run_thumb_rom_kernel(&mut bus, &mut view, yield_at);
            if count == 0 {
                refusals += 1;
                assert_eq!(cpu, before, "refused trial {trial}");
                assert_eq!(bus.now(), started, "refused trial {trial}");
                assert_eq!(bus.game_pak_prefetch, reference_bus.game_pak_prefetch);
            } else {
                runs += 1;
                executed += count;
                for _ in 0..count {
                    assert!(
                        matches!(reference.step(&mut reference_bus), StepOutcome::Executed(_)),
                        "trial {trial}"
                    );
                }
                assert_eq!(cpu, reference, "CPU trial {trial}, {count} instructions");
                assert_eq!(bus.now(), reference_bus.now(), "clock trial {trial}");
                assert!(bus.now() <= yield_at && bus.now() < bus.batch_deadline);
                assert_eq!(
                    bus.open_bus_value, reference_bus.open_bus_value,
                    "open bus trial {trial}"
                );
                assert_eq!(bus.fetch_kind, reference_bus.fetch_kind, "fetch trial {trial}");
                assert_eq!(
                    bus.game_pak_prefetch, reference_bus.game_pak_prefetch,
                    "prefetch trial {trial}"
                );
                assert_eq!(bus.duty_valid_until, reference_bus.duty_valid_until);
                assert_eq!(bus.game_pak_code_active, reference_bus.game_pak_code_active);
                assert_eq!(&bus.iwram[..], &reference_bus.iwram[..]);
                assert_eq!(&bus.ewram[..], &reference_bus.ewram[..]);
            }
            bus.end_device_batch();
            reference_bus.end_device_batch();
        }
        assert!(runs > 600, "only {runs} runs; {refusals} refusals");
        assert!(executed > 3 * runs, "{executed} instructions in {runs} runs");
    }

    /// ZiGBA `thumbKernelBranch`: a taken branch and a POP {pc} must admit
    /// both refill fetches, not only the opcode and data beat. One cycle
    /// short refuses the *whole* instruction without changing its pipeline.
    #[test]
    fn thumb_rom_branch_refill_requires_its_full_horizon() {
        for opcode in [0xe7fe_u16, 0xbd00] {
            let mut rom = vec![0u8; 0x200];
            rom[0x80..0x82].copy_from_slice(&opcode.to_le_bytes());
            rom[0x82..0x84].copy_from_slice(&0x2103_u16.to_le_bytes());
            rom[0xa0..0xa2].copy_from_slice(&0x2007_u16.to_le_bytes());
            let mut bus = Bus::init(&rom);
            let mut reference_bus = Bus::init(&rom);
            for target in [&mut bus, &mut reference_bus] {
                target.iwram[0x7c00..0x7c04].copy_from_slice(&0x0800_00a1_u32.to_le_bytes());
                target.write(
                    Access {
                        address: 0x0400_0204,
                        width: AccessWidth::Half,
                        kind: AccessKind::Nonsequential,
                        source: AccessSource::CpuData,
                    },
                    0x4000,
                );
                target.begin_device_batch();
                target.elapse(100);
            }
            let mut cpu = Cpu::init(0x0800_0080, 0x3f, false);
            cpu.registers[13] = 0x0300_7c00;
            let mut reference = cpu.clone();
            cpu.refill_pipeline(&mut bus);
            reference.refill_pipeline(&mut reference_bus);
            let started = bus.now();
            assert!(matches!(reference.step(&mut reference_bus), StepOutcome::Executed(_)));
            let required = reference_bus.now() - started;
            assert!(required > 1);
            let before = cpu.clone();
            assert_eq!(
                cpu.run_thumb_rom_kernel(
                    &mut bus,
                    &mut InstructionFetchRegion::default(),
                    started + required - 1,
                ),
                0,
            );
            assert_eq!(cpu, before);
            assert_eq!(bus.now(), started);
            assert_eq!(
                cpu.run_thumb_rom_kernel(
                    &mut bus,
                    &mut InstructionFetchRegion::default(),
                    started + required,
                ),
                1,
            );
            assert_eq!(cpu, reference);
            assert_eq!(bus.now(), reference_bus.now());
            assert_eq!(bus.open_bus_value, reference_bus.open_bus_value);
            assert_eq!(bus.game_pak_prefetch, reference_bus.game_pak_prefetch);
            assert_eq!(bus.fetch_kind, reference_bus.fetch_kind);
            bus.end_device_batch();
            reference_bus.end_device_batch();
        }
    }

    /// ZiGBA `runNormalUntil`: IWRAM kernel instructions, RAM transfers and
    /// a backward branch publish the same pipeline, memory and bus timing.
    #[test]
    fn arm_iwram_runner_matches_ordinary_steps_through_a_backwards_branch() {
        let mut reference_bus = Bus::init(&[]);
        let mut runner_bus = Bus::init(&[]);
        for (i, instruction) in [
            0xe280_0003_u32, // ADD r0, r0, #3
            0xe581_0000,     // STR r0, [r1]
            0xe591_2000,     // LDR r2, [r1]
            0xeaff_fffb,     // B back to ADD
        ]
        .iter()
        .enumerate()
        {
            let offset = 0x100 + i * 4;
            reference_bus.iwram[offset..offset + 4].copy_from_slice(&instruction.to_le_bytes());
            runner_bus.iwram[offset..offset + 4].copy_from_slice(&instruction.to_le_bytes());
        }
        let mut reference = Cpu::init(0x0300_0100, 0x1f, false);
        reference.registers[1] = 0x0200_0100;
        let mut runner = reference.clone();
        reference.refill_pipeline(&mut reference_bus);
        runner.refill_pipeline(&mut runner_bus);
        reference_bus.begin_device_batch();
        runner_bus.begin_device_batch();
        for _ in 0..11 {
            assert!(matches!(reference.step(&mut reference_bus), StepOutcome::Executed(_)));
        }
        let mut reason = None;
        let mut fault = None;
        assert_eq!(
            runner.run_normal_until(&mut runner_bus, reference_bus.now(), &mut reason, &mut fault),
            ChunkOutcome::Deadline
        );
        assert_eq!(reason, None);
        assert_eq!(fault, None);
        assert_eq!(runner, reference);
        assert_eq!(runner.registers[0], 9);
        assert_eq!(runner.registers[2], 9);
        assert_eq!(&runner_bus.ewram[0x100..0x104], &9_u32.to_le_bytes());
        assert_eq!(runner_bus.now(), reference_bus.now());
        assert_eq!(runner_bus.open_bus_value, reference_bus.open_bus_value);
        assert_eq!(runner_bus.fetch_kind, reference_bus.fetch_kind);
        assert_eq!(runner_bus.game_pak_prefetch.count, reference_bus.game_pak_prefetch.count);
        reference_bus.end_device_batch();
        runner_bus.end_device_batch();
    }

    /// Zig: unified kernel RAM transfers match reference at full and short horizons.
    #[test]
    fn unified_kernel_ram_transfers_match_reference_at_full_and_short_horizons() {
        for base in [0x0200_0103_u32, 0x03ff_8103] {
            for (class, prefix) in [(0, 0xe401_2003_u32), (1, 0xe801_000b), (2, 0xe001_20b3)] {
                // L, W, B/user, U, P and (single only) register offset.
                for mode in 0_u32..64 {
                    if class != 0 && mode >= 32 {
                        continue;
                    }
                    for short in [false, true] {
                        let mut bus = Bus::init(&[]);
                        let mut reference_bus = Bus::init(&[]);
                        for i in 0..256 {
                            let ewram_byte = (i as u8).wrapping_mul(13).wrapping_add(83);
                            let iwram_byte = (i as u8).wrapping_mul(37).wrapping_add(19);
                            bus.ewram[0x80 + i] = ewram_byte;
                            reference_bus.ewram[0x80 + i] = ewram_byte;
                            bus.iwram[0x80 + i] = iwram_byte;
                            reference_bus.iwram[0x80 + i] = iwram_byte;
                        }
                        bus.iwram[8..12].copy_from_slice(&0xef08_0000_u32.to_le_bytes());
                        reference_bus.iwram[8..12].copy_from_slice(&0xef08_0000_u32.to_le_bytes());
                        let opcode = prefix | mode << 20;
                        bus.begin_device_batch();
                        reference_bus.begin_device_batch();
                        bus.elapse(1);
                        reference_bus.elapse(1);
                        begin_arm_iwram_slice(&mut bus);
                        begin_arm_iwram_slice(&mut reference_bus);
                        let mut cpu = Cpu::init(0x0300_0000, 0xa000_001f, true);
                        cpu.pipeline = Some(Pipeline {
                            instructions: [opcode, 0xef08_0000],
                            instruction_set: InstructionSet::Arm,
                        });
                        cpu.registers[0] = 0x9173_250b;
                        cpu.registers[1] = base;
                        cpu.registers[2] = 0xab37_1589;
                        cpu.registers[3] = 3;
                        let before = cpu.clone();
                        let mut reference = cpu.clone();
                        let _ = reference.step(&mut reference_bus);
                        let load = mode & 1 != 0;
                        let per_access = if base >> 24 == 3 {
                            1
                        } else if class == 2 || (class == 0 && mode & 4 != 0) {
                            3
                        } else {
                            6
                        };
                        let required =
                            1 + per_access * if class == 1 { 3 } else { 1 } + u64::from(load);
                        let started = bus.now();
                        let ran = cpu
                            .run_arm_iwram_kernel(&mut bus, started + required - u64::from(short));
                        assert_eq!(ran, !short, "class {class} mode {mode} base {base:08x}");
                        if short {
                            assert_eq!(cpu, before);
                            assert_eq!(bus.now(), started);
                            let _ = cpu.step(&mut bus);
                        }
                        assert_eq!(cpu, reference, "class {class} mode {mode} base {base:08x}");
                        assert_eq!(bus.now(), reference_bus.now());
                        assert_eq!(bus.open_bus_value, reference_bus.open_bus_value);
                        assert_eq!(bus.fetch_kind, reference_bus.fetch_kind);
                        assert_eq!(&bus.iwram[..], &reference_bus.iwram[..]);
                        assert_eq!(&bus.ewram[..], &reference_bus.ewram[..]);
                        bus.end_device_batch();
                        reference_bus.end_device_batch();
                    }
                }
            }
        }
    }

    /// Zig: ARM kernel matches ordinary execution for random encodings and operands.
    #[test]
    fn arm_kernel_matches_ordinary_execution_for_random_encodings_and_operands() {
        let mut seed = 0x5eed_a4a1_u32;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut executed = 0;
        for trial in 0..3000 {
            let mut instruction = random();
            if instruction & (1 << 29) == 0 {
                instruction = instruction & 0x0fff_ffff | 0xe000_0000;
            }
            if (instruction >> 26) & 3 == 3 {
                instruction &= !(1 << 27);
            }
            if (instruction >> 25) & 7 == 5 {
                instruction =
                    instruction & 0xff00_0000 | (random() % 17).wrapping_sub(8) & 0xff_ffff;
            }
            match trial % 8 {
                0 => instruction = instruction & 0xf03f_ff0f | 0x0000_0090,
                1 | 2 => {
                    instruction =
                        instruction & 0xf1ff_ff9f | 0x0000_0090 | ((random() % 3 + 1) << 5)
                }
                _ => {}
            }
            let address = 0x0300_0100 + (random() & 0x3fc);
            let mut runner_bus = Bus::init(&[]);
            let mut reference_bus = Bus::init(&[]);
            for index in (address & 0x7fff).saturating_sub(0x40)..(address & 0x7fff) + 0x40 {
                if index & 3 == 0 {
                    runner_bus.iwram[index as usize..index as usize + 4]
                        .copy_from_slice(&0xef08_0000_u32.to_le_bytes());
                    reference_bus.iwram[index as usize..index as usize + 4]
                        .copy_from_slice(&0xef08_0000_u32.to_le_bytes());
                }
            }
            runner_bus.begin_device_batch();
            reference_bus.begin_device_batch();
            runner_bus.elapse(1);
            reference_bus.elapse(1);
            begin_arm_iwram_slice(&mut runner_bus);
            begin_arm_iwram_slice(&mut reference_bus);
            let mut cpu = Cpu::init(address, (random() & 0xf000_0000) | 0x1f, true);
            cpu.pipeline = Some(Pipeline {
                instructions: [instruction, 0xef08_0000],
                instruction_set: InstructionSet::Arm,
            });
            for register in &mut cpu.registers[..15] {
                *register = match random() & 7 {
                    0 | 1 => random() & 255,
                    2 => 0x0300_0000 + (random() & 0x7fff),
                    3 => 0x0200_0000 + (random() & 0x3_ffff),
                    4 => 0x03ff_fff0 + (random() & 15),
                    5 => 0x0800_0000 + (random() & 0x1_ffff),
                    6 => 0x0400_0000 + (random() & 0x3ff),
                    _ => random(),
                };
            }
            let mut reference = cpu.clone();
            let before = cpu.clone();
            let started = runner_bus.now();
            let budget = u64::from(random() % 40 + 1);
            if cpu.run_arm_iwram_kernel(&mut runner_bus, started + budget) {
                executed += 1;
                let _ = reference.step(&mut reference_bus);
                assert_eq!(cpu, reference, "trial {trial}: {instruction:08x}");
                assert_eq!(runner_bus.now(), reference_bus.now(), "trial {trial}");
                assert_eq!(
                    runner_bus.open_bus_value, reference_bus.open_bus_value,
                    "trial {trial}"
                );
                assert_eq!(runner_bus.fetch_kind, reference_bus.fetch_kind, "trial {trial}");
                assert_eq!(&runner_bus.iwram[..], &reference_bus.iwram[..], "trial {trial}");
                assert_eq!(&runner_bus.ewram[..], &reference_bus.ewram[..], "trial {trial}");
                assert!(runner_bus.now() <= started + budget);
            } else {
                assert_eq!(cpu, before, "trial {trial}");
                assert_eq!(runner_bus.now(), started, "trial {trial}");
            }
            runner_bus.end_device_batch();
            reference_bus.end_device_batch();
        }
        assert!(executed > 3000 / 3, "kernel did not exercise enough random instructions");
    }
}
