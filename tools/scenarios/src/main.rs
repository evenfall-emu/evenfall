mod gba;
mod mgba_suite;

use evenfall_gba::machine::Machine;
use evenfall_replay::{Difference, InputChange, Scenario, Trace, compare};
use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn manifests() -> PathBuf {
    root().join("tools/scenarios/manifests")
}
fn traces() -> PathBuf {
    root().join("target/scenarios/traces")
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

fn report(
    scenario: &str,
    schedule: usize,
    differences: &[Difference],
    actual: &Trace,
    machine: &mut Machine,
) {
    for diff in differences {
        let field = if diff.kind == "state" {
            diff.detail
                .strip_prefix("byte ")
                .and_then(|s| s.split(':').next())
                .and_then(|s| s.parse::<usize>().ok())
                .and_then(|offset| actual.0.get(diff.checkpoint).map(|obs| (offset, obs)))
                .map(|(offset, obs)| {
                    format!(" ({})", gba::state_field(machine, &obs.state, offset))
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

/// Runs a scenario once per slice schedule on a fresh machine, writes each
/// schedule's trace, and checks that every schedule's trace matches the
/// first's.
fn run(path: &Path) -> Result<(), Box<dyn Error>> {
    let scenario: Scenario<InputChange> = serde_json::from_slice(&fs::read(path)?)?;
    scenario.validate()?;
    let path = fs::canonicalize(path)?;
    let name = path.file_stem().ok_or("scenario filename missing")?.to_string_lossy();
    let dir = traces().join(name.as_ref());
    // Never leave a stale trace from an earlier run next to this run's.
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    if scenario.link.is_some() {
        return run_linked(&path, &scenario, &dir);
    }
    let mut first: Option<Trace> = None;
    let mut failed = false;
    for (index, &slice) in scenario.slice_cycles.iter().enumerate() {
        let (actual, mut machine) = match gba::run(&path, &scenario, slice) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("{}: schedule {index} ({slice} cycles): {error}", scenario.name);
                failed = true;
                continue;
            }
        };
        let output = dir.join(format!("slice-{index}.trace"));
        actual.write(fs::File::create(&output)?)?;
        if let Some(ref first) = first {
            let differences = compare(first, &actual);
            if !differences.is_empty() {
                eprintln!("{}: slice independence failed at {slice} cycles", scenario.name);
                report(&scenario.name, index, &differences, &actual, &mut machine);
                failed = true;
            }
        } else {
            first = Some(actual);
        }
        println!("{}: schedule {index} ({slice} cycles): {}", scenario.name, output.display());
    }
    if failed {
        Err(format!("{}: schedules differ or failed", scenario.name).into())
    } else {
        Ok(())
    }
}

fn run_linked(
    path: &Path,
    scenario: &Scenario<InputChange>,
    dir: &Path,
) -> Result<(), Box<dyn Error>> {
    let count = scenario.link.as_ref().unwrap().peers.len() + 1;
    let mut first: Vec<Option<Trace>> = vec![None; count];
    let mut failed = false;
    for (schedule, &slice) in scenario.slice_cycles.iter().enumerate() {
        let mut results = match gba::run_linked(path, scenario, slice) {
            Ok(results) => results,
            Err(error) => {
                eprintln!(
                    "{}: linked schedule {schedule} ({slice} cycles): {error}",
                    scenario.name
                );
                failed = true;
                continue;
            }
        };
        for (index, (actual, machine)) in results.iter_mut().enumerate() {
            let output = machine_path(&dir.join(format!("slice-{schedule}.trace")), index);
            actual.write(fs::File::create(&output)?)?;
            let label = format!("{} machine {index}", scenario.name);
            if let Some(first) = &first[index] {
                let differences = compare(first, actual);
                if !differences.is_empty() {
                    eprintln!("{label}: linked slice independence failed at {slice} cycles");
                    report(&label, schedule, &differences, actual, machine);
                    failed = true;
                }
            } else {
                first[index] = Some(actual.clone());
            }
            println!("{label}: schedule {schedule} ({slice} cycles): {}", output.display());
        }
    }
    if failed {
        Err(format!("{}: linked schedules differ or failed", scenario.name).into())
    } else {
        Ok(())
    }
}

fn run_all() -> Result<(), Box<dyn Error>> {
    let mut paths = fs::read_dir(manifests())?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()?;
    paths.retain(|p| p.extension().is_some_and(|ext| ext == "json"));
    paths.sort();
    if paths.is_empty() {
        return Err("no scenarios found".into());
    }
    let mut failures = 0;
    for path in &paths {
        match std::panic::catch_unwind(|| run(path)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                eprintln!("{}: {error}", path.display());
                failures += 1;
            }
            Err(_) => {
                eprintln!("{}: panicked (see panic location above)", path.display());
                failures += 1;
            }
        }
    }
    println!("{} scenarios: {} passed, {failures} failed", paths.len(), paths.len() - failures);
    if failures > 0 { Err("scenario run failed".into()) } else { Ok(()) }
}

/// Runs every mGBA suite scenario under all its slice schedules, then
/// reports the suite's own results from the traces.
fn run_mgba_suite() -> Result<(), Box<dyn Error>> {
    let mut failures = 0;
    for boot in mgba_suite::BOOTS {
        for suite in &mgba_suite::SUITES {
            let path = mgba_suite::scenario_path(&root(), suite, boot);
            if let Err(error) = run(&path) {
                eprintln!("{}: {error}", path.display());
                failures += 1;
            }
        }
    }
    mgba_suite::report(&root(), &traces(), false)?;
    if failures > 0 {
        return Err(format!("{failures} mGBA suite scenarios failed").into());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().collect();
    match args.get(1).and_then(|v| v.to_str()) {
        Some("run") if args.len() == 3 && args[2] == "--all" => run_all(),
        Some("run") if args.len() == 3 => run(Path::new(&args[2])),
        Some("mgba-suite") => match args.get(2).and_then(|v| v.to_str()) {
            Some("scenarios") if args.len() == 3 => mgba_suite::write_scenarios(&root()),
            Some("run") if args.len() == 3 => run_mgba_suite(),
            Some("report") if args.len() == 3 => mgba_suite::report(&root(), &traces(), false),
            Some("report") if args.len() == 4 && args[3] == "--log" => {
                mgba_suite::report(&root(), &traces(), true)
            }
            _ => {
                Err("usage: evenfall-scenarios mgba-suite scenarios | run | report [--log]".into())
            }
        },
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
        _ => Err("usage: evenfall-scenarios run SCENARIO | run --all | mgba-suite scenarios|run|report [--log] | compare A B".into()),
    }
}
