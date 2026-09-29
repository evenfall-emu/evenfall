//! ROM validators. `cargo test -p evenfall-gba --test validator -- probe-timer jsmolka-ppu`
//! selects exact names or hyphen-delimited prefixes; without arguments all
//! available ROMs run. Missing ROMs are reported, never synthesized.
//! `--list`, `--gba-tests DIR`, `--probes DIR`, and `--bios FILE` mirror the
//! Zig runner's input options. Probes always use HLE boot.

mod jsmolka_cases;
mod probe_cases;

use evenfall_gba::{
    machine::{Config, Machine, StepOutcome},
    types::CYCLES_PER_FRAME,
};
use std::{env, fs, path::PathBuf};

type Result<T = ()> = std::result::Result<T, String>;
type Pixel = fn(usize, usize) -> u16;

struct Case {
    name: &'static str,
    roms: &'static [&'static str],
    run: fn(&Context) -> Result,
}

struct Context {
    roms: Vec<Vec<u8>>,
    bios: Vec<u8>,
}

impl Context {
    fn boot(&self, index: usize, config: Config<'_>) -> Box<Machine> {
        Machine::new(Config { rom: &self.roms[index], ..config })
    }

    fn advance(&self, machine: &mut Machine) -> Result {
        match machine.step() {
            StepOutcome::Executed(_) => Ok(()),
            StepOutcome::Waiting(evenfall_gba::cpu::WaitReason::Halted) => {
                machine
                    .wait_until(machine.now() + CYCLES_PER_FRAME)
                    .map_err(|e| format!("wait: {e:?}"))?;
                Ok(())
            }
            StepOutcome::Waiting(_) => Ok(()),
            StepOutcome::Faulted(fault) => Err(format!(
                "machine fault at {:08x}: {:?}",
                fault.instruction_address, fault.reason
            )),
        }
    }

    fn expect_frame(&self, machine: &Machine, expected: Pixel) -> Result {
        for y in 0..160 {
            for x in 0..240 {
                let want = expected(x, y);
                let got = machine.framebuffer()[y * 240 + x];
                if got != want {
                    return Err(format!("pixel ({x},{y}) is {got:04x}, expected {want:04x}"));
                }
            }
        }
        Ok(())
    }
}

fn selects(filter: &str, name: &str) -> bool {
    name.strip_prefix(filter).is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

fn main() {
    let mut filters = Vec::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut gba_tests = root.join("refs/gba-tests");
    let mut probes = root.join("probe/build");
    let mut bios_path = env::var_os("EVENFALL_VALIDATE_BIOS").map(PathBuf::from);
    let mut list = false;
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--gba-tests") => {
                gba_tests = args.next().expect("--gba-tests needs a directory").into()
            }
            Some("--probes") => probes = args.next().expect("--probes needs a directory").into(),
            Some("--bios") => bios_path = Some(args.next().expect("--bios needs a file").into()),
            Some("--list") => list = true,
            Some(name) if !name.starts_with('-') => filters.push(name.to_owned()),
            _ => panic!("unknown validator argument: {}", arg.to_string_lossy()),
        }
    }
    let suites =
        [("jsmolka", gba_tests, jsmolka_cases::cases()), ("probe", probes, probe_cases::cases())];
    let bios = match bios_path {
        Some(path) if !list => {
            let bytes = fs::read(&path).unwrap_or_else(|e| panic!("BIOS {}: {e}", path.display()));
            assert_eq!(bytes.len(), 16 * 1024, "BIOS must be 16 KiB");
            bytes
        }
        _ => Vec::new(),
    };
    for filter in &filters {
        assert!(
            suites.iter().any(|(_, _, cases)| cases.iter().any(|case| selects(filter, case.name))),
            "unknown validator case: {filter}"
        );
    }
    let mut passed = 0;
    let mut skipped = 0;
    let mut failed = Vec::new();
    for (suite, directory, cases) in suites {
        for case in cases {
            if !filters.is_empty() && !filters.iter().any(|f| selects(f, case.name)) {
                continue;
            }
            if list {
                println!(
                    "{} {}",
                    case.name,
                    case.roms
                        .iter()
                        .map(|rom| directory.join(rom).display().to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                continue;
            }
            let mut roms = Vec::new();
            let mut missing = None;
            for name in case.roms {
                let path = directory.join(name);
                match fs::read(&path) {
                    Ok(data) => roms.push(data),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        missing = Some(path);
                        break;
                    }
                    Err(e) => panic!("{}: {}: {e}", case.name, path.display()),
                }
            }
            if let Some(path) = missing {
                println!(
                    "skip  {}: {} absent ({} ROMs not installed/built)",
                    case.name,
                    path.display(),
                    suite
                );
                skipped += 1;
                continue;
            }
            let context =
                Context { roms, bios: if suite == "jsmolka" { bios.clone() } else { Vec::new() } };
            match (case.run)(&context) {
                Ok(()) => {
                    println!("pass  {}", case.name);
                    passed += 1;
                }
                Err(reason) => {
                    eprintln!("FAIL  {}: {reason}", case.name);
                    failed.push(case.name);
                }
            }
        }
    }
    if list {
        return;
    }
    println!("{passed} passed, {skipped} skipped, {} failed", failed.len());
    assert!(failed.is_empty(), "failed: {}", failed.join(", "));
}
