use evenfall_replay::{InputChange, Scenario, Trace, compare};
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
            Err("usage: evenfall-parity oracle build | oracle run SCENARIO [OUTPUT] | compare A B"
                .into())
        }
    }
}
