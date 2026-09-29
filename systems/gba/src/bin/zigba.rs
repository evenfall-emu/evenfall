//! ZiGBA's headless CLI, with file policy separated from the machine.

use evenfall_gba::bus::{KeyState, TimedInput, TimedInputChange};
use evenfall_gba::cpu::InstructionSet;
use evenfall_gba::host::host_save;
use evenfall_gba::machine::{BootMode, Config, Machine, StepOutcome};
use std::io::{self, Write};

const USAGE: &str = "usage: zigba run --steps <count> [--bios <path>] [--save-type <type>] [--save-file <path>] [--state-in <path>] [--state-out <path>] [--trace <path>] [--trace-from <steps>] [--input <movie>] [--quiet] [--frame-hash] [--no-rtc | --rtc-epoch <seconds>] [--no-solar | --solar-level <0..255>] [--no-gyro | --gyro-sample <0..4095>] [--rumble | --no-rumble] [--no-tilt | --tilt <x> <y>] <rom.gba|rom.zip>\n";

fn main() {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if let Err(error) = run(&std::env::args().collect::<Vec<_>>(), &mut stdout) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn keys_from_bits(bits: u16) -> KeyState {
    KeyState {
        a: bits & 1 != 0,
        b: bits & 2 != 0,
        select: bits & 4 != 0,
        start: bits & 8 != 0,
        right: bits & 16 != 0,
        left: bits & 32 != 0,
        up: bits & 64 != 0,
        down: bits & 128 != 0,
        r: bits & 256 != 0,
        l: bits & 512 != 0,
    }
}

fn run(args: &[String], stdout: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() < 5 || args[1] != "run" || args[2] != "--steps" {
        stdout.write_all(USAGE.as_bytes())?;
        return Ok(());
    }
    let Ok(step_count) = args[3].parse::<usize>() else {
        stdout.write_all(USAGE.as_bytes())?;
        return Ok(());
    };
    let tail: Vec<&str> = args[4..].iter().map(String::as_str).collect();
    let options = match host_save::parse_options(&tail) {
        Ok(options) => options,
        Err(_) => {
            stdout.write_all(USAGE.as_bytes())?;
            return Ok(());
        }
    };
    let rom = match host_save::load_rom_file(options.rom_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            writeln!(stdout, "ROM load failed: {error:?}")?;
            return Ok(());
        }
    };
    let prepared = match host_save::prepare(&rom, &options) {
        Ok(value) => value,
        Err(error) => {
            writeln!(stdout, "save setup failed: {error:?}")?;
            return Ok(());
        }
    };
    let bios = match host_save::load_bios_file(&options) {
        Ok(bytes) => bytes,
        Err(error) => {
            writeln!(stdout, "BIOS setup failed: {error:?}")?;
            return Ok(());
        }
    };
    let mut machine = Machine::new(Config {
        rom: &rom,
        bios: &bios,
        immutable_media: true,
        boot: if bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
        save_type: prepared.save_type,
        save_data: prepared.save_data.as_deref(),
        rtc_epoch_seconds: host_save::resolve_rtc_epoch(
            &rom,
            options.rtc,
            host_save::current_unix_epoch(),
        ),
        solar_level: host_save::resolve_solar_level(&rom, options.solar),
        gyro_sample: host_save::resolve_gyro_sample(&rom, options.gyro),
        rumble: host_save::resolve_rumble(&rom, options.rumble_override),
        tilt_sample: host_save::resolve_tilt(&rom, options.tilt),
    });
    if let Some(path) = options.state_in {
        let bytes = match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 8 * 1024 * 1024 => bytes,
            Ok(_) => {
                writeln!(stdout, "state load failed: StreamTooLong")?;
                return Ok(());
            }
            Err(error) => {
                writeln!(stdout, "state load failed: {error}")?;
                return Ok(());
            }
        };
        if let Err(error) = machine.load_state(&bytes) {
            writeln!(stdout, "state load failed: {error:?}")?;
            return Ok(());
        }
    }
    let mut trace = match options.trace_path {
        Some(path) => Some(io::BufWriter::new(std::fs::File::create(path)?)),
        None => None,
    };
    let movie = match options.input_movie {
        Some(text) => host_save::parse_input_movie(text).map_err(|error| format!("{error:?}"))?,
        None => host_save::InputMovie { events: Vec::new() },
    };
    let inputs: Vec<TimedInput> = movie
        .events
        .iter()
        .map(|event| TimedInput {
            at: event.at,
            change: TimedInputChange::Keys(keys_from_bits(event.keys)),
        })
        .collect();
    let mut input_index = 0;
    let mut initial_keys = KeyState::default();
    while input_index < inputs.len() && inputs[input_index].at <= machine.now() {
        let TimedInputChange::Keys(keys) = inputs[input_index].change else { unreachable!() };
        initial_keys = keys;
        input_index += 1;
    }
    if options.input_movie.is_some() {
        machine.set_keys(initial_keys);
    }

    let mut executed_count = 0;
    let mut terminal_fault = None;
    'execution: while executed_count < step_count {
        let outcome = if input_index < inputs.len() {
            let result = machine
                .step_with_input(&inputs[input_index..input_index + 1])
                .map_err(|error| format!("{error:?}"))?;
            input_index += result.inputs_consumed;
            result.outcome
        } else {
            machine.step()
        };
        let step = match outcome {
            StepOutcome::Executed(step) => step,
            StepOutcome::Waiting(reason) => {
                let mut woke = false;
                loop {
                    let waiting_at = machine.now();
                    let frame_deadline = waiting_at.saturating_add(Machine::CYCLES_PER_FRAME);
                    let deadline = inputs
                        .get(input_index)
                        .map_or(frame_deadline, |event| frame_deadline.min(event.at));
                    let waiting_inputs =
                        inputs.get(input_index).map_or(&[][..], std::slice::from_ref);
                    let progress = machine
                        .wait_until_with_input(deadline, waiting_inputs)
                        .map_err(|error| format!("{error:?}"))?;
                    input_index += progress.inputs_consumed;
                    if progress.woke {
                        woke = true;
                        break;
                    }
                    if input_index < inputs.len()
                        && (machine.now() != waiting_at || progress.inputs_consumed != 0)
                    {
                        continue;
                    }
                    break;
                }
                if woke {
                    continue 'execution;
                }
                if !options.quiet {
                    writeln!(
                        stdout,
                        "wait cycle={} power={}",
                        machine.now(),
                        match reason {
                            evenfall_gba::cpu::WaitReason::Halted => "halted",
                            evenfall_gba::cpu::WaitReason::Stopped => "stopped",
                        }
                    )?;
                }
                break 'execution;
            }
            StepOutcome::Faulted(fault) => {
                terminal_fault = Some(fault);
                break 'execution;
            }
        };
        if let Some(trace) = trace.as_mut().filter(|_| executed_count >= options.trace_from) {
            let mut record = [0; 16];
            record[0..4].copy_from_slice(&step.instruction_address.to_le_bytes());
            record[4..8].copy_from_slice(&step.instruction.to_le_bytes());
            record[8..12].copy_from_slice(&machine.read_cpsr().to_le_bytes());
            record[12] = match step.instruction_set {
                InstructionSet::Arm => 0,
                InstructionSet::Thumb => 1,
            };
            trace.write_all(&record)?;
        }
        if !options.quiet {
            writeln!(
                stdout,
                "{}..{} pc={:08x} {}={:0width$x}",
                step.started_at,
                step.stopped_at,
                step.instruction_address,
                match step.instruction_set {
                    InstructionSet::Arm => "arm",
                    InstructionSet::Thumb => "thumb",
                },
                step.instruction,
                width = if step.instruction_set == InstructionSet::Arm { 8 } else { 4 }
            )?;
        }
        executed_count += 1;
    }
    if let Some(fault) = terminal_fault {
        fault.write_diagnostic(stdout)?;
    }
    if let Some(mut trace) = trace {
        trace.flush()?;
    }
    host_save::write_atomic(&prepared.save_path, machine.save_data())
        .map_err(|error| format!("{error:?}"))?;
    if let Some(path) = options.state_out {
        if terminal_fault.is_some() {
            stdout.write_all(b"state save skipped: machine faulted\n")?;
        } else {
            let bytes = machine.write_state_alloc().map_err(|error| format!("{error:?}"))?;
            host_save::write_atomic(path, &bytes).map_err(|error| format!("{error:?}"))?;
        }
    }
    if options.frame_hash {
        let pixels: Vec<u8> = machine.framebuffer().iter().flat_map(|p| p.to_le_bytes()).collect();
        writeln!(
            stdout,
            "frame count={} wyhash={:016x}",
            machine.completed_frames(),
            wyhash::wyhash(&pixels, 0)
        )?;
    }
    writeln!(
        stdout,
        "stop cycle={} next={:08x} cpsr={:08x} r0={:08x}",
        machine.now(),
        machine.next_instruction_address(),
        machine.read_cpsr(),
        machine.read_register(0)
    )?;
    Ok(())
}
