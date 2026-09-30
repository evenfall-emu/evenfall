//! Scenarios and result extraction for the pinned mGBA suite ROM built by
//! `tools/mgba-suite/build.sh`.
//!
//! The suite is one menu-driven ROM. Each scenario boots it, selects one test
//! suite with DOWN, starts it with A, waits, then pages the suite's result
//! list with RIGHT, taking a checkpoint on every page. The extractor reads the
//! list the ROM draws: the text tilemap at `06000800h`, where a tile index is
//! a character minus `' '` and palette bank 1 marks a failing line. The ROM
//! draws only the first 29 characters of a test name.

use crate::rust_backend;
use evenfall_gba::machine::Machine;
use evenfall_replay::{InputChange, Scenario, Trace};
use serde::Serialize;
use std::{
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

pub struct Suite {
    pub slug: &'static str,
    /// The menu title, which the ROM draws on row 1 of every result page.
    title: &'static str,
    /// Tests in the suite's table (`TestSuite.nTests`).
    tests: usize,
}

/// The suite menu in order. The final menu entry, "Video tests", is
/// interactive and reports no results, so it has no scenario.
pub const SUITES: [Suite; 13] = [
    Suite { slug: "memory", title: "Memory tests", tests: 36 },
    Suite { slug: "io-read", title: "I/O read tests", tests: 130 },
    Suite { slug: "timing", title: "Timing tests", tests: 132 },
    Suite { slug: "timers", title: "Timer count-up tests", tests: 26 },
    Suite { slug: "timer-irq", title: "Timer IRQ tests", tests: 9 },
    Suite { slug: "shifter", title: "Shifter tests", tests: 70 },
    Suite { slug: "carry", title: "Carry tests", tests: 31 },
    Suite { slug: "multiply-long", title: "Multiply long tests", tests: 36 },
    Suite { slug: "bios-math", title: "BIOS math tests", tests: 123 },
    Suite { slug: "dma", title: "DMA tests", tests: 718 },
    Suite { slug: "sio-read", title: "SIO register R/W tests", tests: 90 },
    Suite { slug: "sio-timing", title: "SIO timing tests", tests: 8 },
    Suite { slug: "misc-edge", title: "Misc. edge case tests", tests: 3 },
];

const FRAME: u64 = 280_896;
/// Boot to the menu (the replacement BIOS shows its logo first), and each
/// key press: held, then released, in frames.
const BOOT_FRAMES: u64 = 30;
const BIOS_BOOT_FRAMES: u64 = 180;
const HOLD_FRAMES: u64 = 3;
/// Every suite finishes well within this many frames after A.
const RUN_FRAMES: u64 = 300;
/// Rows the list shows (`VIEW_SIZE`) and its first row.
const VIEW: usize = 16;
const FIRST_ROW: usize = 3;
const TEXT_MAP: u32 = 0x0600_0800;
const KEY_A: u16 = 1;
const KEY_RIGHT: u16 = 16;
const KEY_DOWN: u16 = 128;

/// HLE boot, or boot through the pinned Cult-of-GBA replacement BIOS.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Boot {
    Hle,
    ExternalBios,
}

pub const BOOTS: [Boot; 2] = [Boot::Hle, Boot::ExternalBios];

fn scenario_name(suite: &Suite, boot: Boot) -> String {
    match boot {
        Boot::Hle => format!("mgba-suite-{}", suite.slug),
        Boot::ExternalBios => format!("mgba-suite-{}-external_bios", suite.slug),
    }
}

pub fn scenario_path(root: &Path, suite: &Suite, boot: Boot) -> PathBuf {
    root.join(format!("tools/parity/scenarios/{}.json", scenario_name(suite, boot)))
}

/// The list's RIGHT key: the page's last row, then the next page, then the
/// last test, then a wrap to the first (`runSuite` in the suite's `main.c`).
fn right(cursor: usize, top: usize, tests: usize) -> (usize, usize) {
    let cursor = if cursor == tests - 1 {
        0
    } else if cursor + VIEW >= tests {
        tests - 1
    } else if top + VIEW - 1 == cursor {
        cursor + VIEW
    } else {
        top + VIEW - 1
    };
    let top = if cursor < top {
        cursor
    } else if cursor >= top + VIEW {
        cursor + 1 - VIEW
    } else {
        top
    };
    (cursor, top)
}

/// RIGHT presses that page through the list and wrap to its first test.
fn pages(tests: usize) -> usize {
    let (mut cursor, mut top, mut presses) = (0, 0, 0);
    loop {
        (cursor, top) = right(cursor, top, tests);
        presses += 1;
        if cursor == 0 {
            return presses;
        }
    }
}

/// The manifest layout of `evenfall_replay::Scenario`, without the options
/// these scenarios leave unset.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    name: String,
    assets: ManifestAssets,
    config: ManifestConfig,
    inputs: Vec<ManifestInput>,
    end_cycle: String,
    checkpoints: Vec<String>,
    slice_cycles: Vec<String>,
}

#[derive(Serialize)]
struct ManifestAssets {
    rom: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bios: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestConfig {
    boot: &'static str,
    save_type: &'static str,
}

#[derive(Serialize)]
struct ManifestInput {
    at: String,
    change: ManifestKeys,
}

#[derive(Serialize)]
struct ManifestKeys {
    keys: u16,
}

fn scenario(index: usize, suite: &Suite, boot: Boot) -> Manifest {
    let mut inputs = Vec::new();
    let mut at = FRAME
        * match boot {
            Boot::Hle => BOOT_FRAMES,
            Boot::ExternalBios => BIOS_BOOT_FRAMES,
        };
    let mut press = |at: &mut u64, keys: u16| {
        inputs.push(ManifestInput { at: at.to_string(), change: ManifestKeys { keys } });
        let release = *at + HOLD_FRAMES * FRAME;
        inputs.push(ManifestInput { at: release.to_string(), change: ManifestKeys { keys: 0 } });
        *at += 2 * HOLD_FRAMES * FRAME;
    };
    for _ in 0..index {
        press(&mut at, KEY_DOWN);
    }
    press(&mut at, KEY_A);
    at += RUN_FRAMES * FRAME;
    let mut checkpoints = vec![at.to_string()];
    for _ in 0..pages(suite.tests) {
        at += FRAME;
        press(&mut at, KEY_RIGHT);
        checkpoints.push(at.to_string());
    }
    Manifest {
        name: scenario_name(suite, boot),
        assets: ManifestAssets {
            rom: "../../../tools/mgba-suite/build/suite.gba",
            bios: (boot == Boot::ExternalBios).then_some("../../../refs/cult-gba-bios/bios.bin"),
        },
        config: ManifestConfig {
            boot: match boot {
                Boot::Hle => "hle",
                Boot::ExternalBios => "external_bios",
            },
            save_type: "auto",
        },
        inputs,
        end_cycle: at.to_string(),
        checkpoints,
        slice_cycles: ["70224", "280896", "1232"].map(String::from).to_vec(),
    }
}

pub fn write_scenarios(root: &Path) -> Result<(), Box<dyn Error>> {
    for boot in BOOTS {
        for (index, suite) in SUITES.iter().enumerate() {
            let json = serde_json::to_string_pretty(&scenario(index, suite, boot))?;
            fs::write(scenario_path(root, suite, boot), json + "\n")?;
        }
    }
    Ok(())
}

/// One suite's results as its list shows them.
#[derive(PartialEq, Eq)]
struct Results {
    /// Passing and total checks, from the list header.
    checks: (u32, u32),
    /// Each test's (truncated) name and whether all its checks passed.
    tests: Vec<(String, bool)>,
}

struct Page {
    title: String,
    checks: (u32, u32),
    cursor_row: Option<usize>,
    rows: Vec<(String, bool)>,
}

fn read_page(machine: &mut Machine, state: &[u8], tests: usize) -> Result<Page, Box<dyn Error>> {
    machine.load_state(state).map_err(|e| format!("checkpoint state: {e:?}"))?;
    let mut bytes = [0; 20 * 32 * 2];
    machine.inspect_memory(TEXT_MAP, &mut bytes).map_err(|_| "text map address")?;
    let row = |row: usize| -> (String, bool) {
        let entries = (0..30).map(|col| {
            let at = (row * 32 + col) * 2;
            u16::from_le_bytes([bytes[at], bytes[at + 1]])
        });
        let mut text = String::new();
        let mut failed = false;
        for entry in entries {
            text.push(char::from(b' '.wrapping_add((entry & 0x3ff) as u8)));
            failed |= entry >> 12 == 1;
        }
        (text, failed)
    };
    let (header, _) = row(1);
    let (title, summary) = header.split_at(21);
    let checks = summary
        .trim()
        .split_once('/')
        .and_then(|(p, t)| Some((p.trim().parse().ok()?, t.trim().parse().ok()?)))
        .ok_or_else(|| format!("no result summary in {header:?}"))?;
    let mut cursor_row = None;
    let mut rows = Vec::new();
    for index in 0..tests.min(VIEW) {
        let (text, failed) = row(FIRST_ROW + index);
        if text.starts_with('>') {
            cursor_row = Some(index);
        }
        rows.push((text[1..].trim_end().to_string(), !failed));
    }
    Ok(Page { title: title.trim_end().to_string(), checks, cursor_row, rows })
}

fn extract(
    machine: &mut Machine,
    trace: &Trace,
    suite: &Suite,
) -> Result<(Results, Vec<u8>), Box<dyn Error>> {
    let expected_pages = pages(suite.tests) + 1;
    if trace.0.len() != expected_pages {
        return Err(format!("{} checkpoints, expected {expected_pages}", trace.0.len()).into());
    }
    let mut tests: Vec<Option<(String, bool)>> = vec![None; suite.tests];
    let mut checks = None;
    let (mut cursor, mut top) = (0, 0);
    for (index, observation) in trace.0.iter().enumerate() {
        if index > 0 {
            (cursor, top) = right(cursor, top, suite.tests);
        }
        let page = read_page(machine, &observation.state, suite.tests)?;
        if !suite.title.starts_with(&page.title) || page.title.is_empty() {
            return Err(format!("checkpoint {index}: list title {:?}", page.title).into());
        }
        if page.cursor_row != Some(cursor - top) {
            return Err(format!(
                "checkpoint {index}: cursor row {:?}, expected {} (suite still running?)",
                page.cursor_row,
                cursor - top
            )
            .into());
        }
        if checks.is_some_and(|c| c != page.checks) {
            return Err(format!("checkpoint {index}: summary changed").into());
        }
        checks = Some(page.checks);
        for (row, test) in page.rows.into_iter().enumerate() {
            let slot = &mut tests[top + row];
            if slot.as_ref().is_some_and(|seen| *seen != test) {
                return Err(format!("checkpoint {index}: test {} changed", top + row).into());
            }
            *slot = Some(test);
        }
    }
    let tests = tests
        .into_iter()
        .enumerate()
        .map(|(index, test)| test.ok_or_else(|| format!("test {index} never shown")))
        .collect::<Result<_, _>>()?;
    let log = trace.0.last().map(|o| o.save.clone()).unwrap_or_default();
    Ok((Results { checks: checks.ok_or("no checkpoints")?, tests }, log))
}

/// The SRAM log the suite writes with `savprintf`, up to its terminator.
fn log_text(save: &[u8]) -> String {
    let end = save.iter().position(|&b| b == 0).unwrap_or(save.len());
    String::from_utf8_lossy(&save[..end]).into_owned()
}

fn summary(results: &Results) -> String {
    let passed = results.tests.iter().filter(|(_, pass)| *pass).count();
    format!(
        "{}/{} checks, {passed}/{} tests",
        results.checks.0,
        results.checks.1,
        results.tests.len()
    )
}

/// Prints each suite's results from the first Rust schedule's trace written
/// by `evenfall-parity run`, and whether the ZiGBA oracle's trace shows the
/// same results. The list is read from Evenfall's save states. ZiGBA's states
/// use its own format, so its results are the same exactly when every
/// checkpoint's frame, which draws the list, and SRAM log match Evenfall's.
/// Returns an error when a trace is missing or unreadable or when the two
/// cores' checkpoints differ; failing suite tests are results, not errors.
pub fn report(root: &Path, traces: &Path, with_log: bool) -> Result<(), Box<dyn Error>> {
    let mut problems = 0;
    let mut out = String::new();
    for (boot, suite) in BOOTS.into_iter().flat_map(|boot| SUITES.iter().map(move |s| (boot, s))) {
        let scenario_file = scenario_path(root, suite, boot);
        let scenario: Scenario<InputChange> = serde_json::from_slice(&fs::read(&scenario_file)?)?;
        let mut machine =
            rust_backend::make_machine(&scenario_file, &scenario.assets, &scenario.config)?;
        let dir = traces.join(&scenario.name);
        let read = |file: &str| -> Result<Trace, Box<dyn Error>> {
            Trace::read(fs::File::open(dir.join(file))?).map_err(|e| format!("{file}: {e}").into())
        };
        let traces = read("oracle.trace").and_then(|zigba| {
            let evenfall = read("rust-slice-0.trace")?;
            let results = extract(&mut machine, &evenfall, suite)
                .map_err(|e| format!("rust-slice-0.trace: {e}"))?;
            Ok((zigba, evenfall, results))
        });
        let (zigba, evenfall, (results, log)) = match traces {
            Ok(traces) => traces,
            Err(error) => {
                eprintln!("{}: {error}", scenario.name);
                problems += 1;
                continue;
            }
        };
        let same = zigba.0.len() == evenfall.0.len()
            && zigba.0.iter().zip(&evenfall.0).all(|(z, e)| z.frame == e.frame && z.save == e.save);
        problems += usize::from(!same);
        writeln!(
            out,
            "{} ({}): {}; ZiGBA {}",
            suite.title,
            if boot == Boot::Hle { "HLE" } else { "Cult-of-GBA BIOS" },
            summary(&results),
            if same { "same" } else { "DIFFERENT" }
        )?;
        for (index, (name, pass)) in results.tests.iter().enumerate() {
            if !pass {
                writeln!(out, "  {index:4} FAIL: {name}")?;
            }
        }
        if with_log {
            writeln!(out, "  --- SRAM log")?;
            for line in log_text(&log).lines() {
                writeln!(out, "  | {line}")?;
            }
        }
    }
    print!("{out}");
    if problems > 0 {
        return Err(format!("{problems} suites lack traces or differ between cores").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_pages_then_clamps_to_the_last_test_then_wraps() {
        let mut state = (0, 0);
        let mut seen = Vec::new();
        for _ in 0..pages(40) {
            state = right(state.0, state.1, 40);
            seen.push(state);
        }
        assert_eq!(seen, [(15, 0), (31, 16), (39, 24), (0, 0)]);
        assert_eq!(pages(3), 2);
        assert_eq!(pages(1), 1);
    }
}
