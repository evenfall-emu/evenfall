//! ZiGBA's headless CLI, with file policy separated from the machine.

use evenfall_gba::bus::{KeyState, TimedInput, TimedInputChange};
use evenfall_gba::cpu::InstructionSet;
use evenfall_gba::host::host_save;
use evenfall_gba::machine::{BootMode, Config, Machine, StepOutcome};
use std::io::{self, Write};

const USAGE: &str = "usage: evenfall-gba run --steps <count> [--bios <path>] [--save-type <type>] [--save-file <path>] [--state-in <path>] [--state-out <path>] [--trace <path>] [--trace-from <steps>] [--input <movie>] [--quiet] [--frame-hash] [--no-rtc | --rtc-epoch <seconds>] [--no-solar | --solar-level <0..255>] [--no-gyro | --gyro-sample <0..4095>] [--rumble | --no-rumble] [--no-tilt | --tilt <x> <y>] <rom.gba|rom.zip>\n";

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

/// Zig 0.16's Wyhash final v4.1, seed zero. The `wyhash` crate's top-level
/// function implements a different generation and changes `--frame-hash`.
fn frame_wyhash(input: &[u8]) -> u64 {
    const SECRET: [u64; 4] = [
        0xa076_1d64_78bd_642f,
        0xe703_7ed1_a0b4_28db,
        0x8ebc_6af0_9c88_c6e3,
        0x5899_65cc_7537_4cc3,
    ];
    fn read4(bytes: &[u8]) -> u64 {
        u64::from(u32::from_le_bytes(bytes[..4].try_into().unwrap()))
    }
    fn read8(bytes: &[u8]) -> u64 {
        u64::from_le_bytes(bytes[..8].try_into().unwrap())
    }
    fn mum(a: u64, b: u64) -> (u64, u64) {
        let product = u128::from(a) * u128::from(b);
        (product as u64, (product >> 64) as u64)
    }
    fn mix(a: u64, b: u64) -> u64 {
        let (low, high) = mum(a, b);
        low ^ high
    }

    let len = input.len();
    let mut state = [mix(SECRET[0], SECRET[1]); 3];
    let (mut a, mut b);
    if len <= 16 {
        if len >= 4 {
            let end = len - 4;
            let quarter = (len >> 3) << 2;
            a = (read4(input) << 32) | read4(&input[quarter..]);
            b = (read4(&input[end..]) << 32) | read4(&input[end - quarter..]);
        } else if len > 0 {
            a = (u64::from(input[0]) << 16)
                | (u64::from(input[len >> 1]) << 8)
                | u64::from(input[len - 1]);
            b = 0;
        } else {
            a = 0;
            b = 0;
        }
    } else {
        let mut offset = 0;
        if len >= 48 {
            while offset + 48 < len {
                for lane in 0..3 {
                    let pos = offset + lane * 16;
                    state[lane] = mix(
                        read8(&input[pos..]) ^ SECRET[lane + 1],
                        read8(&input[pos + 8..]) ^ state[lane],
                    );
                }
                offset += 48;
            }
            state[0] ^= state[1] ^ state[2];
        }
        let mut pos = offset;
        while pos + 16 < len {
            state[0] = mix(read8(&input[pos..]) ^ SECRET[1], read8(&input[pos + 8..]) ^ state[0]);
            pos += 16;
        }
        a = read8(&input[len - 16..]);
        b = read8(&input[len - 8..]);
    }
    a ^= SECRET[1];
    b ^= state[0];
    let (a, b) = mum(a, b);
    mix(a ^ SECRET[0] ^ len as u64, b ^ SECRET[1])
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
                            evenfall_gba::cpu::WaitReason::DmaStall => "dma_stall",
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
            frame_wyhash(&pixels)
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

#[cfg(test)]
mod tests {
    use super::frame_wyhash;

    /// Independent results from Zig 0.16 std.hash.Wyhash.hash(0, bytes),
    /// including either side of both the 16- and 48-byte block boundaries.
    #[test]
    fn frame_hash_matches_zig_wyhash_vectors() {
        assert_eq!(frame_wyhash(&[0, 1, 3, 8, 21, 55]), 0x85a2_09c1_31ba_b780);
        let pixels: Vec<u8> = (0..240 * 160 * 2).map(|i| (i * 37 + 11) as u8).collect();
        for (length, expected) in [
            (0, 0x0409_638e_e2bd_e459),
            (16, 0x411f_fa3c_1331_fbaf),
            (17, 0x08a9_b771_ef15_13d6),
            (48, 0x8284_ed8c_b813_67a5),
            (49, 0xe6aa_ced3_f92b_5365),
            (240 * 160 * 2, 0x3c9a_ac8c_32c4_b966),
        ] {
            assert_eq!(frame_wyhash(&pixels[..length]), expected, "length {length}");
        }
    }
}
