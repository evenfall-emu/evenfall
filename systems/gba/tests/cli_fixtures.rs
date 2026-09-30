//! Byte-exact headless CLI fixtures.

use std::process::Command;

#[test]
fn headless_cli_output_matches_fixtures() {
    let directory = std::env::temp_dir().join(format!(
        "evenfall-cli-fixtures-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();

    let cases: &[(&str, &[u8], &[&str], &str)] = &[
        (
            "mov",
            &[0x2a, 0x00, 0xa0, 0xe3],
            &["run", "--steps", "1"],
            "0..8 pc=08000000 arm=e3a0002a\nstop cycle=8 next=08000004 cpsr=0000001f r0=0000002a\n",
        ),
        (
            "thumb",
            &[0x05, 0x00, 0x8f, 0xe2, 0x10, 0xff, 0x2f, 0xe1, 0x00, 0x00, 0x00, 0x00, 0x2a, 0x21],
            &["run", "--steps", "3"],
            "0..8 pc=08000000 arm=e28f0005\n8..22 pc=08000004 arm=e12fff10\n22..25 pc=0800000c thumb=212a\nstop cycle=25 next=0800000e cpsr=0000003f r0=0800000d\n",
        ),
        (
            "halt",
            &[0x00, 0x00, 0x02, 0xef],
            &["run", "--steps", "2"],
            "0..53 pc=08000000 arm=ef020000\nwait cycle=280949 power=halted\nstop cycle=280949 next=08000004 cpsr=0000001f r0=00000000\n",
        ),
        (
            "halt-wake",
            &[
                0x01, 0x03, 0xa0, 0xe3, 0x08, 0x10, 0xa0, 0xe3, 0xb4, 0x10, 0xc0, 0xe1, 0x01, 0x10,
                0xa0, 0xe3, 0x02, 0x0c, 0x80, 0xe2, 0xb0, 0x10, 0xc0, 0xe1, 0x00, 0x00, 0x02, 0xef,
                0x2a, 0x00, 0xa0, 0xe3,
            ],
            &["run", "--steps", "8", "--quiet"],
            "stop cycle=197126 next=08000020 cpsr=0000001f r0=0000002a\n",
        ),
        (
            "stop",
            &[0x00, 0x00, 0x03, 0xef],
            &["run", "--steps", "2"],
            "0..53 pc=08000000 arm=ef030000\nwait cycle=53 power=stopped\nstop cycle=53 next=08000004 cpsr=0000001f r0=00000000\n",
        ),
    ];

    for &(name, rom, args, expected) in cases {
        let path = directory.join(format!("{name}.gba"));
        std::fs::write(&path, rom).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_evenfall-gba"))
            .args(args)
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{name}");
        assert!(output.stderr.is_empty(), "{name}: {output:?}");
    }

    let usage = Command::new(env!("CARGO_BIN_EXE_evenfall-gba")).output().unwrap();
    assert!(usage.status.success());
    assert!(usage.stdout.starts_with(b"usage: evenfall-gba run --steps <count>"));
    assert!(usage.stderr.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}
