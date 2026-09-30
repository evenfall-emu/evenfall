//! BIOS read protection with an external BIOS image.
//!
//! GBATEK (Unpredictable Things, Reading from BIOS Memory): the BIOS answers
//! reads only while the program counter is inside it; any other read of
//! `00000000h`–`00003FFFh` returns the most recent opcode fetched from the
//! BIOS. The CPU's pipeline fetches two instructions ahead, so a branch out
//! of the BIOS at address A leaves the opcode at A+8 (ARM) or the BIOS word
//! holding A+4 (Thumb). The expected values below follow from those rules
//! and the synthetic images, not from the emulator.

use evenfall_gba::bus::{AccessKind, AccessWidth, Bus};
use evenfall_gba::machine::{BootMode, Config, Machine, StepOutcome};

const ROM: u32 = 0x0800_0000;
const IWRAM: u32 = 0x0300_0000;

/// The literal BIOS code loads, and the markers two slots after each exit.
const LITERAL: u32 = 0x1234_5678;
const EXIT_PLUS_4: u32 = 0xc0de_0110;
const EXIT_PLUS_8: u32 = 0xc0de_0114;
const RETURN_PLUS_8: u32 = 0xc0de_0208;
const THUMB_EXIT_WORD: u32 = 0xc0de_0304;

fn put(image: &mut [u8], at: u32, words: &[u32]) {
    for (i, word) in words.iter().enumerate() {
        let at = at as usize + 4 * i;
        image[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
}

fn put_halves(image: &mut [u8], at: u32, halves: &[u16]) {
    for (i, half) in halves.iter().enumerate() {
        let at = at as usize + 2 * i;
        image[at..at + 2].copy_from_slice(&half.to_le_bytes());
    }
}

/// A BIOS that reads its own literal, then leaves with `BX r12` for
/// `target`, and has a subroutine at 0x200 that returns with `BX lr`.
fn arm_bios(target: u32) -> Vec<u8> {
    let mut bios = vec![0; 0x4000];
    put(&mut bios, 0, &[0xea00_003e]); // B 0x100
    let mov_r12 = match target {
        ROM => 0xe3a0_c302,   // MOV r12, #0x08000000
        IWRAM => 0xe3a0_c403, // MOV r12, #0x03000000
        _ => unreachable!(),
    };
    put(
        &mut bios,
        0x100,
        &[
            0xe59f_1078, // LDR r1, [pc, #0x78]    the literal at 0x180
            0xe5df_3075, // LDRB r3, [pc, #0x75]   its byte 1
            mov_r12,
            0xe12f_ff1c, // BX r12                 fetches 0x114 as it runs
            EXIT_PLUS_4,
            EXIT_PLUS_8,
        ],
    );
    put(&mut bios, 0x180, &[LITERAL]);
    // BX lr, which fetches 0x208 as it runs.
    put(&mut bios, 0x200, &[0xe12f_ff1e, 0xc0de_0204, RETURN_PLUS_8]);
    bios
}

/// ARM code that reads the BIOS outside it, calls the BIOS subroutine and
/// reads again, then parks at `+0x24`.
const READER: [u32; 10] = [
    0xe3a0_0000, // MOV r0, #0
    0xe590_4000, // LDR r4, [r0]
    0xe590_5180, // LDR r5, [r0, #0x180]  the literal's address
    0xe1d0_60b2, // LDRH r6, [r0, #2]
    0xe5d0_7181, // LDRB r7, [r0, #0x181]
    0xe3a0_9c02, // MOV r9, #0x200
    0xe1a0_e00f, // MOV lr, pc
    0xe12f_ff19, // BX r9
    0xe590_8000, // LDR r8, [r0]
    0xeaff_fffe, // B .
];

fn reader_machine(target: u32, bios: &[u8]) -> Box<Machine> {
    let mut rom = vec![0; 0x100];
    if target == ROM {
        put(&mut rom, 0, &READER);
    }
    let mut machine =
        Machine::new(Config { rom: &rom, bios, boot: BootMode::ExternalBios, ..Config::default() });
    if target == IWRAM {
        put(&mut machine.bus.iwram[..], 0, &READER);
    }
    machine
}

/// The gate that opens the image to data reads records where the CPU
/// fetched last; at an instruction boundary that follows from the CPU.
fn assert_gate_follows_cpu(machine: &Machine, context: &str) {
    assert_eq!(
        machine.bus.bios_code_active,
        machine.cpu.last_fetch_in_bios(),
        "{context}: gate at {:#x}",
        machine.next_instruction_address()
    );
}

fn step_to(machine: &mut Machine, park: u32) {
    for _ in 0..1000 {
        assert_gate_follows_cpu(machine, "stepping");
        if machine.next_instruction_address() == park {
            return;
        }
        assert!(matches!(machine.step(), StepOutcome::Executed(_)));
    }
    panic!("never reached {park:#x}; at {:#x}", machine.next_instruction_address());
}

fn assert_reader_results(machine: &Machine, context: &str) {
    let registers: Vec<u32> = [1, 3, 4, 5, 6, 7, 8].map(|r| machine.read_register(r)).to_vec();
    assert_eq!(
        registers,
        [
            LITERAL,                 // r1: BIOS code reads its own bytes
            0x56,                    // r3: byte 1 of the literal
            EXIT_PLUS_8,             // r4: the opcode fetched while BX r12 ran
            EXIT_PLUS_8,             // r5: any address, the same latch
            EXIT_PLUS_8 >> 16,       // r6: the latch's upper half
            EXIT_PLUS_8 >> 8 & 0xff, // r7: the latch's byte 1
            RETURN_PLUS_8,           // r8: after the subroutine's BX lr
        ],
        "{context}: r1 r3 r4 r5 r6 r7 r8"
    );
}

/// Reads of the BIOS from ROM and from IWRAM code return the opcode the BIOS
/// fetched last, whether the machine steps or runs; BIOS code reads its own
/// bytes.
#[test]
fn reads_from_outside_the_bios_return_the_last_fetched_bios_opcode() {
    for target in [ROM, IWRAM] {
        let bios = arm_bios(target);
        let park = target + 0x24;

        let mut stepped = reader_machine(target, &bios);
        step_to(&mut stepped, park);
        assert_reader_results(&stepped, &format!("stepped to {target:#x}"));

        let mut run = reader_machine(target, &bios);
        run.run_until(20_000).unwrap();
        assert_eq!(run.next_instruction_address(), park);
        assert_gate_follows_cpu(&run, "run");
        assert_reader_results(&run, &format!("run to {target:#x}"));
    }
}

/// A Thumb exit latches the whole BIOS word that holds the opcode two
/// halfwords on: the BIOS bus is 32 bits wide. ROM code turns the Game Pak
/// prefetch on and calls the BIOS, whose exit into Thumb ROM then completes
/// its refill in one step when the machine runs.
#[test]
fn a_thumb_exit_latches_the_bios_word_holding_its_prefetch() {
    let mut bios = vec![0; 0x4000];
    put(&mut bios, 0, &[0xea00_003e]); // B 0x100
    put(
        &mut bios,
        0x100,
        &[
            0xe3a0_c302, // MOV r12, #0x08000000
            0xe12f_ff1c, // BX r12
        ],
    );
    put_halves(
        &mut bios,
        0x300,
        &[
            0x4901, // LDR r1, [pc, #4]   ROM + 0x21
            0x4708, // BX r1              fetches 0x306 as it runs
        ],
    );
    put(&mut bios, 0x304, &[THUMB_EXIT_WORD, ROM + 0x21]);
    let mut rom = vec![0; 0x100];
    put(
        &mut rom,
        0,
        &[
            0xe3a0_2301, // MOV r2, #0x04000000
            0xe282_2c02, // ADD r2, r2, #0x200
            0xe3a0_3901, // MOV r3, #0x4000
            0xe1c2_30b4, // STRH r3, [r2, #4]    WAITCNT: prefetch on
            0xe59c_6000, // LDR r6, [r12]        a ROM access at the new timing
            0xe3a0_cc03, // MOV r12, #0x300
            0xe38c_c001, // ORR r12, r12, #1
            0xe12f_ff1c, // BX r12
        ],
    );
    put_halves(
        &mut rom,
        0x20,
        &[
            0x2000, // MOV r0, #0
            0x6804, // LDR r4, [r0]
            0xe7fe, // B .
        ],
    );
    let config =
        Config { rom: &rom, bios: &bios, boot: BootMode::ExternalBios, ..Config::default() };
    let park = ROM + 0x24;

    let mut stepped = Machine::new(config);
    step_to(&mut stepped, park);
    assert_eq!(stepped.read_register(4), THUMB_EXIT_WORD, "stepped");

    let mut run = Machine::new(Config { ..config });
    // Stop at every boundary, the one after the fused refill included.
    while run.now() < 2_000 {
        let deadline = run.now() + 1;
        run.run_until(deadline).unwrap();
        assert_gate_follows_cpu(&run, &format!("run to {deadline}"));
    }
    assert_eq!(run.next_instruction_address(), park);
    assert_eq!(run.read_register(4), THUMB_EXIT_WORD, "run");
}

/// A state saved while the CPU runs BIOS code restores the BIOS's view of
/// itself: the next instruction, a load of a BIOS literal, reads the image.
#[test]
fn a_state_saved_inside_the_bios_keeps_bios_reads_open() {
    let bios = arm_bios(ROM);
    let mut source = reader_machine(ROM, &bios);
    source.step(); // B 0x100
    assert_eq!(source.next_instruction_address(), 0x100);
    let mut state = Vec::new();
    source.write_state(&mut state).unwrap();

    let mut restored = reader_machine(ROM, &bios);
    restored.load_state(&state).unwrap();
    assert_gate_follows_cpu(&restored, "restored");
    restored.step(); // LDR r1, [pc, #0x78]
    assert_eq!(restored.read_register(1), LITERAL);
}

/// The gate follows the CPU's last opcode fetch: CPU data reads see the
/// image after a BIOS fetch and the latch after any other. A BIOS fetch
/// latches its aligned word whatever its width; data reads never change the
/// latch. DMA never reads the BIOS: a source below EWRAM presents the DMA's
/// own data latch, wherever the CPU runs.
#[test]
fn the_bios_gate_follows_the_last_opcode_fetch_and_dma_never_reads_the_bios() {
    let mut bios = vec![0; 0x4000];
    for (i, word) in bios.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        *word = (0xb105_0000 | (i as u32 * 4)).to_le_bytes();
    }
    let mut bus = Bus::init_with_bios(&[], &bios);
    let data = |bus: &mut Bus, address, width| {
        bus.read_cpu_data(address, width, AccessKind::Nonsequential)
    };

    bus.fetch_instruction(0x102, AccessWidth::Half);
    assert_eq!(bus.bios_latch, 0xb105_0100);
    assert_eq!(data(&mut bus, 0x2000, AccessWidth::Word), 0xb105_2000);
    assert_eq!(data(&mut bus, 0x2002, AccessWidth::Half), 0xb105);
    assert_eq!(bus.bios_latch, 0xb105_0100);
    dma_word_from(&mut bus, 0x100, 0xd1a5_0001);
    assert_eq!(read_iwram_word(&bus, 0x300), 0xd1a5_0001);
    assert_eq!(bus.bios_latch, 0xb105_0100);

    bus.fetch_instruction(IWRAM + 0x40, AccessWidth::Word);
    assert_eq!(data(&mut bus, 0x2000, AccessWidth::Word), 0xb105_0100);
    assert_eq!(data(&mut bus, 0x2003, AccessWidth::Byte), 0xb1);
    dma_word_from(&mut bus, 0x100, 0xd1a5_0002);
    assert_eq!(read_iwram_word(&bus, 0x300), 0xd1a5_0002);
    assert_eq!(bus.bios_latch, 0xb105_0100);
}

/// Runs one immediate DMA3 word from `source` to IWRAM `0x03000300`, with
/// the DMA data latch holding `latch`.
fn dma_word_from(bus: &mut Bus, source: u32, latch: u32) {
    bus.dma_transfer_latch = latch;
    let write_half = |bus: &mut Bus, address: u32, value: u16| {
        bus.write_cpu_data(
            evenfall_gba::bus::Access {
                address,
                width: AccessWidth::Half,
                kind: AccessKind::Nonsequential,
                source: evenfall_gba::bus::AccessSource::CpuData,
            },
            u32::from(value),
        );
    };
    for (address, value) in [
        (0x0400_00d4, source as u16),
        (0x0400_00d6, (source >> 16) as u16),
        (0x0400_00d8, 0x0300),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 1),
        (0x0400_00de, (1 << 10) | (1 << 15)),
    ] {
        write_half(bus, address, value);
    }
    bus.elapse(3);
}

fn read_iwram_word(bus: &Bus, offset: usize) -> u32 {
    u32::from_le_bytes(bus.iwram[offset..offset + 4].try_into().unwrap())
}
