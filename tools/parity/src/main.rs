mod mgba_suite;
mod rust_backend;

use evenfall_replay::{Difference, InputChange, Scenario, Trace, compare};
use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn output() -> PathBuf {
    root().join("target/parity")
}
fn binary() -> PathBuf {
    output().join("bin/evenfall-zigba-oracle")
}

fn command(program: &str, args: &[&std::ffi::OsStr], cwd: &Path) -> Result<(), Box<dyn Error>> {
    let status = Command::new(program).args(args).current_dir(cwd).status()?;
    if !status.success() {
        return Err(format!("{program} exited with {status}").into());
    }
    Ok(())
}

fn build() -> Result<(), Box<dyn Error>> {
    let root = root();
    let scratch = output().join("zigba");
    if scratch.exists() {
        fs::remove_dir_all(&scratch)?;
    }
    fs::create_dir_all(&scratch)?;
    command(
        "cp",
        &["-a".as_ref(), root.join("refs/zigba/src").as_os_str(), scratch.as_os_str()],
        &root,
    )?;
    fs::copy(root.join("tools/parity/oracle/schema.zig"), scratch.join("src/parity_schema.zig"))?;
    let patch = fs::File::open(root.join("tools/parity/patches/schema-root.patch"))?;
    let status = Command::new("patch").args(["-p1", "-d"]).arg(&scratch).stdin(patch).status()?;
    if !status.success() {
        return Err("oracle scratch patch failed".into());
    }
    command(
        "zig",
        &[
            "build".as_ref(),
            "--build-file".as_ref(),
            root.join("tools/parity/oracle/build.zig").as_os_str(),
            "--prefix".as_ref(),
            output().as_os_str(),
            "--cache-dir".as_ref(),
            output().join("zig-cache").as_os_str(),
            "-Doptimize=ReleaseFast".as_ref(),
        ],
        &root,
    )?;
    let fixture = output().join("schema.bin");
    command(
        binary().to_str().ok_or("non-UTF8 binary path")?,
        &["schema".as_ref(), fixture.as_os_str()],
        &root,
    )?;
    let expected = fs::read(root.join("tools/parity/fixtures/schema-v6.bin"))?;
    if fs::read(fixture)? != expected {
        return Err("ZiGBA schema differs from committed v6 fixture".into());
    }
    Ok(())
}

fn trace(path: &Path) -> io::Result<Trace> {
    Trace::read(fs::File::open(path)?)
}

fn machine_path(base: &Path, index: usize) -> PathBuf {
    base.with_file_name(format!(
        "{}-machine-{index}.trace",
        base.file_stem().unwrap().to_string_lossy()
    ))
}

fn run(path: &Path, dest: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let scenario: Scenario<InputChange> = serde_json::from_slice(&fs::read(path)?)?;
    scenario.validate()?;
    if !binary().exists() {
        return Err("run `evenfall-parity oracle build` first".into());
    }
    let path = fs::canonicalize(path)?;
    let name = path.file_stem().ok_or("scenario filename missing")?.to_string_lossy();
    let dir = output().join("traces").join(name.as_ref());
    fs::create_dir_all(&dir)?;
    if let Some(link) = &scenario.link {
        let count = link.peers.len() + 1;
        let mut references: Vec<Option<Trace>> = vec![None; count];
        for (schedule, &slice) in scenario.slice_cycles.iter().enumerate() {
            let base = dir.join(format!("slice-{schedule}.trace"));
            command(
                binary().to_str().ok_or("non-UTF8 binary path")?,
                &[path.as_os_str(), schedule.to_string().as_ref(), base.as_os_str()],
                &root(),
            )?;
            for (machine, reference) in references.iter_mut().enumerate() {
                let actual = trace(&machine_path(&base, machine))?;
                if let Some(reference) = reference {
                    let differences = compare(reference, &actual);
                    if !differences.is_empty() {
                        for diff in differences {
                            eprintln!(
                                "{}: machine {machine}, checkpoint {} {}: {}",
                                scenario.name, diff.checkpoint, diff.kind, diff.detail
                            );
                        }
                        return Err(format!(
                            "oracle linked slice independence failed at {slice} cycles"
                        )
                        .into());
                    }
                } else {
                    *reference = Some(actual);
                }
            }
        }
        let base = dest.map(Path::to_path_buf).unwrap_or_else(|| dir.join("oracle.trace"));
        for machine in 0..count {
            let source = machine_path(&dir.join("slice-0.trace"), machine);
            let target = machine_path(&base, machine);
            fs::copy(source, &target)?;
            println!(
                "{}: machine {machine}, {} checkpoints, {} schedules; {}",
                scenario.name,
                scenario.checkpoints.len(),
                scenario.slice_cycles.len(),
                target.display()
            );
        }
        return Ok(());
    }
    let mut reference: Option<Trace> = None;
    for (index, &slice) in scenario.slice_cycles.iter().enumerate() {
        let output = dir.join(format!("slice-{index}.trace"));
        command(
            binary().to_str().ok_or("non-UTF8 binary path")?,
            &[path.as_os_str(), index.to_string().as_ref(), output.as_os_str()],
            &root(),
        )?;
        let actual = trace(&output)?;
        if let Some(ref reference) = reference {
            let differences = compare(reference, &actual);
            if !differences.is_empty() {
                for diff in differences {
                    eprintln!(
                        "{}: checkpoint {} {}: {}",
                        scenario.name, diff.checkpoint, diff.kind, diff.detail
                    );
                }
                return Err(format!("slice independence failed at {slice} cycles").into());
            }
        } else {
            reference = Some(actual);
        }
    }
    let output = dest.map(Path::to_path_buf).unwrap_or_else(|| dir.join("oracle.trace"));
    fs::copy(dir.join("slice-0.trace"), &output)?;
    println!(
        "{}: {} checkpoints, {} schedules; {}",
        scenario.name,
        scenario.checkpoints.len(),
        scenario.slice_cycles.len(),
        output.display()
    );
    Ok(())
}

fn report(
    scenario: &str,
    schedule: usize,
    differences: &[Difference],
    actual: &Trace,
    machine: &mut evenfall_gba::machine::Machine,
) {
    for diff in differences {
        let field = if diff.kind == "state" {
            diff.detail
                .strip_prefix("byte ")
                .and_then(|s| s.split(':').next())
                .and_then(|s| s.parse::<usize>().ok())
                .and_then(|offset| actual.0.get(diff.checkpoint).map(|obs| (offset, obs)))
                .map(|(offset, obs)| {
                    format!(" ({})", rust_backend::state_field(machine, &obs.state, offset))
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        eprintln!(
            "{scenario}: schedule {schedule}, checkpoint {} {}{field}: {}",
            diff.checkpoint, diff.kind, diff.detail
        );
    }
}

fn run_parity(path: &Path) -> Result<(), Box<dyn Error>> {
    let scenario: Scenario<InputChange> = serde_json::from_slice(&fs::read(path)?)?;
    scenario.validate()?;
    // Oracle run also verifies all oracle slicing schedules and writes the
    // reference trace. Never compare against a stale trace from a prior run.
    run(path, None)?;
    let path = fs::canonicalize(path)?;
    let name = path.file_stem().ok_or("scenario filename missing")?.to_string_lossy();
    let dir = output().join("traces").join(name.as_ref());
    if scenario.link.is_some() {
        return run_linked_parity(&path, &scenario, &dir);
    }
    let reference = trace(&dir.join("oracle.trace"))?;
    let mut first: Option<Trace> = None;
    let mut failed = false;
    for (index, &slice) in scenario.slice_cycles.iter().enumerate() {
        let (actual, mut machine) = match rust_backend::run(&path, &scenario, slice) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("{}: Rust schedule {index} ({slice} cycles): {error}", scenario.name);
                failed = true;
                continue;
            }
        };
        let output = dir.join(format!("rust-slice-{index}.trace"));
        actual.write(fs::File::create(&output)?)?;
        let differences = compare(&reference, &actual);
        report(&scenario.name, index, &differences, &actual, &mut machine);
        failed |= !differences.is_empty();
        if let Some(ref first) = first {
            let differences = compare(first, &actual);
            if !differences.is_empty() {
                eprintln!("{}: Rust slice independence failed at {slice} cycles", scenario.name);
                report(&scenario.name, index, &differences, &actual, &mut machine);
                failed = true;
            }
        } else {
            first = Some(actual);
        }
        println!("{}: Rust schedule {index} ({slice} cycles): {}", scenario.name, output.display());
    }
    if failed { Err(format!("{}: parity differs", scenario.name).into()) } else { Ok(()) }
}

fn run_linked_parity(
    path: &Path,
    scenario: &Scenario<InputChange>,
    dir: &Path,
) -> Result<(), Box<dyn Error>> {
    let count = scenario.link.as_ref().unwrap().peers.len() + 1;
    let references = (0..count)
        .map(|index| trace(&machine_path(&dir.join("oracle.trace"), index)))
        .collect::<io::Result<Vec<_>>>()?;
    let mut first: Vec<Option<Trace>> = vec![None; count];
    let mut failed = false;
    for (schedule, &slice) in scenario.slice_cycles.iter().enumerate() {
        let mut results = match rust_backend::run_linked(path, scenario, slice) {
            Ok(results) => results,
            Err(error) => {
                eprintln!(
                    "{}: Rust linked schedule {schedule} ({slice} cycles): {error}",
                    scenario.name
                );
                failed = true;
                continue;
            }
        };
        for (index, (actual, machine)) in results.iter_mut().enumerate() {
            let output = machine_path(&dir.join(format!("rust-slice-{schedule}.trace")), index);
            actual.write(fs::File::create(&output)?)?;
            let label = format!("{} machine {index}", scenario.name);
            let differences = compare(&references[index], actual);
            report(&label, schedule, &differences, actual, machine);
            failed |= !differences.is_empty();
            if let Some(first) = &first[index] {
                let differences = compare(first, actual);
                if !differences.is_empty() {
                    eprintln!("{label}: Rust linked slice independence failed at {slice} cycles");
                    report(&label, schedule, &differences, actual, machine);
                    failed = true;
                }
            } else {
                first[index] = Some(actual.clone());
            }
            println!("{label}: Rust schedule {schedule} ({slice} cycles): {}", output.display());
        }
    }
    if failed { Err(format!("{}: linked parity differs", scenario.name).into()) } else { Ok(()) }
}

fn run_all() -> Result<(), Box<dyn Error>> {
    let mut paths = fs::read_dir(root().join("tools/parity/scenarios"))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()?;
    paths.retain(|p| p.extension().is_some_and(|ext| ext == "json"));
    paths.sort();
    if paths.is_empty() {
        return Err("no scenarios found".into());
    }
    let mut failures = 0;
    for path in &paths {
        match std::panic::catch_unwind(|| run_parity(path)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                eprintln!("{}: {error}", path.display());
                failures += 1;
            }
            Err(_) => {
                eprintln!("{}: Rust backend panicked (see panic location above)", path.display());
                failures += 1;
            }
        }
    }
    println!("{} scenarios: {} passed, {failures} failed", paths.len(), paths.len() - failures);
    if failures > 0 { Err("parity run failed".into()) } else { Ok(()) }
}

/// Runs every mGBA suite scenario through the parity check, then reports the
/// suite's own results from both cores' traces.
fn run_mgba_suite() -> Result<(), Box<dyn Error>> {
    let mut failures = 0;
    for boot in mgba_suite::BOOTS {
        for suite in &mgba_suite::SUITES {
            let path = mgba_suite::scenario_path(&root(), suite, boot);
            if let Err(error) = run_parity(&path) {
                eprintln!("{}: {error}", path.display());
                failures += 1;
            }
        }
    }
    mgba_suite::report(&root(), &output().join("traces"), false)?;
    if failures > 0 {
        return Err(format!("{failures} mGBA suite scenarios failed parity").into());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().collect();
    match args.get(1).and_then(|v| v.to_str()) {
        Some("oracle") => match args.get(2).and_then(|v| v.to_str()) {
            Some("build") if args.len() == 3 => build(),
            Some("run") if (4..=5).contains(&args.len()) => {
                run(Path::new(&args[3]), args.get(4).map(Path::new))
            }
            _ => Err("usage: evenfall-parity oracle build | oracle run SCENARIO [OUTPUT]".into()),
        },
        Some("run") if args.len() == 3 && args[2] == "--all" => run_all(),
        Some("mgba-suite") => match args.get(2).and_then(|v| v.to_str()) {
            Some("scenarios") if args.len() == 3 => mgba_suite::write_scenarios(&root()),
            Some("run") if args.len() == 3 => run_mgba_suite(),
            Some("report") if args.len() == 3 => {
                mgba_suite::report(&root(), &output().join("traces"), false)
            }
            Some("report") if args.len() == 4 && args[3] == "--log" => {
                mgba_suite::report(&root(), &output().join("traces"), true)
            }
            _ => Err("usage: evenfall-parity mgba-suite scenarios | run | report [--log]".into()),
        },
        Some("run") if args.len() == 3 => run_parity(Path::new(&args[2])),
        Some("compare") if args.len() == 4 => {
            let differences = compare(&trace(Path::new(&args[2]))?, &trace(Path::new(&args[3]))?);
            if differences.is_empty() {
                println!("identical traces");
                Ok(())
            } else {
                for diff in differences {
                    println!("checkpoint {} {}: {}", diff.checkpoint, diff.kind, diff.detail);
                }
                Err("traces differ".into())
            }
        }
        _ => {
            Err("usage: evenfall-parity oracle build | oracle run SCENARIO [OUTPUT] | run SCENARIO | run --all | mgba-suite scenarios|run|report [--log] | compare A B"
                .into())
        }
    }
}
