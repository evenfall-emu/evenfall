//! Byte-exact headless CLI fixtures from ZiGBA's `build.zig` test-host step.

use std::process::Command;

#[test]
fn zigba_build_cli_fixtures() {
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

/// Evenfall's cheat flags, beyond ZiGBA's CLI: a loop loads the EWRAM word
/// at 02000000 into r0, and the `--cheat` or `--cheats` value must reach it
/// once the first VBlank has passed.
#[test]
fn cheat_flags_patch_memory_the_guest_reads() {
    let directory =
        std::env::temp_dir().join(format!("evenfall-cli-cheats-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    // mov r1, #0x02000000; loop: ldr r0, [r1]; b loop
    let rom: Vec<u8> =
        [0xe3a0_1402u32, 0xe591_0000, 0xeaff_fffd].iter().flat_map(|w| w.to_le_bytes()).collect();
    let rom_path = directory.join("loop.gba");
    std::fs::write(&rom_path, rom).unwrap();
    let cheat_file = directory.join("codes.txt");
    std::fs::write(&cheat_file, "# raw\n!raw\n02000000:0000BEEF\n").unwrap();
    let run = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_evenfall-gba"))
            .args(["run", "--steps", "40000", "--quiet", "--save-type", "none"])
            .args(extra)
            .arg(&rom_path)
            .output()
            .unwrap();
        (output.status.success(), String::from_utf8(output.stdout).unwrap(), output.stderr)
    };
    let r0 = |stdout: &str| stdout.trim_end().rsplit("r0=").next().unwrap().to_owned();
    let (ok, stdout, _) = run(&[]);
    assert!(ok);
    assert_eq!(r0(&stdout), "00000000");
    let (ok, stdout, _) = run(&["--cheat", "raw:02000000:0000002A"]);
    assert!(ok, "{stdout}");
    assert_eq!(r0(&stdout), "0000002a");
    let (ok, stdout, _) = run(&["--cheats", cheat_file.to_str().unwrap()]);
    assert!(ok, "{stdout}");
    assert_eq!(r0(&stdout), "0000beef");
    let (ok, _, stderr) = run(&["--cheat", "action-replay:0202XYZ0 00000001"]);
    assert!(!ok);
    assert!(String::from_utf8_lossy(&stderr).contains("not a code line"));
    std::fs::remove_dir_all(&directory).unwrap();
}
