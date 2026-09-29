//! Engine-versus-interpreter differentials: the interpreter's `step` is the
//! semantic and timing reference for every instruction the engine runs.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::{Cpu, StepOutcome};
use evenfall_state::Codec;

struct Random(u32);

impl Random {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn below(&mut self, bound: u32) -> u32 {
        self.next() % bound
    }
}

fn write_word(bus: &mut Bus, address: u32, value: u32) {
    let index = (address & 0x00ff_ffff) as usize;
    match address >> 24 {
        2 => bus.ewram[index & 0x3_ffff..(index & 0x3_ffff) + 4]
            .copy_from_slice(&value.to_le_bytes()),
        3 => bus.iwram[index & 0x7fff..(index & 0x7fff) + 4].copy_from_slice(&value.to_le_bytes()),
        _ => unreachable!(),
    }
}

/// Two identical machines, one to run the engine and one to step.
struct Pair {
    engine: Cpu,
    engine_bus: Bus,
    reference: Cpu,
    reference_bus: Bus,
    /// IRQs an MMIO store in an engine run raised.
    irq_entries: u64,
}

impl Pair {
    fn new(rom: &[u8], setup: impl Fn(&mut Bus), cpu: Cpu, lead: u64) -> Self {
        let mut buses = [Bus::init(rom), Bus::init(rom)];
        let mut cpus = [cpu.clone(), cpu];
        for (bus, cpu) in buses.iter_mut().zip(&mut cpus) {
            setup(bus);
            bus.begin_device_batch();
            bus.elapse(lead);
            cpu.refill_pipeline(bus);
        }
        let [engine_bus, reference_bus] = buses;
        let [engine, reference] = cpus;
        Self { engine, engine_bus, reference, reference_bus, irq_entries: 0 }
    }

    /// Runs the engine once and steps the reference through the same
    /// instructions; returns whether the engine ran.
    fn compare(&mut self, yield_at: u64, context: &str) -> bool {
        let before = self.engine.clone();
        let started = self.engine_bus.now();
        let ran = self.engine.run_engine(&mut self.engine_bus, yield_at);
        if !ran {
            assert_eq!(self.engine, before, "{context}: a refused run changed the CPU");
            assert_eq!(self.engine_bus.now(), started, "{context}: a refused run took time");
            return false;
        }
        let mut end = self.engine_bus.now();
        if self.engine_bus.irq_line && self.engine.cpsr & (1 << 7) == 0 {
            // As the ordinary loop does after a run that raised an IRQ.
            self.engine_bus.resume_power();
            self.engine.enter_irq(&mut self.engine_bus).expect("IRQ entry");
            self.engine.refill_pipeline(&mut self.engine_bus);
            self.irq_entries += 1;
            assert!(end <= yield_at, "{context}: a run that raised an IRQ passed its yield");
            end = self.engine_bus.now();
            let mut steps = 0;
            while self.reference_bus.now() < end {
                let outcome = self.reference.step(&mut self.reference_bus);
                assert!(matches!(outcome, StepOutcome::Executed(_)), "{context}: {outcome:?}");
                steps += 1;
            }
            self.assert_equal(&format!("{context} after {steps} steps and an IRQ"));
            return true;
        }
        assert!(
            end <= yield_at && end > started,
            "{context}: clock {started}..{end} vs {yield_at}; before {before:x?}, after {:x?}",
            self.engine
        );
        let mut steps = 0;
        while self.reference_bus.now() < end {
            let outcome = self.reference.step(&mut self.reference_bus);
            assert!(matches!(outcome, StepOutcome::Executed(_)), "{context}: {outcome:?}");
            steps += 1;
        }
        self.assert_equal(&format!("{context} after {steps} steps"));
        true
    }

    fn assert_equal(&self, context: &str) {
        assert_eq!(self.engine, self.reference, "{context}: CPU");
        let (a, b) = (&self.engine_bus, &self.reference_bus);
        assert_eq!(a.now(), b.now(), "{context}: clock");
        assert_eq!(a.irq_line, b.irq_line, "{context}: IRQ line");
        assert_eq!(a.power_state(), b.power_state(), "{context}: power");
        assert_eq!(a.pending_dma, b.pending_dma, "{context}: pending DMA");
        assert_eq!(a.batch_deadline, b.batch_deadline, "{context}: device horizon");
        assert_eq!(a.open_bus_value, b.open_bus_value, "{context}: open bus");
        assert_eq!(a.fetch_kind, b.fetch_kind, "{context}: fetch kind");
        assert_eq!(a.game_pak_prefetch, b.game_pak_prefetch, "{context}: prefetch");
        assert_eq!(a.game_pak_code_active, b.game_pak_code_active, "{context}: code active");
        assert!(a.iwram[..] == b.iwram[..], "{context}: IWRAM");
        assert!(a.ewram[..] == b.ewram[..], "{context}: EWRAM");
        assert!(a.vram[..] == b.vram[..], "{context}: VRAM");
        assert!(a.palette[..] == b.palette[..], "{context}: palette");
        assert!(a.oam[..] == b.oam[..], "{context}: OAM");
        assert_eq!(a.oam_compact_valid, b.oam_compact_valid, "{context}: OAM cache");
        assert_eq!(a.object_line_index, b.object_line_index, "{context}: object lines");
        assert!(a.scanline_memo.same_changes(&b.scanline_memo), "{context}: memo marks");
        // Raster history: VRAM and palette writes during visible dots.
        let history = |bus: &Bus| {
            let count = usize::from(bus.backdrop_segment_count);
            let segments: Vec<_> =
                bus.backdrop_segments[..count].iter().map(|s| (s.start_x, s.color)).collect();
            (bus.backdrop_segment_line, segments)
        };
        assert_eq!(history(a), history(b), "{context}: raster history");
    }

    /// Ends both device batches, which brings the lazily advanced devices
    /// up to the clock, and compares the PPU position.
    fn settle(&mut self, context: &str) {
        self.engine_bus.end_device_batch();
        self.reference_bus.end_device_batch();
        assert_eq!(self.engine_bus.ppu_timing, self.reference_bus.ppu_timing, "{context}: PPU");
        let bytes = |bus: &Bus| {
            let mut bytes = Vec::new();
            bus.encode(&mut evenfall_state::Sink::new(Some(&mut bytes), None)).unwrap();
            bytes
        };
        assert!(bytes(&self.engine_bus) == bytes(&self.reference_bus), "{context}: bus state");
    }
}

/// The cycles before a trial: mostly a few lines into the frame, sometimes
/// in VBlank, where video memory stores run in the engine.
fn lead(random: &mut Random) -> u64 {
    if random.below(4) == 0 {
        160 * 1232 + u64::from(random.below(68 * 1232 - 2000))
    } else {
        1 + u64::from(random.below(1200))
    }
}

fn set_io(bus: &mut Bus, address: u32, value: u16) {
    bus.write(
        Access {
            address,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        },
        u32::from(value),
    );
}

fn set_waitcnt(bus: &mut Bus, value: u16) {
    set_io(bus, 0x0400_0204, value);
}

/// Selects a display mode, which changes how VRAM addresses map.
fn set_dispcnt(bus: &mut Bus, mode: u16) {
    set_io(bus, 0x0400_0000, mode);
}

/// A register value: small numbers, RAM, video memory and ROM pointers, or
/// noise.
fn operand(random: &mut Random) -> u32 {
    match random.below(11) {
        0..=2 => random.below(64),
        3 => 0x0300_4000 + (random.next() & 0x0ffc),
        4 => 0x0200_8000 + (random.next() & 0x0fff),
        5 => 0x0800_0000 + (random.next() & 0x3ffe),
        6 => 0x0300_4000 + random.below(64),
        7 => 1 << random.below(32),
        8 if random.below(2) == 0 => mmio_pointer(random),
        8 => video_pointer(random),
        _ => random.next(),
    }
}

/// A pointer at or just below an I/O register the engine runs transfers
/// to: display and affine registers, DMA and timers, interrupt control,
/// sound and its FIFO, WAITCNT (which it leaves to the interpreter),
/// HALTCNT, and unmapped ones that read as open bus.
fn mmio_pointer(random: &mut Random) -> u32 {
    const OFFSETS: [u32; 24] = [
        0x000, 0x004, 0x006, 0x010, 0x020, 0x028, 0x030, 0x03c, 0x050, 0x060, 0x080, 0x084, 0x0a0,
        0x0b0, 0x0d4, 0x0dc, 0x100, 0x104, 0x130, 0x200, 0x202, 0x204, 0x208, 0x0e0,
    ];
    let offset = OFFSETS[random.below(OFFSETS.len() as u32) as usize];
    match random.below(8) {
        0 => 0x0400_0300 + random.below(4),
        1 => 0x0400_0400 + random.below(64),
        _ => (0x0400_0000 + offset + random.below(4)).wrapping_sub(random.below(2) * 16),
    }
}

/// A pointer into palette RAM, VRAM (with its mirrors and the bitmap-mode
/// hole) or OAM.
fn video_pointer(random: &mut Random) -> u32 {
    match random.below(3) {
        0 => 0x0500_0000 + (random.next() & 0x3ff),
        1 => 0x0600_0000 + (random.next() & 0x1_ffff),
        _ => 0x0700_0000 + (random.next() & 0x3ff),
    }
}

/// An ARM instruction drawn to cover the engine's kinds, the forms it
/// leaves to the interpreter, and r15 in every operand position.
fn arm_instruction(random: &mut Random, index: u32, length: u32) -> u32 {
    let cond = match random.below(8) {
        0 => random.below(16),
        1 => random.below(15),
        _ => 14,
    } << 28;
    let reg = |random: &mut Random| if random.below(12) == 0 { 15 } else { random.below(15) };
    let (rd, rn, rm, rs) = (reg(random), reg(random), reg(random), reg(random));
    let body = match random.below(20) {
        // Data processing: immediate, immediate shifts, register shifts.
        0..=3 => {
            random.next() & 0x01ff_ffff & !(0xf << 12)
                | rd << 12
                | rn << 16
                | if random.below(2) == 0 { 1 << 25 } else { 0 }
        }
        4 | 5 => (random.next() & 0x01f0_0f80) | rd << 12 | rn << 16 | rm | (random.below(4) << 5),
        6 => {
            (random.next() & 0x01f0_0000)
                | rd << 12
                | rn << 16
                | rm
                | rs << 8
                | 0x10
                | (random.below(4) << 5)
        }
        // Single transfers with every addressing mode.
        7..=9 => {
            let base = 8 + random.below(4);
            (0x0400_0000 | (random.next() & 0x03f0_0000) & !(1 << 25))
                | base << 16
                | rd << 12
                | random.below(64)
        }
        10 => {
            let base = 8 + random.below(4);
            0x0600_0000
                | (random.next() & 0x01f0_0000)
                | base << 16
                | rd << 12
                | (random.below(4) << 7)
                | rm
        }
        // Halfword and signed transfers.
        11 | 12 => {
            let base = 8 + random.below(4);
            (random.next() & 0x01f0_0000)
                | base << 16
                | rd << 12
                | 0x90
                | (random.below(4) << 5)
                | random.below(16)
                | (random.below(4) << 8)
        }
        // Block transfers.
        13 | 14 => {
            let base = 8 + random.below(4);
            0x0800_0000 | (random.next() & 0x01f0_0000) | base << 16 | (random.next() & 0xffff)
        }
        // Multiplies.
        15 => (random.next() & 0x0030_0000) | rd << 16 | rn << 12 | rs << 8 | 0x90 | rm,
        16 => {
            0x0080_0000 | (random.next() & 0x0070_0000) | rd << 16 | rn << 12 | rs << 8 | 0x90 | rm
        }
        // Branches within or just outside the block.
        17 => {
            let target = random.below(length + 4) as i32 - 2;
            let offset = (target - index as i32 - 2) as u32 & 0x00ff_ffff;
            0x0a00_0000 | (random.below(2) << 24) | offset
        }
        // MRS, MSR, BX, SWP, SWI and undefined encodings.
        18 => match random.below(6) {
            0 => 0x010f_0000 | rd << 12 | (random.below(2) << 22),
            1 => 0x0128_f000 | rm,
            2 => 0x012f_ff10 | rm,
            3 => 0x0100_0090 | rn << 16 | rd << 12 | rm,
            4 => 0x0f00_0000 | random.below(0x100),
            _ => 0x0600_0010 | (random.next() & 0x01ff_ffef),
        },
        // Stores into the block itself.
        _ => 0x0580_0000 | 12 << 16 | rd << 12 | (random.below(length) * 4),
    };
    cond | body & 0x0fff_ffff
}

fn arm_block(random: &mut Random, base: u32, length: u32) -> Vec<(u32, u32)> {
    (0..length).map(|i| (base + i * 4, arm_instruction(random, i, length))).collect()
}

fn arm_cpu(random: &mut Random, pc: u32) -> Cpu {
    let mut cpu = Cpu::init(pc, random.next() & 0xf000_0000 | 0x1f, true);
    for register in 0..15 {
        cpu.registers[register] = operand(random);
    }
    cpu.registers[8] = 0x0300_4000 + (random.next() & 0x3fc);
    cpu.registers[9] = 0x0200_8000 + (random.next() & 0x3ff);
    cpu.registers[10] = match random.below(3) {
        0 => 0x0800_0000 + (random.next() & 0xffe),
        1 => video_pointer(random),
        _ => mmio_pointer(random),
    };
    cpu.registers[11] = 0x0300_4000 + (random.next() & 0xfff);
    cpu
}

/// Pending interrupt requests that a store to IME (or to IE) raises, in
/// some trials, as `(IF, IE, IME)`.
fn interrupts(random: &mut Random) -> Option<(u16, u16, bool)> {
    let pending = random.next() as u16 & 0x3fff | 1;
    match random.below(3) {
        0 => Some((pending, random.next() as u16 & 0x3fff | pending & 1, false)),
        1 => Some((pending, 0, true)),
        _ => None,
    }
}

fn set_interrupts(bus: &mut Bus, interrupts: Option<(u16, u16, bool)>) {
    if let Some((flags, enabled, master)) = interrupts {
        bus.interrupt_request_flags = flags;
        bus.interrupt_enable_mask = enabled;
        bus.interrupt_master_enabled = master;
        bus.update_irq_line();
    }
}

fn data_setup(random: &mut Random) -> (Vec<u8>, Vec<u8>) {
    let iwram = (0..0x2000).map(|_| random.next() as u8).collect();
    let ewram = (0..0x2000).map(|_| random.next() as u8).collect();
    (iwram, ewram)
}

/// ARM code from IWRAM and EWRAM: every run of random code, registers,
/// flags, horizons and self-modification matches single stepping.
#[test]
fn arm_ram_engine_matches_interpreter_for_random_code() {
    let mut random = Random(0x5eed_a4a1);
    let mut rom = vec![0u8; 0x8000];
    for byte in &mut rom {
        *byte = random.next() as u8;
    }
    let (mut runs, mut refusals, mut instructions, mut irq_entries) = (0, 0, 0, 0);
    for trial in 0..3000 {
        let ewram_code = trial % 3 == 2;
        let pc = if ewram_code { 0x0203_0000 } else { 0x0300_1000 } + (random.below(64) * 4);
        let length = 8 + random.below(40);
        let block = arm_block(&mut random, pc, length);
        let (iwram, ewram) = data_setup(&mut random);
        let cpu = {
            let mut cpu = arm_cpu(&mut random, pc);
            cpu.registers[12] = pc;
            cpu
        };
        let waitcnt = random.next() as u16 & 0x5fff;
        let dispcnt = random.below(6) as u16;
        let irqs = interrupts(&mut Random(0x1e1e_0001 ^ (trial as u32).wrapping_mul(0x9e37_79b9)));
        let mut pair = Pair::new(
            &rom,
            |bus| {
                set_waitcnt(bus, waitcnt);
                set_dispcnt(bus, dispcnt);
                set_interrupts(bus, irqs);
                bus.iwram[0x4000..0x6000].copy_from_slice(&iwram);
                bus.ewram[0x8000..0xa000].copy_from_slice(&ewram);
                for &(address, word) in &block {
                    write_word(bus, address, word);
                }
            },
            cpu,
            lead(&mut random),
        );
        for attempt in 0..6 {
            let budget = 1 + u64::from(random.below(if attempt == 0 { 40 } else { 400 }));
            let yield_at = pair.engine_bus.now() + budget;
            let before = pair.engine_bus.now();
            if pair.compare(yield_at, &format!("trial {trial} attempt {attempt}")) {
                runs += 1;
                instructions += pair.engine_bus.now() - before;
            } else {
                refusals += 1;
                // Let the interpreter take the refused instruction.
                let outcome = pair.engine.step(&mut pair.engine_bus);
                let reference = pair.reference.step(&mut pair.reference_bus);
                assert_eq!(outcome, reference, "trial {trial}");
                pair.assert_equal(&format!("trial {trial} step after refusal"));
                if matches!(outcome, StepOutcome::Faulted(_)) {
                    break;
                }
            }
        }
        irq_entries += pair.irq_entries;
        pair.settle(&format!("trial {trial}"));
    }
    assert!(runs > 4000, "only {runs} runs ({refusals} refusals)");
    assert!(instructions > 40_000, "only {instructions} cycles in engine runs");
    let video = super::mem::VIDEO_STORES.get();
    assert!(video > 100, "only {video} video memory stores in engine runs");
    let mmio = super::MMIO_ACCESSES.get();
    assert!(mmio > 150, "only {mmio} MMIO accesses in engine runs");
    assert!(irq_entries <= runs, "{irq_entries} IRQs");
}

/// A long loop in IWRAM: decode once, then many passes through a backward
/// branch, ending exactly at the yield point or the device horizon.
#[test]
fn arm_iwram_loop_runs_to_the_horizon_like_stepping() {
    let code = [
        0xe280_0003_u32, // ADD r0, r0, #3
        0xe022_1080,     // EOR r1, r2, r0, LSL #1
        0xe581_0000,     // STR r0, [r1]  (r1 is rewritten below)
        0xe3a0_1403,     // MOV r1, #0x03000000
        0xe381_1a05,     // ORR r1, r1, #0x5000
        0xe581_0000,     // STR r0, [r1]
        0xe591_2000,     // LDR r2, [r1]
        0xe252_2001,     // SUBS r2, r2, #1
        0x1afffff6,      // BNE back to ADD
        0xeafffff5,      // B back to ADD
    ];
    for yield_offset in [1, 2, 3, 50, 500, 5000, 50_000] {
        let mut pair = Pair::new(
            &[],
            |bus| {
                for (i, &word) in code.iter().enumerate() {
                    write_word(bus, 0x0300_0100 + 4 * i as u32, word);
                }
            },
            Cpu::init(0x0300_0100, 0x1f, true),
            1,
        );
        let yield_at = pair.engine_bus.now() + yield_offset;
        while pair.engine_bus.now() < yield_at {
            if !pair.compare(yield_at, &format!("yield {yield_offset}")) {
                assert!(matches!(pair.engine.step(&mut pair.engine_bus), StepOutcome::Executed(_)));
                assert!(matches!(
                    pair.reference.step(&mut pair.reference_bus),
                    StepOutcome::Executed(_)
                ));
                pair.assert_equal("after refusal");
            }
        }
    }
}

/// Runs the engine and the reference to `yield_at`, letting the
/// interpreter take every instruction the engine refuses.
fn run_pair_to(pair: &mut Pair, yield_at: u64, context: &str) {
    while pair.engine_bus.now() < yield_at {
        if !pair.compare(yield_at, context) {
            let outcome = pair.engine.step(&mut pair.engine_bus);
            assert_eq!(outcome, pair.reference.step(&mut pair.reference_bus), "{context}");
            pair.assert_equal(&format!("{context} after refusal"));
        }
    }
}

/// A measured block of ALU ops whose slots change afterwards, at every
/// position: to ops outside blocks, which end the block there, and to other
/// block ops, which it runs.
#[test]
fn arm_blocks_follow_slots_that_change_after_measuring() {
    // ADD rN, rN, #i for r0..r3, then B back to the first.
    let mut code: Vec<u32> =
        (0..16).map(|i| 0xe280_0000 | ((i % 4) * 0x0001_1000) | (i + 1)).collect();
    code.push(0xeaff_ffee);
    let replacements = [
        0xe598_1000, // LDR r1, [r8]
        0xe001_0091, // MUL r1, r1, r0
        0xe081_1110, // ADD r1, r1, r0, LSL r1
        0xe281_1005, // ADD r1, r1, #5
        0x1081_1002, // ADDNE r1, r1, r2
    ];
    for slot in 1..16 {
        for replacement in replacements {
            let context = format!("slot {slot} replacement {replacement:08x}");
            let mut cpu = Cpu::init(0x0300_0100, 0x1f, true);
            cpu.registers[8] = 0x0300_4000;
            let mut pair = Pair::new(
                &[],
                |bus| {
                    for (i, &word) in code.iter().enumerate() {
                        write_word(bus, 0x0300_0100 + 4 * i as u32, word);
                    }
                },
                cpu,
                1,
            );
            let yield_at = pair.engine_bus.now() + 200;
            run_pair_to(&mut pair, yield_at, &context);
            for bus in [&mut pair.engine_bus, &mut pair.reference_bus] {
                bus.write(
                    Access {
                        address: 0x0300_0100 + 4 * slot,
                        width: AccessWidth::Word,
                        kind: AccessKind::Nonsequential,
                        source: AccessSource::CpuData,
                    },
                    replacement,
                );
            }
            pair.assert_equal(&context);
            let yield_at = pair.engine_bus.now() + 300;
            run_pair_to(&mut pair, yield_at, &context);
        }
    }
}

/// Back-to-back VRAM stores from HBlank into the next line: the engine runs
/// the stores that end before the line's first visible dot and leaves the
/// rest to the interpreter, which records them in the raster history.
#[test]
fn video_stores_stop_before_the_first_visible_dot() {
    // STRH r0, [r4] (Thumb, 2 cycles from IWRAM) and STR r0, [r4] (ARM, 3).
    let thumb: Vec<u8> = [0x8020_u16; 24].iter().flat_map(|half| half.to_le_bytes()).collect();
    let arm: Vec<u8> = [0xe584_0000_u32; 24].iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut engine_stores = 0;
    for (code, is_thumb) in [(thumb, true), (arm, false)] {
        for lead in 5 * 1232 + 900..5 * 1232 + 1300 {
            let mut cpu = Cpu::init(0x0300_1000, 0x1f | if is_thumb { 0x20 } else { 0 }, true);
            cpu.registers[0] = 0x1234_5678 ^ lead as u32;
            cpu.registers[4] = 0x0600_0100;
            let mut pair = Pair::new(
                &[],
                |bus| bus.iwram[0x1000..0x1000 + code.len()].copy_from_slice(&code),
                cpu,
                lead,
            );
            let yield_at = pair.engine_bus.now() + 40;
            let before = super::mem::VIDEO_STORES.get();
            while pair.engine_bus.now() < yield_at {
                if !pair.compare(yield_at, &format!("lead {lead}")) {
                    let outcome = pair.engine.step(&mut pair.engine_bus);
                    assert_eq!(outcome, pair.reference.step(&mut pair.reference_bus));
                    pair.assert_equal(&format!("lead {lead} after refusal"));
                }
            }
            engine_stores += super::mem::VIDEO_STORES.get() - before;
        }
    }
    assert!(engine_stores > 1000, "only {engine_stores} stores in engine runs");
}

/// A Thumb instruction drawn to cover the engine's kinds and the forms it
/// leaves to the interpreter.
fn thumb_instruction(random: &mut Random, index: u32, length: u32) -> u16 {
    let bits = random.next() as u16;
    let near = |random: &mut Random| (random.below(length + 6) as i32 - index as i32 - 5) as u16;
    match random.below(24) {
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
        14 => 0xa000 | bits & 0x0fff,
        15 => 0xb000 | bits & 0xff,
        16 => 0xb400 | bits & 0x09ff,
        17 => 0xc000 | bits & 0x0fff,
        18 => 0xd000 | (bits % 14) << 8 | near(random) & 0xff,
        19 => 0xe000 | near(random) & 0x7ff,
        20 => {
            if bits & 1 == 0 {
                0xf000 | (bits >> 1) & 0x0007
            } else {
                0xf7ff
            }
        }
        21 => 0xf800 | near(random) & 0x7ff,
        22 => 0xdf00 | bits & 0xff,
        _ => match bits & 3 {
            0 => 0xde00 | bits & 0xff,
            1 => 0xb100 | bits & 0x02ff,
            2 => 0xe800 | bits & 0x07ff,
            _ => 0x4700 | bits & 0x0078,
        },
    }
}

fn thumb_cpu(random: &mut Random, pc: u32) -> Cpu {
    let mut cpu = Cpu::init(pc, random.next() & 0xf000_0000 | 0x3f, true);
    for register in 0..15 {
        cpu.registers[register] = operand(random);
    }
    // Low registers often point at data; SP at a stack in IWRAM.
    cpu.registers[1] = 0x0300_4000 + (random.next() & 0x3fc);
    cpu.registers[2] = 0x0200_8000 + (random.next() & 0x3ff);
    cpu.registers[3] = random.below(64);
    cpu.registers[4] =
        if random.below(3) == 0 { mmio_pointer(random) } else { video_pointer(random) };
    cpu.registers[13] = 0x0300_5000 + (random.next() & 0x3fc);
    cpu.registers[14] = pc.wrapping_add(random.below(64) * 2) | 1;
    cpu
}

/// Zig: "Thumb ROM kernel matches ordinary stepping for random code, queues
/// and horizons", its closed-form half. From every prefetch buffer state
/// at entry (stopped, filling at each depth, full), an instruction with no,
/// one or several internal cycles or a ROM data load, then a branch to each
/// distance from the buffer head, near block and image ends and on short
/// horizons, runs as single stepping does. Code the buffer cannot run from
/// is refused without a trace.
#[test]
fn thumb_rom_engine_matches_stepping_from_every_buffer_state() {
    use crate::bus::GamePakPrefetch;
    let mut random = Random(0x7eed_7b01);
    let mut rom = vec![0u8; 0x3_0000];
    for byte in &mut rom {
        *byte = random.next() as u8;
    }
    let (mut compared, mut refused, mut loads, mut branches, mut full) = (0, 0, 0, 0, 0);
    let mut combination = 0;
    for wait in [0x4000_u16, 0x4317, 0x43a8] {
        for pc in [0x0800_00d0_u32, 0x0801_ffd0, 0x0a02_ffd0, 0x0c00_0100, 0x0801_fff0, 0x0802_fffc]
        {
            let edge = matches!(pc, 0x0801_fff0 | 0x0802_fffc);
            // MOVS r1, #3; LSL r1, r2; LDR r0, [r3]; MUL r0, r2.
            for body in [0x2103_u16, 0x4091, 0x6818, 0x4350] {
                for queued in [None, Some(0), Some(1), Some(4), Some(7), Some(8)] {
                    for offset in [None, Some(0_u32), Some(2), Some(6), Some(18), Some(0xffff_fff4)]
                    {
                        combination += 1;
                        let mut image = rom.clone();
                        let at = (pc & 0x01ff_ffff) as usize;
                        image[at..at + 2].copy_from_slice(&body.to_le_bytes());
                        if let Some(offset) = offset {
                            let b = 0xe000 | ((offset >> 1) & 0x7ff) as u16;
                            image[at + 2..at + 4].copy_from_slice(&b.to_le_bytes());
                        }
                        let mut cpu = Cpu::init(pc, 0x3f, true);
                        cpu.registers[2] = 0x0012_3456;
                        cpu.registers[3] = (pc & 0xfe00_0000) + 0x120;
                        let mut pair = Pair::new(&image, |bus| set_waitcnt(bus, wait), cpu, 100);
                        let head = pc + 4;
                        for bus in [&mut pair.engine_bus, &mut pair.reference_bus] {
                            let window = ((pc - 0x0800_0000) >> 25) as usize;
                            let duty = u64::from(bus.game_pak_s_cycles[window]);
                            bus.game_pak_code_active = true;
                            bus.duty_valid_until = (pc | 0x1ffff) + 1;
                            bus.game_pak_prefetch = match queued {
                                None => {
                                    bus.fetch_kind = AccessKind::Nonsequential;
                                    GamePakPrefetch::default()
                                }
                                Some(count) => GamePakPrefetch {
                                    active: true,
                                    head_address: head,
                                    last_address: head + 2 * u32::from(count.min(7)),
                                    count,
                                    capacity: 8,
                                    opcode_bytes: 2,
                                    countdown: if count == 8 { 0 } else { duty - 1 },
                                    duty,
                                },
                            };
                        }
                        let budget = if combination % 2 == 0 { 2 } else { 250 };
                        let yield_at = pair.engine_bus.now() + budget;
                        let context = format!(
                            "wait {wait:04x} pc {pc:08x} body {body:04x} queued {queued:?} \
                             offset {offset:?} budget {budget}"
                        );
                        if pair.compare(yield_at, &context) {
                            assert!(!edge, "{context}: ran at a block or image end");
                            compared += 1;
                            loads += usize::from(body == 0x6818);
                            branches += usize::from(offset.is_some());
                            full += usize::from(queued == Some(8));
                        } else {
                            refused += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(compared > 800, "only {compared} runs compared");
    assert!(loads > 150 && branches > 600 && full > 100, "{loads} {branches} {full}");
    assert!(refused > 400, "only {refused} refusals");
}

/// Thumb code from IWRAM, EWRAM and Game Pak ROM, with and without the
/// prefetch buffer: every run of random code, flags, wait states, buffer
/// states, horizons and page edges matches single stepping.
#[test]
fn thumb_engine_matches_interpreter_for_random_code() {
    let mut random = Random(0x7eed_7b01);
    let mut rom = vec![0u8; 0x2_0000];
    for byte in &mut rom {
        *byte = random.next() as u8;
    }
    let (mut runs, mut refusals, mut cycles, mut prefetch_runs, mut irq_entries) = (0, 0, 0, 0, 0);
    for trial in 0..4000 {
        let place = trial % 4;
        let base = match place {
            0 => 0x0300_1000,
            1 => 0x0203_0000,
            2 => 0x0800_7f80, // a ROM page edge
            _ => [0x0800_2000, 0x0a00_2000, 0x0c00_2000][(trial / 4) % 3],
        };
        let pc = base + random.below(48) * 2;
        let length = 8 + random.below(40);
        let code: Vec<u16> =
            (0..length).map(|i| thumb_instruction(&mut random, i, length)).collect();
        let first = pc - random.below(4) * 2;
        let mut image = rom.clone();
        if pc >> 24 >= 8 {
            for (i, &half) in code.iter().enumerate() {
                let at = ((first & 0x01ff_ffff) as usize + 2 * i) % image.len();
                image[at..at + 2].copy_from_slice(&half.to_le_bytes());
            }
        }
        let (iwram, ewram) = data_setup(&mut random);
        let cpu = thumb_cpu(&mut random, pc);
        // Mostly with the prefetch buffer, as games run.
        let waitcnt = random.next() as u16 & 0x3fff | if trial % 3 == 0 { 0 } else { 0x4000 };
        let dispcnt = random.below(6) as u16;
        let irqs = interrupts(&mut Random(0x1e1e_0001 ^ (trial as u32).wrapping_mul(0x9e37_79b9)));
        let mut pair = Pair::new(
            &image,
            |bus| {
                set_waitcnt(bus, waitcnt);
                set_dispcnt(bus, dispcnt);
                set_interrupts(bus, irqs);
                bus.iwram[0x4000..0x6000].copy_from_slice(&iwram);
                bus.ewram[0x8000..0xa000].copy_from_slice(&ewram);
                if pc >> 24 < 8 {
                    for (i, &half) in code.iter().enumerate() {
                        let address = first + 2 * i as u32;
                        let index = (address & 0x00ff_ffff) as usize;
                        let ram: &mut [u8] = if address >> 24 == 3 {
                            &mut bus.iwram[..]
                        } else {
                            &mut bus.ewram[..]
                        };
                        let index = index & (ram.len() - 1);
                        ram[index..index + 2].copy_from_slice(&half.to_le_bytes());
                    }
                }
            },
            cpu,
            lead(&mut random),
        );
        // Enter with the buffer filling, full, or stopped.
        match random.below(4) {
            0 => {}
            1 => {
                let pause = 1 + u64::from(random.below(80));
                pair.engine_bus.elapse(pause);
                pair.reference_bus.elapse(pause);
            }
            2 => {
                let pause = 1 + u64::from(random.below(12));
                pair.engine_bus.elapse(pause);
                pair.reference_bus.elapse(pause);
            }
            _ => {
                let kind = if random.below(2) == 0 {
                    AccessKind::Sequential
                } else {
                    AccessKind::Nonsequential
                };
                for bus in [&mut pair.engine_bus, &mut pair.reference_bus] {
                    bus.game_pak_prefetch = Default::default();
                    bus.fetch_kind = kind;
                }
            }
        }
        for attempt in 0..6 {
            let budget = 1 + u64::from(random.below(if attempt == 0 { 30 } else { 300 }));
            let yield_at = pair.engine_bus.now() + budget;
            let before = pair.engine_bus.now();
            let prefetch =
                pair.engine.core.next_instruction_address >> 24 >= 8 && waitcnt & 0x4000 != 0;
            if pair.compare(yield_at, &format!("trial {trial} attempt {attempt}")) {
                runs += 1;
                cycles += pair.engine_bus.now() - before;
                prefetch_runs += usize::from(prefetch);
            } else {
                refusals += 1;
                let outcome = pair.engine.step(&mut pair.engine_bus);
                let reference = pair.reference.step(&mut pair.reference_bus);
                assert_eq!(outcome, reference, "trial {trial}");
                pair.assert_equal(&format!("trial {trial} step after refusal"));
                if !matches!(outcome, StepOutcome::Executed(_)) {
                    break;
                }
            }
        }
        irq_entries += pair.irq_entries;
        pair.settle(&format!("trial {trial}"));
    }
    assert!(runs > 5000, "only {runs} runs ({refusals} refusals)");
    assert!(cycles > 60_000, "only {cycles} cycles in engine runs");
    assert!(prefetch_runs > 1000, "only {prefetch_runs} runs with the prefetch buffer");
    let video = super::mem::VIDEO_STORES.get();
    assert!(video > 100, "only {video} video memory stores in engine runs");
    let mmio = super::MMIO_ACCESSES.get();
    assert!(mmio > 150, "only {mmio} MMIO accesses in engine runs");
    assert!(irq_entries <= runs, "{irq_entries} IRQs");
}

/// Interrupt-handler-shaped code: loads of IF, VCOUNT and DISPSTAT, an
/// affine reference point store, an IF acknowledge, IE and IME stores that
/// raise an IRQ (which the run stops at, and the IRQ entry follows at once)
/// or are masked by the CPSR's I bit, and HALTCNT, in ARM code from IWRAM
/// and Thumb code from IWRAM and Game Pak ROM with and without the prefetch
/// buffer, from leads across lines, HBlank and VBlank, match stepping.
#[test]
fn mmio_transfers_in_runs_match_stepping() {
    // r4 = 0x04000000, r5 = 0x04000200, r6 = IME value, r7 = IE value.
    let arm = [
        0xe283_3001_u32, // ADD r3, r3, #1
        0xe1d5_00b2,     // LDRH r0, [r5, #2]   IF
        0xe594_1004,     // LDR r1, [r4, #4]    DISPSTAT and VCOUNT
        0xe584_203c,     // STR r2, [r4, #0x3c] BG3X
        0xe1c5_20b2,     // STRH r2, [r5, #2]   acknowledge some of IF
        0xe1c5_60b8,     // STRH r6, [r5, #8]   IME
        0xe283_3001,     // ADD r3, r3, #1
        0xe1c5_70b0,     // STRH r7, [r5]       IE
        0xe1d5_10b0,     // LDRH r1, [r5]       IE
        0xe5c4_8301,     // STRB r8, [r4, #0x301] HALTCNT (r8 = 0x80: no-op below)
        0xeaff_fff4,     // B back to the first
    ];
    let thumb = [
        0x3301_u16, // ADDS r3, #1
        0x8868,     // LDRH r0, [r5, #2]   IF
        0x6861,     // LDR r1, [r4, #4]    DISPSTAT and VCOUNT
        0x63e2,     // STR r2, [r4, #0x3c] BG3X
        0x806a,     // STRH r2, [r5, #2]   acknowledge some of IF
        0x812e,     // STRH r6, [r5, #8]   IME
        0x3301,     // ADDS r3, #1
        0x802f,     // STRH r7, [r5]       IE
        0x8829,     // LDRH r1, [r5]       IE
        0x88e1,     // LDRH r1, [r4, #6]   VCOUNT
        0xe7f4,     // B back to the first
    ];
    let mut random = Random(0x3a3a_1717);
    let mut rom = vec![0u8; 0x1000];
    for (i, half) in thumb.iter().enumerate() {
        rom[0x100 + 2 * i..0x102 + 2 * i].copy_from_slice(&half.to_le_bytes());
    }
    let (mut runs, mut irq_entries, mut halts) = (0, 0, 0);
    let mmio_before = super::MMIO_ACCESSES.get();
    for trial in 0..600 {
        let place = trial % 4;
        let (pc, is_thumb) = match place {
            0 => (0x0300_1000, false),
            1 => (0x0300_1000, true),
            _ => (0x0800_0100, true),
        };
        let waitcnt = if place == 3 { 0x0317 } else { 0x4317 };
        let masked = random.below(4) == 0;
        let mut cpu = Cpu::init(
            pc,
            0x1f | if is_thumb { 0x20 } else { 0 } | if masked { 0x80 } else { 0 },
            true,
        );
        cpu.registers[2] = random.next();
        cpu.registers[4] = 0x0400_0000;
        cpu.registers[5] = 0x0400_0200;
        cpu.registers[6] = random.below(2);
        cpu.registers[7] = random.next() & 0x3fff;
        cpu.registers[8] = if random.below(3) == 0 { 0 } else { 0x80 };
        let irqs = interrupts(&mut random);
        let lead = lead(&mut random);
        let mut pair = Pair::new(
            &rom,
            |bus| {
                set_waitcnt(bus, waitcnt);
                set_interrupts(bus, irqs);
                // HBlank, VBlank and VCOUNT interrupts on line 100.
                set_io(bus, 0x0400_0004, 0x6438);
                if is_thumb {
                    for (i, half) in thumb.iter().enumerate() {
                        let at = 0x1000 + 2 * i;
                        bus.iwram[at..at + 2].copy_from_slice(&half.to_le_bytes());
                    }
                } else {
                    for (i, &word) in arm.iter().enumerate() {
                        write_word(bus, 0x0300_1000 + 4 * i as u32, word);
                    }
                }
            },
            cpu,
            lead,
        );
        let context = format!("trial {trial} place {place} lead {lead} masked {masked}");
        for attempt in 0..8 {
            let yield_at = pair.engine_bus.now() + 1 + u64::from(random.below(600));
            let entries = pair.irq_entries;
            if pair.compare(yield_at, &format!("{context} attempt {attempt}")) {
                runs += 1;
                irq_entries += pair.irq_entries - entries;
            } else {
                let outcome = pair.engine.step(&mut pair.engine_bus);
                let reference = pair.reference.step(&mut pair.reference_bus);
                assert_eq!(outcome, reference, "{context}");
                pair.assert_equal(&format!("{context} step after refusal"));
                if !matches!(outcome, StepOutcome::Executed(_)) {
                    halts += 1;
                    break;
                }
            }
        }
        pair.settle(&context);
    }
    let mmio = super::MMIO_ACCESSES.get() - mmio_before;
    assert!(runs > 1500, "only {runs} runs");
    assert!(mmio > 50_000, "only {mmio} MMIO accesses in engine runs");
    assert!(irq_entries > 80, "only {irq_entries} IRQs raised in engine runs");
    assert!(halts > 40, "only {halts} halts");
}
