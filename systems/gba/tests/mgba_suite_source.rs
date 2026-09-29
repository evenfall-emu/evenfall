//! Hardware table from pinned mgba-emu/suite src/dma.c (MIT).
//! Each row primes the shared DMA latch with a halfword transfer, then checks
//! the seven destination words after the measured four-unit transfer.

use evenfall_gba::{
    bus::{Access, AccessKind, AccessSource, AccessWidth, Bus},
    cartridge::save::Type,
};

const SRC: [u32; 7] =
    [0xcafe_babb, 0xcafe_babc, 0xcafe_babd, 0xcafe_babe, 0xcafe_babf, 0xcafe_bac0, 0xcafe_bac1];
const ROM: [u32; 8] = [
    0xdead_beec,
    0xdead_beed,
    0xdead_beee,
    0xdead_beef,
    0xdead_bef0,
    0xdead_bef1,
    0xdead_bef2,
    0xdead_bef3,
];
const SRC_BASE: u32 = 0x0300_0000;
const DST_BASE: u32 = 0x0300_0100;
const IWRAM_SOURCE: u32 = SRC_BASE + 12;
const ROM_SOURCE: u32 = 0x0800_000c;

fn write(bus: &mut Bus, width: AccessWidth, address: u32, value: u32) {
    let _ = bus.write(
        Access { address, width, kind: AccessKind::Nonsequential, source: AccessSource::CpuData },
        value,
    );
}
fn read_word(bus: &mut Bus, address: u32) -> u32 {
    bus.read(Access {
        address,
        width: AccessWidth::Word,
        kind: AccessKind::Nonsequential,
        source: AccessSource::CpuData,
    })
    .value
}
fn run_dma(bus: &mut Bus, channel: usize, source: u32, destination: u32, control: u32) {
    let base = 0x0400_00b0 + channel as u32 * 12;
    write(bus, AccessWidth::Half, base, source & 0xffff);
    write(bus, AccessWidth::Half, base + 2, source >> 16);
    write(bus, AccessWidth::Half, base + 4, destination & 0xffff);
    write(bus, AccessWidth::Half, base + 6, destination >> 16);
    write(bus, AccessWidth::Half, base + 8, 4);
    write(bus, AccessWidth::Half, base + 10, control >> 16);
    bus.elapse(64);
}

#[test]
fn suite_immediate_dma_rows_leave_the_recorded_destination_word() {
    // src/dma.c rows: the suite's exact seven destination words, including
    // unchanged neighbors. DMA0's narrower source mask and ROM sequential
    // reads make the channel 0/1 pairs deliberately asymmetric.
    let rows: [(&str, usize, u32, u32, u32); 15] = [
        ("0 Imm H =IWRAM/=IWRAM (line 57)", 0, IWRAM_SOURCE, 0x8140_0004, 0xcb0e_babe),
        ("1 Imm H =IWRAM/=IWRAM (line 58)", 1, IWRAM_SOURCE, 0x8140_0004, 0xcb0e_babe),
        ("2 Imm H =IWRAM/=IWRAM (line 59)", 2, IWRAM_SOURCE, 0x8140_0004, 0xcb0e_babe),
        ("3 Imm H =IWRAM/=IWRAM (line 60)", 3, IWRAM_SOURCE, 0x8140_0004, 0xcb0e_babe),
        ("0 Imm H +IWRAM/=IWRAM (line 177)", 0, IWRAM_SOURCE, 0x8040_0004, 0xcb0e_cafe),
        ("0 Imm H -IWRAM/=IWRAM (line 297)", 0, IWRAM_SOURCE, 0x80c0_0004, 0xcb0e_cafe),
        ("0 Imm W =IWRAM/=IWRAM (line 117)", 0, IWRAM_SOURCE, 0x8540_0004, 0xcafe_babe),
        ("0 Imm W +IWRAM/=IWRAM (line 237)", 0, IWRAM_SOURCE, 0x8440_0004, 0xcafe_bac1),
        ("0 Imm W -IWRAM/=IWRAM (line 357)", 0, IWRAM_SOURCE, 0x84c0_0004, 0xcafe_babb),
        ("0 Imm H =ROM/=IWRAM (line 81)", 0, ROM_SOURCE, 0x8140_0004, 0xcb0e_babe),
        ("1 Imm H =ROM/=IWRAM (line 82)", 1, ROM_SOURCE, 0x8140_0004, 0xcb0e_dead),
        ("0 Imm W =ROM/=IWRAM (line 141)", 0, ROM_SOURCE, 0x8540_0004, 0xbabe_babe),
        ("1 Imm W =ROM/=IWRAM (line 142)", 1, ROM_SOURCE, 0x8540_0004, 0xdead_bef2),
        ("0 Imm H =BIOS/=IWRAM (line 93)", 0, 0, 0x8140_0004, 0xcb0e_babe),
        ("0 Imm W =BIOS/=IWRAM (line 153)", 0, 0, 0x8540_0004, 0xbabe_babe),
    ];
    let mut rom = vec![0; 32];
    for (index, word) in ROM.iter().enumerate() {
        rom[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    for (name, channel, source, control, word3) in rows {
        let mut bus = Bus::init_with_save(&rom, Type::Sram);
        for (index, word) in SRC.iter().enumerate() {
            write(&mut bus, AccessWidth::Word, SRC_BASE + index as u32 * 4, *word);
        }
        run_dma(&mut bus, channel, IWRAM_SOURCE, DST_BASE + 12, control & !0x0400_0000);
        for (index, word) in SRC.iter().enumerate() {
            write(
                &mut bus,
                AccessWidth::Word,
                DST_BASE + index as u32 * 4,
                word.wrapping_add(0x100010),
            );
        }
        run_dma(&mut bus, channel, source, DST_BASE + 12, control);
        for (index, word) in SRC.iter().enumerate() {
            let expected = if index == 3 { word3 } else { word.wrapping_add(0x100010) };
            assert_eq!(
                read_word(&mut bus, DST_BASE + index as u32 * 4),
                expected,
                "{name} word {index}"
            );
        }
    }
}

#[test]
fn suite_dma0_aliases_an_sram_source_while_dma1_reads_sram_bytes() {
    // The pinned suite's SRAM rows show a difference but depend on the suite's
    // previous contents. Controlled SRAM A7 and VRAM 1234 isolate that rule.
    for (channel, expected) in [(1, 0xa7a7_u16), (0, 0x1234)] {
        let mut bus = Bus::init_with_save(&[], Type::Sram);
        write(&mut bus, AccessWidth::Byte, 0x0e00_0000, 0xa7);
        write(&mut bus, AccessWidth::Half, 0x0600_0000, 0x1234);
        run_dma(&mut bus, channel, 0x0e00_0000, DST_BASE, 0x8140_0004);
        assert_eq!(read_word(&mut bus, DST_BASE) as u16, expected, "DMA{channel}");
    }
}
