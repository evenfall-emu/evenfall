//! Oracles come from outside this code: GBATEK's code tables and seeds, the
//! published TEA test vector, and cheat codes published both encrypted and
//! in raw form. A master code's `xxxxxxxx 001DC0DE` line must decrypt to the
//! game code in the cartridge header, which fixes the expected words
//! independently of the cipher.

use super::crypt::{CodeBreakerCipher, Tea};
use super::program::{Cond, Memory, Op, Skip, Update, Width};
use super::*;
use std::collections::BTreeMap;

fn cheat(text: &str, format: Format) -> Cheat {
    Cheat::parse(text, Some(format)).unwrap_or_else(|error| panic!("{text}: {error}"))
}

/// Four ASCII characters as the little-endian word at 0x080000AC.
fn game_code(code: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*code)
}

#[test]
fn tea_matches_the_published_zero_key_vector() {
    // Wheeler and Needham's TEA: key 0, plaintext 0 -> 41EA3A0A 94BAA940.
    let tea = Tea::with_seeds([0; 4]);
    assert_eq!(tea.encrypt(0, 0), (0x41ea_3a0a, 0x94ba_a940));
    assert_eq!(tea.decrypt(0x41ea_3a0a, 0x94ba_a940), (0, 0));
}

#[test]
fn action_replay_reseed_zero_gives_gbatek_startup_seeds() {
    // GBATEK: S0=7AA9648F S1=7FAE6994 S2=C0EFAAD5 S3=42712C57 at startup,
    // which DEADFACE 00000000 must reproduce from the T1/T2 tables.
    assert_eq!(Tea::action_replay_reseed(0), Tea::ACTION_REPLAY);
}

/// Pokémon Emerald (USA) master code and Infinite Money, published encrypted
/// and decrypted (wunkolo, "How do things like Gameshark and ActionReplay
/// codes work?"). The ID line is the header game code BPEE.
#[test]
fn action_replay_decrypts_published_emerald_codes() {
    let tea = Tea::ACTION_REPLAY;
    assert_eq!(tea.decrypt(0xd8ba_e4d9, 0x4864_dce5), (0xc400_05ec, 0x0000_8401));
    assert_eq!(tea.decrypt(0xa86c_dba5, 0x19ba_49b3), (game_code(b"BPEE"), 0x001d_c0de));
    assert_eq!(tea.decrypt(0xc051_ccf6, 0x975e_8da1), (0x0422_5e90, 0x000f_423f));

    let master = cheat("D8BAE4D9 4864DCE5\nA86CDBA5 19BA49B3", Format::ActionReplay);
    assert_eq!(master.ops, [Op::Hook(0x0800_05ec), Op::GameId(game_code(b"BPEE"))]);
    let money = cheat("C051CCF6 975E8DA1", Format::ActionReplay);
    assert_eq!(money.ops, [Op::write(0x0202_5e90, Width::Word, 999_999)]);
}

/// Master codes whose ID lines must decrypt to the German and US Ruby game
/// codes (libretro-database "Pokemon - Rubin-Edition (G)"; GameHacking.org
/// "Pokemon - Ruby Version (USA)", Action Replay MAX master).
#[test]
fn action_replay_master_codes_decrypt_to_their_game_codes() {
    let rubin = cheat("f57c7bcb+adc632b9+851b05e0594b5d88", Format::ActionReplay);
    assert_eq!(rubin.ops, [Op::Hook(0x0800_10ae), Op::GameId(game_code(b"AXVD"))]);
    let ruby = cheat("DE00AAFD 2EBD05D0\n530823D9 16558191", Format::ActionReplay);
    assert_eq!(ruby.ops, [Op::Hook(0x0800_0430), Op::GameId(game_code(b"AXVE"))]);
}

/// GameHacking.org, "Pokemon - Ruby Version (USA)": the GameShark Advance
/// master code. Its ID line must be AXVE, and its hook is the address the
/// US Action Replay master hooks.
#[test]
fn gameshark_decrypts_the_published_ruby_master_code() {
    let tea = Tea::GAMESHARK;
    assert_eq!(tea.decrypt(0x9e6a_c862, 0x823a_b7a8), (0xf800_0430, 0x0000_0101));
    assert_eq!(tea.decrypt(0x46b7_d9e4, 0xa709_e9e1), (game_code(b"AXVE"), 0x001d_c0de));
    let master = cheat("9E6AC862 823AB7A8\n46B7D9E4 A709E9E1", Format::GameShark);
    assert_eq!(master.ops, [Op::Hook(0x0800_0430), Op::GameId(game_code(b"AXVE"))]);
}

/// Encrypted CodeBreaker master codes published beside their raw forms:
/// Ruby (USA) by MadCatz on GameHacking.org against libretro-database's raw
/// `0000B138 000A 10000428 0007`, and an Emerald master on PokeCoders against
/// its raw `00006FA7 000A`. They also pin the step GBATEK omits.
#[test]
fn codebreaker_decrypts_published_master_codes() {
    let ruby = CodeBreakerCipher::new(0x9772_6cae, 0x9184);
    assert_eq!(ruby.decrypt(0x8f7a_aa5c, 0x15fe), (0x0000_b138, 0x000a));
    assert_eq!(ruby.decrypt(0xb85e_5a77, 0x0386), (0x1000_0428, 0x0007));

    let emerald = CodeBreakerCipher::new(0x9266_fa6c, 0x97bd);
    assert_eq!(emerald.decrypt(0x905b_5ed3, 0x5f81), (0x0000_6fa7, 0x000a));
    // The next published line of that list is a joypad test (GBATEK D0000020).
    assert_eq!(emerald.decrypt(0x10c6_fc61, 0xe2c9).0, 0xd000_0020);

    let parsed = cheat("97726CAE9184\n8F7AAA5C15FE\nB85E5A770386", Format::CodeBreaker);
    assert_eq!(parsed.ops, [Op::Nop, Op::Nop, Op::Hook(0x0800_0428)]);
}

#[test]
fn autodetection_picks_the_only_plausible_reading() {
    let detect = |text| Cheat::parse(text, None).map(|cheat| cheat.format());
    assert_eq!(detect("D8BAE4D9 4864DCE5 A86CDBA5 19BA49B3"), Ok(Format::ActionReplay));
    assert_eq!(detect("9E6AC862 823AB7A8\n46B7D9E4 A709E9E1"), Ok(Format::GameShark));
    // libretro-database's FireRed master and money codes are raw AR v3.
    assert_eq!(detect("C4000604+00008401+45525042+001DC0DE"), Ok(Format::ActionReplayDecrypted));
    assert_eq!(detect("04225E90 000F423F"), Ok(Format::ActionReplayDecrypted));
    assert_eq!(detect("82025BC4 E0FF"), Ok(Format::CodeBreaker));
    assert_eq!(detect("02025E90:000F423F"), Ok(Format::Raw));
    assert_eq!(detect("0000B138000A 10000428 0007"), Ok(Format::CodeBreaker));
    // A raw AR add is also a raw GameShark device-button write, which
    // detection discounts.
    assert_eq!(detect("84200000 00000001"), Ok(Format::ActionReplayDecrypted));
}

#[test]
fn parser_rejects_malformed_and_unsupported_codes() {
    let error = |text, format| Cheat::parse(text, format).unwrap_err().kind;
    assert_eq!(error("", None), ErrorKind::Empty);
    assert_eq!(error("# only a comment", None), ErrorKind::Empty);
    assert_eq!(error("0202XYZ0 0001", None), ErrorKind::Syntax);
    assert_eq!(error("02024EA4 0063 02024EA4 00000063", None), ErrorKind::MixedLines);
    assert_eq!(error("82025BC4 E0FF", Some(Format::ActionReplay)), ErrorKind::WrongLineShape);
    // GBATEK's ROM patch types, raw.
    assert_eq!(
        error("00000000 1802603C 00002100 00000000", Some(Format::ActionReplayDecrypted)),
        ErrorKind::RomPatch
    );
    assert_eq!(error("60012345 00001234", Some(Format::GameSharkDecrypted)), ErrorKind::RomPatch);
    // The GameShark group write needs its listed addresses.
    assert_eq!(
        error("30000004 01010101 03001FF0 03001FF4", Some(Format::GameSharkDecrypted)),
        ErrorKind::MissingParameters
    );
    // An 8-bit AR fill of 0x1000000 bytes.
    assert_eq!(
        error("00224EA4 FFFFFF00", Some(Format::ActionReplayDecrypted)),
        ErrorKind::TooLarge
    );
    let many = "02000000:00\n".repeat(MAX_LINES + 1);
    assert_eq!(error(&many, None), ErrorKind::TooManyLines);
}

#[test]
fn canonical_text_round_trips() {
    for (text, format) in [
        ("d8bae4d9 4864dce5+a86cdba5 19ba49b3", Format::ActionReplay),
        ("97726cae9184 8f7aaa5c15fe", Format::CodeBreaker),
        ("2024ea4:63, 3000000:1234", Format::Raw),
    ] {
        let parsed = cheat(text, format);
        assert_eq!(cheat(&parsed.code(), format), parsed);
    }
    assert_eq!(cheat("2024ea4:63", Format::Raw).code(), "02024EA4:63");
}

/// Sparse byte memory for interpreter checks.
#[derive(Default)]
struct Bytes(BTreeMap<u32, u8>);

impl Memory for Bytes {
    fn read(&self, address: u32, width: Width) -> u32 {
        let address = address & !(width.bytes() - 1);
        (0..width.bytes()).fold(0, |value, offset| {
            value | u32::from(*self.0.get(&(address + offset)).unwrap_or(&0)) << (8 * offset)
        })
    }
    fn write(&mut self, address: u32, width: Width, value: u32) {
        let address = address & !(width.bytes() - 1);
        for offset in 0..width.bytes() {
            self.0.insert(address + offset, (value >> (8 * offset)) as u8);
        }
    }
}

impl Bytes {
    fn get(&self, address: u32, width: Width) -> u32 {
        Memory::read(self, address, width)
    }
}

/// The AR v3 raw forms of the Hacking Text's "AR V3 Codes Types" table, each
/// shown there for the address 02024EA4, and GBATEK's field layout.
#[test]
fn action_replay_code_types_follow_the_published_tables() {
    let ops = |text| cheat(text, Format::ActionReplayDecrypted).ops;
    let a = 0x0202_4ea4;
    // Type 00: fill 02024EA4..02024EA4+2 with byte 7F.
    assert_eq!(ops("00224EA4 0000027F"), [Op::fill(a, Width::Byte, 0x7f, 3)]);
    // Type 01: halfwords 02024EA4..+1*2.
    assert_eq!(ops("02224EA4 00011234"), [Op::fill(a, Width::Half, 0x1234, 2)]);
    assert_eq!(ops("04224EA4 13371337"), [Op::write(a, Width::Word, 0x1337_1337)]);
    // Types 20-22: pointer writes.
    assert_eq!(
        ops("40224EA4 00001005"),
        [Op::Pointer { pointer: a, offset: 0x10, width: Width::Byte, value: 5 }]
    );
    assert_eq!(
        ops("42224EA4 00030063"),
        [Op::Pointer { pointer: a, offset: 6, width: Width::Half, value: 0x63 }]
    );
    // Types 40-42: add.
    assert_eq!(
        ops("82224EA4 00000010"),
        [Op::Update { address: a, width: Width::Half, update: Update::Add, value: 0x10 }]
    );
    // Types 63 and E3: I/O writes to 04000130-style addresses.
    assert_eq!(ops("C6000130 000003FF"), [Op::write(0x0400_0130, Width::Half, 0x3ff)]);
    assert_eq!(ops("C7000200 00002001"), [Op::write(0x0400_0200, Width::Word, 0x2001)]);
    // GBATEK's example: ii = 18h+02h+40h = 5Ah, IF [a0aaaaa] < zzzz THEN
    // next 2 codes.
    assert_eq!(
        ops("5A224EA4 00000100"),
        [Op::If {
            address: a,
            width: Width::Half,
            cond: Cond::SignedLess,
            value: 0x100,
            skip: Skip::Next(2)
        }]
    );
    // Type 46: IF word equal THEN all following codes, until ELSE or ENDIF.
    assert_eq!(
        ops("8C224EA4 000000FF"),
        [Op::If {
            address: a,
            width: Width::Word,
            cond: Cond::Equal,
            value: 0xff,
            skip: Skip::Block
        }]
    );
    // "Always skip next line" (type 07).
    assert_eq!(
        ops("0E224EA4 00000000"),
        [Op::If {
            address: a,
            width: Width::Word,
            cond: Cond::Never,
            value: 0,
            skip: Skip::Next(1)
        }]
    );
    // Special codes: end, ELSE, ENDIF, slowdown, 16-bit slide.
    assert_eq!(ops("00000000 00000000"), [Op::Stop]);
    assert_eq!(ops("00000000 60000000 00000000 40000000"), [Op::Else, Op::EndIf]);
    assert_eq!(ops("00000000 08000400"), [Op::DeviceOnly]);
    assert_eq!(
        ops("00000000 82224EA4 00001000 01050002"),
        [Op::Slide {
            address: a,
            width: Width::Half,
            value: 0x1000,
            count: 5,
            address_step: 4,
            value_step: 1
        }]
    );
}

/// GameShark v1/v2 raw types from GBATEK, and the Hacking Text's group
/// write example: `30000004 01010101 03001FF0 03001FF4 03001FF8 00000000`
/// writes 01010101 to 01010101, 03001FF0, 03001FF4 and 03001FF8.
#[test]
fn gameshark_code_types_follow_the_published_tables() {
    let ops = |text| cheat(text, Format::GameSharkDecrypted).ops;
    assert_eq!(ops("02024EA4 00000063"), [Op::write(0x0202_4ea4, Width::Byte, 0x63)]);
    assert_eq!(ops("12024EA4 00001234"), [Op::write(0x0202_4ea4, Width::Half, 0x1234)]);
    assert_eq!(ops("22024EA4 12345678"), [Op::write(0x0202_4ea4, Width::Word, 0x1234_5678)]);
    assert_eq!(
        ops("30000004 01010101 03001FF0 03001FF4 03001FF8 00000000"),
        [Op::WriteList {
            addresses: [0x0101_0101, 0x0300_1ff0, 0x0300_1ff4, 0x0300_1ff8].into(),
            value: 0x0101_0101
        }]
    );
    assert_eq!(
        ops("D4000130 000003FE"),
        [Op::If {
            address: 0x0400_0130,
            width: Width::Half,
            cond: Cond::Equal,
            value: 0x3fe,
            skip: Skip::Next(1)
        }]
    );
    assert_eq!(
        ops("E0031234 02000000"),
        [Op::If {
            address: 0x0200_0000,
            width: Width::Half,
            cond: Cond::Equal,
            value: 0x1234,
            skip: Skip::Next(3)
        }]
    );
    assert_eq!(ops("80F00000 00000010 81100000 00000001"), [Op::DeviceOnly, Op::DeviceOnly]);
}

/// GBATEK's CodeBreaker table; the slide and super-code parameter layouts
/// from GBATEK and EnHacklopedia (`iiiicccc ssss`).
#[test]
fn codebreaker_code_types_follow_the_published_tables() {
    let ops = |text| cheat(text, Format::CodeBreaker).ops;
    // PokeCoders' Emerald "1st Pokemon Max Stats": 7 halfwords of 999.
    assert_eq!(
        ops("42024542 03E7\n00000007 0002"),
        [Op::Slide {
            address: 0x0202_4542,
            width: Width::Half,
            value: 999,
            count: 7,
            address_step: 2,
            value_step: 0
        }]
    );
    assert_eq!(
        ops("52000000 0008 11223344 5566 77880000 0000"),
        [Op::WriteBytes {
            address: 0x0200_0000,
            bytes: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88].into()
        }]
    );
    assert_eq!(ops("32026A60 0001"), [Op::write(0x0202_6a60, Width::Byte, 1)]);
    assert_eq!(
        ops("D0000020 0008"),
        [Op::If {
            address: 0x0400_0130,
            width: Width::Half,
            cond: Cond::NoneSet,
            value: 8,
            skip: Skip::Next(1)
        }]
    );
    let first = |text| ops(text)[0].clone();
    assert!(matches!(first("22000000 00F0"), Op::Update { update: Update::Or, .. }));
    assert!(matches!(first("62000000 00F0"), Op::Update { update: Update::And, .. }));
    assert!(matches!(first("E2000000 0001"), Op::Update { update: Update::Add, .. }));
    assert!(matches!(first("B2000000 0001"), Op::If { cond: Cond::SignedGreater, .. }));
    assert!(matches!(first("C2000000 0001"), Op::If { cond: Cond::SignedLess, .. }));
    assert!(matches!(first("F2000000 0001"), Op::If { cond: Cond::AnySet, .. }));
}

#[test]
fn interpreter_runs_writes_conditions_and_blocks() {
    let mut memory = Bytes::default();
    memory.write(0x0300_0000, Width::Word, 0x0200_0100);
    memory.write(0x0200_0010, Width::Half, 0xfffe); // -2 signed
    let run = |text: &str, format, memory: &mut Bytes| cheat(text, format).run(memory);

    // [[03000000] + 6] = 0063.
    run("42300000 00030063", Format::ActionReplayDecrypted, &mut memory);
    assert_eq!(memory.get(0x0200_0106, Width::Half), 0x63);

    // Signed: -2 < 1 holds, so both following codes run; unsigned it fails.
    run(
        "5A200010 00000001 02200020 00000011 02200022 00000012 02200024 00000013",
        Format::ActionReplayDecrypted,
        &mut memory,
    );
    assert_eq!(memory.get(0x0200_0020, Width::Half), 0x11);
    assert_eq!(memory.get(0x0200_0022, Width::Half), 0x12);
    assert_eq!(memory.get(0x0200_0024, Width::Half), 0x13);
    run(
        "6A200010 00000001 02200030 00000011 02200032 00000012 02200034 00000013",
        Format::ActionReplayDecrypted,
        &mut memory,
    );
    assert_eq!(memory.get(0x0200_0030, Width::Half), 0);
    assert_eq!(memory.get(0x0200_0032, Width::Half), 0);
    assert_eq!(memory.get(0x0200_0034, Width::Half), 0x13);

    // A false block skips to ELSE, which resumes; ENDIF ends it.
    run(
        "8A200010 00000000 02200040 00000001 00000000 60000000 02200042 00000002 \
         00000000 40000000 02200044 00000003",
        Format::ActionReplayDecrypted,
        &mut memory,
    );
    assert_eq!(memory.get(0x0200_0040, Width::Half), 0);
    assert_eq!(memory.get(0x0200_0042, Width::Half), 2);
    assert_eq!(memory.get(0x0200_0044, Width::Half), 3);

    // A false "turn off all codes" and the end code stop the rest.
    run("CA200010 00000000 02200050 00000001", Format::ActionReplayDecrypted, &mut memory);
    run("00000000 00000000 02200052 00000001", Format::ActionReplayDecrypted, &mut memory);
    assert_eq!(memory.get(0x0200_0050, Width::Half), 0);
    assert_eq!(memory.get(0x0200_0052, Width::Half), 0);

    // CodeBreaker joypad test: keys held read as zero bits in KEYINPUT.
    memory.write(0x0400_0130, Width::Half, 0x03f7); // Start held
    run("D0000020 0008\n32000060 0001", Format::CodeBreaker, &mut memory);
    run("D0000020 0001\n32000061 0001", Format::CodeBreaker, &mut memory);
    assert_eq!(memory.get(0x0200_0060, Width::Byte), 1);
    assert_eq!(memory.get(0x0200_0061, Width::Byte), 0);

    // Slides step both address and value; E adds and wraps.
    run("42000070 0010\n00010003 0004", Format::CodeBreaker, &mut memory);
    assert_eq!(
        [0x70, 0x74, 0x78, 0x7c].map(|offset| memory.get(0x0200_0000 + offset, Width::Half)),
        [0x10, 0x11, 0x12, 0]
    );
    run("E2000070 FFFF", Format::CodeBreaker, &mut memory);
    assert_eq!(memory.get(0x0200_0070, Width::Half), 0x0f);
}

#[test]
fn state_envelope_round_trips_and_rejects_damage() {
    let cheats = vec![
        cheat("D8BAE4D9 4864DCE5 A86CDBA5 19BA49B3", Format::ActionReplay),
        cheat("82025BC4 E0FF", Format::CodeBreaker),
        cheat("02025E90:0F423F00", Format::Raw),
    ];
    let mut bytes = state_prefix(&cheats);
    let header = bytes.len();
    bytes.extend_from_slice(b"EVENFALL...");
    let (decoded, rest) = split_state(&bytes).unwrap();
    assert_eq!(decoded.unwrap(), cheats);
    assert_eq!(rest, b"EVENFALL...");
    assert_eq!(split_state(b"EVENFALL...").unwrap(), (None, &b"EVENFALL..."[..]));
    for index in [9, 13, header - 1] {
        let mut damaged = bytes.clone();
        damaged[index] ^= 1;
        assert!(split_state(&damaged).is_err(), "byte {index}");
    }
    assert!(split_state(&bytes[..header - 1]).is_err());
}
