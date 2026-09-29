//! Engine-versus-interpreter differentials: the interpreter's `step` is the
//! semantic and timing reference for every instruction the engine runs.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::{Cpu, StepOutcome};

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
        Self { engine, engine_bus, reference, reference_bus }
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
        let end = self.engine_bus.now();
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
        assert_eq!(a.open_bus_value, b.open_bus_value, "{context}: open bus");
        assert_eq!(a.fetch_kind, b.fetch_kind, "{context}: fetch kind");
        assert_eq!(a.game_pak_prefetch, b.game_pak_prefetch, "{context}: prefetch");
        assert_eq!(a.game_pak_code_active, b.game_pak_code_active, "{context}: code active");
        assert!(a.iwram[..] == b.iwram[..], "{context}: IWRAM");
        assert!(a.ewram[..] == b.ewram[..], "{context}: EWRAM");
    }
}

fn set_waitcnt(bus: &mut Bus, value: u16) {
    bus.write(
        Access {
            address: 0x0400_0204,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        },
        u32::from(value),
    );
}

/// A register value: small numbers, RAM and ROM pointers, or noise.
fn operand(random: &mut Random) -> u32 {
    match random.below(10) {
        0..=2 => random.below(64),
        3 => 0x0300_4000 + (random.next() & 0x0ffc),
        4 => 0x0200_8000 + (random.next() & 0x0fff),
        5 => 0x0800_0000 + (random.next() & 0x3ffe),
        6 => 0x0300_4000 + random.below(64),
        7 => 1 << random.below(32),
        _ => random.next(),
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
    cpu.registers[10] = 0x0800_0000 + (random.next() & 0xffe);
    cpu.registers[11] = 0x0300_4000 + (random.next() & 0xfff);
    cpu
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
    let (mut runs, mut refusals, mut instructions) = (0, 0, 0);
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
        let mut pair = Pair::new(
            &rom,
            |bus| {
                set_waitcnt(bus, waitcnt);
                bus.iwram[0x4000..0x6000].copy_from_slice(&iwram);
                bus.ewram[0x8000..0xa000].copy_from_slice(&ewram);
                for &(address, word) in &block {
                    write_word(bus, address, word);
                }
            },
            cpu,
            1 + u64::from(random.below(1200)),
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
    }
    assert!(runs > 4000, "only {runs} runs ({refusals} refusals)");
    assert!(instructions > 40_000, "only {instructions} cycles in engine runs");
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
    cpu.registers[13] = 0x0300_5000 + (random.next() & 0x3fc);
    cpu.registers[14] = pc.wrapping_add(random.below(64) * 2) | 1;
    cpu
}

/// Thumb code from IWRAM, EWRAM and Game Pak ROM without prefetch: every
/// run of random code, flags, wait states, horizons and page edges matches
/// single stepping.
#[test]
fn thumb_engine_matches_interpreter_for_random_code() {
    let mut random = Random(0x7eed_7b01);
    let mut rom = vec![0u8; 0x2_0000];
    for byte in &mut rom {
        *byte = random.next() as u8;
    }
    let (mut runs, mut refusals, mut cycles) = (0, 0, 0);
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
        let waitcnt = random.next() as u16 & 0x3fff;
        let mut pair = Pair::new(
            &image,
            |bus| {
                set_waitcnt(bus, waitcnt);
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
            1 + u64::from(random.below(1200)),
        );
        for attempt in 0..6 {
            let budget = 1 + u64::from(random.below(if attempt == 0 { 30 } else { 300 }));
            let yield_at = pair.engine_bus.now() + budget;
            let before = pair.engine_bus.now();
            if pair.compare(yield_at, &format!("trial {trial} attempt {attempt}")) {
                runs += 1;
                cycles += pair.engine_bus.now() - before;
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
    }
    assert!(runs > 5000, "only {runs} runs ({refusals} refusals)");
    assert!(cycles > 60_000, "only {cycles} cycles in engine runs");
}
