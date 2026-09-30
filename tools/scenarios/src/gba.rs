//! Scenario media and system-specific controls for the GBA machine.

use evenfall_gba::{
    bus::{JoyBusCommand, KeyState, SioConnection, TimedInputChange},
    cartridge::{save, tilt},
    machine::{self, Machine},
};
use evenfall_replay::{
    Assets, BootMode, Config, Connection, HostCommand, Input, InputChange, Observation, SaveType,
    Scenario, Trace, run_with_host_actions,
};
use std::{error::Error, fs, path::Path};

pub type LinkedTrace = (Trace, Box<Machine>);

fn asset(parent: &Path, path: &str, max: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let bytes = fs::read(parent.join(path))?;
    if bytes.len() > max {
        return Err(format!("{path}: asset larger than {max} bytes").into());
    }
    Ok(bytes)
}

fn keys(bits: u16) -> Result<KeyState, String> {
    if bits > 0x3ff {
        return Err(format!("invalid KEYINPUT pressed mask {bits:#x}"));
    }
    Ok(KeyState {
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
    })
}

fn tilt(sample: [u16; 2]) -> Result<tilt::Sample, String> {
    if sample.iter().any(|&axis| axis > 4095) {
        return Err(format!("invalid tilt sample {sample:?}"));
    }
    Ok(tilt::Sample { x: sample[0], y: sample[1] })
}

fn change(input: InputChange) -> Result<TimedInputChange, String> {
    Ok(match input {
        InputChange::Keys(mask) => TimedInputChange::Keys(keys(mask)?),
        InputChange::SolarLevel(v) => TimedInputChange::SolarLevel(v),
        InputChange::GyroSample(v) if v <= 4095 => TimedInputChange::GyroSample(v),
        InputChange::GyroSample(v) => return Err(format!("invalid gyro sample {v}")),
        InputChange::Tilt(v) => TimedInputChange::Tilt(tilt(v)?),
    })
}

fn inputs(scenario: &Scenario<InputChange>) -> Result<Scenario<TimedInputChange>, String> {
    let inputs = scenario
        .inputs
        .iter()
        .map(|input| Ok(Input { at: input.at, change: change(input.change)? }))
        .collect::<Result<_, String>>()?;
    Ok(Scenario {
        name: scenario.name.clone(),
        assets: scenario.assets.clone(),
        config: scenario.config.clone(),
        link: scenario.link.clone(),
        inputs,
        host_actions: scenario.host_actions.clone(),
        end_cycle: scenario.end_cycle,
        checkpoints: scenario.checkpoints.clone(),
        slice_cycles: scenario.slice_cycles.clone(),
        terminal_pc: scenario.terminal_pc,
        terminal_r12: scenario.terminal_r12,
    })
}

pub fn run(
    path: &Path,
    scenario: &Scenario<InputChange>,
    slice: u64,
) -> Result<(Trace, Box<Machine>), Box<dyn Error>> {
    let mapped = inputs(scenario)?;
    let mut machine = make_machine(path, &scenario.assets, &scenario.config)?;
    let trace = run_with_host_actions(
        &mut *machine,
        &mapped,
        slice,
        |m| {
            let cpu = m.inspect_cpu();
            evenfall_replay::Cpu {
                registers: cpu.registers,
                pc: cpu.next_instruction_address,
                cpsr: cpu.cpsr,
            }
        },
        |m, action| {
            let command = match action {
                HostCommand::JoybusWrite(value) => JoyBusCommand::WriteToGba(*value),
                HostCommand::JoybusRead(true) => JoyBusCommand::ReadFromGba,
                HostCommand::JoybusRead(false) => return Err("invalid JoyBus read action".into()),
            };
            m.bus.schedule_joy_bus(command, m.now() + 1000).map_err(|e| format!("JoyBus: {e:?}"))
        },
    )?;
    Ok((trace, machine))
}

pub(crate) fn make_machine(
    path: &Path,
    assets: &Assets,
    config: &Config,
) -> Result<Box<Machine>, Box<dyn Error>> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let rom = asset(parent, &assets.rom, 32 * 1024 * 1024)?;
    let bios = assets
        .bios
        .as_deref()
        .map(|name| asset(parent, name, 16 * 1024))
        .transpose()?
        .unwrap_or_default();
    let save = assets.save.as_deref().map(|name| asset(parent, name, 128 * 1024)).transpose()?;
    let boot = match config.boot {
        BootMode::Hle => machine::BootMode::Hle,
        BootMode::SkipBios => machine::BootMode::SkipBios,
        BootMode::ExternalBios => machine::BootMode::ExternalBios,
    };
    if (boot == machine::BootMode::Hle && !bios.is_empty())
        || (boot != machine::BootMode::Hle && bios.len() != 16 * 1024)
    {
        return Err("HLE requires no BIOS; skip/external boot requires exactly 16 KiB".into());
    }
    let save_type = match config.save_type {
        SaveType::Auto => save::Type::Auto,
        SaveType::None => save::Type::None,
        SaveType::Sram => save::Type::Sram,
        SaveType::Flash64k => save::Type::Flash64k,
        SaveType::Flash64kAtmel => save::Type::Flash64kAtmel,
        SaveType::Flash128k => save::Type::Flash128k,
        SaveType::Eeprom512 => save::Type::Eeprom512,
        SaveType::Eeprom8k => save::Type::Eeprom8k,
    };
    if let Some(data) = &save {
        let required = match save_type {
            save::Type::Auto => return Err("explicit save requires a save type".into()),
            save::Type::None => 0,
            save::Type::Sram => 32 * 1024,
            save::Type::Flash64k | save::Type::Flash64kAtmel => 64 * 1024,
            save::Type::Flash128k => 128 * 1024,
            save::Type::Eeprom512 => 512,
            save::Type::Eeprom8k => 8 * 1024,
        };
        if data.len() != required {
            return Err(format!("initial save: {} bytes, expected {required}", data.len()).into());
        }
    }
    if config.gyro_sample.is_some_and(|v| v > 4095) {
        return Err("initial gyro sample must fit in 12 bits".into());
    }
    let initial_tilt = config.tilt.map(tilt).transpose()?;
    let mut machine = Machine::new(machine::Config {
        rom: &rom,
        bios: &bios,
        boot,
        save_type,
        save_data: save.as_deref(),
        rtc_epoch_seconds: config.rtc_epoch,
        solar_level: config.solar_level,
        gyro_sample: config.gyro_sample,
        tilt_sample: initial_tilt,
        rumble: config.rumble,
    });
    machine.set_audio_output(config.audio_output);
    Ok(machine)
}

pub fn run_linked(
    path: &Path,
    scenario: &Scenario<InputChange>,
    slice: u64,
) -> Result<Vec<LinkedTrace>, Box<dyn Error>> {
    scenario.validate()?;
    let link = scenario.link.as_ref().ok_or("missing link")?;
    let connection = match link.connection {
        Connection::Cable2p => SioConnection::Cable2p,
        Connection::CableMulti => SioConnection::CableMulti,
    };
    let mut machines = vec![make_machine(path, &scenario.assets, &scenario.config)?];
    for peer in &link.peers {
        machines.push(make_machine(path, &peer.assets, &peer.config)?);
    }
    let mut traces = vec![Trace::default(); machines.len()];
    let mut audio = vec![Vec::<u8>::new(); machines.len()];
    let mut buffer = [evenfall_host::StereoSample::default(); 4096];
    for &checkpoint in &scenario.checkpoints {
        while machines.iter().any(|m| m.now() < checkpoint) {
            let before = machines.iter().map(|m| m.now()).min().unwrap();
            let target = before.saturating_add(slice).min(checkpoint).max(before);
            let result = machine::run_linked_until(
                &mut machines.iter_mut().map(|m| &mut **m).collect::<Vec<_>>(),
                target,
                connection,
            )
            .map_err(|e| format!("linked cable: {e:?}"))?;
            if result.reason != machine::LinkedStopReason::Deadline {
                return Err(format!("linked stop at {target}: {:?}", result.reason).into());
            }
            for (index, m) in machines.iter_mut().enumerate() {
                loop {
                    let n = m.drain_audio_samples(&mut buffer);
                    for sample in &buffer[..n] {
                        audio[index].extend_from_slice(&sample.left.to_le_bytes());
                        audio[index].extend_from_slice(&sample.right.to_le_bytes());
                    }
                    if n < buffer.len() {
                        break;
                    }
                }
            }
            if machines.iter().map(|m| m.now()).min().unwrap() == before && target > before {
                return Err("linked run made no progress".into());
            }
        }
        for (index, machine) in machines.iter_mut().enumerate() {
            let cpu = machine.inspect_cpu();
            let (terminal_pc, terminal_r12) = if index == 0 {
                (scenario.terminal_pc, scenario.terminal_r12)
            } else {
                let peer = &link.peers[index - 1];
                (peer.terminal_pc, peer.terminal_r12)
            };
            if checkpoint == scenario.end_cycle {
                if terminal_pc.is_some_and(|want| want != cpu.next_instruction_address) {
                    return Err(format!(
                        "machine {index}: terminal PC {:08x}, expected {terminal_pc:?}",
                        cpu.next_instruction_address
                    )
                    .into());
                }
                if terminal_r12.is_some_and(|want| want != cpu.registers[12]) {
                    return Err(format!(
                        "machine {index}: terminal r12 {:08x}, expected {terminal_r12:?}",
                        cpu.registers[12]
                    )
                    .into());
                }
            }
            let mut frame =
                Vec::with_capacity(evenfall_replay::WIDTH * evenfall_replay::HEIGHT * 2);
            for pixel in machine.framebuffer() {
                frame.extend_from_slice(&pixel.to_le_bytes());
            }
            if frame.len() != evenfall_replay::WIDTH * evenfall_replay::HEIGHT * 2 {
                return Err(format!("machine {index}: wrong frame length").into());
            }
            traces[index].0.push(Observation {
                requested_cycle: checkpoint,
                cycle: machine.now(),
                frames: machine.completed_frames(),
                cpu: evenfall_replay::Cpu {
                    registers: cpu.registers,
                    pc: cpu.next_instruction_address,
                    cpsr: cpu.cpsr,
                },
                audio_rate: machine.audio_sample_rate(),
                frame,
                audio: std::mem::take(&mut audio[index]),
                save: machine.save_data().to_vec(),
                state: machine
                    .write_state_alloc()
                    .map_err(|e| format!("machine {index} state: {e:?}"))?,
            });
        }
    }
    Ok(traces.into_iter().zip(machines).collect())
}

/// Return the most specific field containing the first mismatching byte.
/// The codec trace is payload-relative, after the state's 133-byte header.
pub fn state_field(machine: &mut Machine, state: &[u8], offset: usize) -> String {
    const HEADER: usize = Machine::STATE_HEADER_SIZE;
    if offset < HEADER {
        return match offset {
            0..8 => "header.magic",
            8..12 => "header.version",
            12..44 => "header.schema_digest",
            44..52 => "header.payload_length",
            52 => "header.boot_mode",
            53..61 => "header.rom_length",
            61..93 => "header.rom_digest",
            93..101 => "header.bios_length",
            _ => "header.bios_digest",
        }
        .into();
    }
    if state.len() < HEADER + 32 || offset >= state.len() - 32 {
        return "payload_digest (or length)".into();
    }
    if machine.write_state_alloc().ok().as_deref() != Some(state)
        && machine.load_state(state).is_err()
    {
        return "payload (checkpoint state cannot be loaded)".into();
    }
    let Ok(fields) = machine.state_field_ranges() else {
        return "payload (field trace unavailable)".into();
    };
    let relative = (offset - HEADER) as u64;
    fields
        .iter()
        .filter(|(_, span)| span.contains(&relative))
        .min_by_key(|(_, span)| span.end - span.start)
        .map_or_else(|| "payload (untraced)".into(), |(path, _)| path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_mask_uses_all_ten_independent_pressed_bits() {
        let bits = keys(0b10_0000_1001).unwrap();
        assert!(bits.a && bits.start && bits.l);
        assert!(!bits.b && !bits.select && !bits.r);
        assert!(keys(0x400).is_err());
    }

    #[test]
    fn sensors_reject_values_outside_twelve_bits() {
        assert!(change(InputChange::GyroSample(4096)).is_err());
        assert!(change(InputChange::Tilt([0, 4096])).is_err());
        assert!(change(InputChange::Tilt([4095, 1])).is_ok());
    }

    #[test]
    fn state_offsets_name_header_payload_and_digest() {
        let mut machine = Machine::new(machine::Config::default());
        let state = machine.write_state_alloc().unwrap();
        assert_eq!(state_field(&mut machine, &state, 12), "header.schema_digest");
        assert_eq!(state_field(&mut machine, &state, Machine::STATE_HEADER_SIZE), "cycle");
        assert_eq!(
            state_field(&mut machine, &state, state.len() - 1),
            "payload_digest (or length)"
        );
    }
}
