//! The headless CLI, with file policy separated from the machine, and its
//! GDB remote debugging mode.

use evenfall_gba::bus::{KeyState, TimedInput, TimedInputChange};
use evenfall_gba::cpu::InstructionSet;
use evenfall_gba::host::gdb::{self, Command, Decoder, Event, Resume, Stop, Stub};
use evenfall_gba::host::host_save;
use evenfall_gba::machine::{BootMode, Config, Fault, Machine, StepOutcome};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};

const USAGE: &str = "usage: evenfall-gba run --steps <count> [--bios <path>] [--save-type <type>] [--save-file <path>] [--state-in <path>] [--state-out <path>] [--trace <path>] [--trace-from <steps>] [--input <movie>] [--cheat <[format:]code>]... [--cheats <file>] [--quiet] [--frame-hash] [--no-rtc | --rtc-epoch <seconds>] [--no-solar | --solar-level <0..255>] [--no-gyro | --gyro-sample <0..4095>] [--rumble | --no-rumble] [--no-tilt | --tilt <x> <y>] <rom.gba|rom.zip>\n       evenfall-gba gdb [--listen <address:port>] [run options other than --steps] <rom.gba|rom.zip>\n";

const DEFAULT_GDB_ADDRESS: &str = "127.0.0.1:2345";

/// Executed instructions between checks for the client's interrupt byte.
const GDB_POLL_INTERVAL: u32 = 1 << 14;

enum Mode<'a> {
    Run { steps: usize },
    Gdb { listen: &'a str },
}

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

/// Wyhash final v4.1, seed zero. The `wyhash` crate's top-level
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

/// `run --steps <count> ...` or `gdb [--listen <address>] ...`, with the
/// arguments left for the shared option parser.
fn parse_mode(args: &[String]) -> Option<(Mode<'_>, Vec<&str>)> {
    let mut tail: Vec<&str> = args.get(2..)?.iter().map(String::as_str).collect();
    match args[1].as_str() {
        "run" if args.len() >= 5 && args[2] == "--steps" => {
            let steps = args[3].parse().ok()?;
            tail.drain(..2);
            Some((Mode::Run { steps }, tail))
        }
        "gdb" => {
            let mut listen = DEFAULT_GDB_ADDRESS;
            if let Some(index) = tail.iter().position(|&arg| arg == "--listen") {
                listen = *tail.get(index + 1)?;
                tail.drain(index..index + 2);
            }
            (!tail.contains(&"--listen")).then_some((Mode::Gdb { listen }, tail))
        }
        _ => None,
    }
}

fn run(args: &[String], stdout: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
    let Some((mode, tail)) = parse_mode(args) else {
        stdout.write_all(USAGE.as_bytes())?;
        return Ok(());
    };
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
    if !options.cheats.is_empty() || options.cheat_file.is_some() {
        let mut cheats = Vec::new();
        if let Some(path) = options.cheat_file {
            let text = std::fs::read_to_string(path)?;
            cheats = host_save::parse_cheat_file(&text)
                .map_err(|(number, error)| format!("cheat {number} in {path}: {error}"))?;
        }
        for spec in &options.cheats {
            cheats.push(
                host_save::parse_cheat_spec(spec)
                    .map_err(|error| format!("cheat {spec:?}: {error}"))?,
            );
        }
        machine.set_cheats(cheats).map_err(|error| format!("cheats: {error}"))?;
    }
    let trace = match options.trace_path {
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

    let mut execution = Execution {
        machine,
        inputs,
        input_index,
        trace,
        trace_from: options.trace_from,
        executed_count: 0,
    };
    let terminal_fault = match mode {
        Mode::Run { steps } => {
            let fault = run_steps(&mut execution, steps, options.quiet, stdout)?;
            if let Some(fault) = fault {
                fault.write_diagnostic(stdout)?;
            }
            fault
        }
        Mode::Gdb { listen } => debug(&mut execution, listen, stdout)?,
    };
    let machine = execution.machine;
    if let Some(mut trace) = execution.trace {
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

/// The machine with its scheduled input and trace. Both modes advance it
/// through the same two operations, so a debugger's stops never change what
/// the guest computes.
struct Execution {
    machine: Box<Machine>,
    inputs: Vec<TimedInput>,
    input_index: usize,
    trace: Option<io::BufWriter<std::fs::File>>,
    trace_from: usize,
    executed_count: usize,
}

impl Execution {
    /// One instruction, receiving input scheduled during it.
    fn step(&mut self) -> Result<StepOutcome, Box<dyn std::error::Error>> {
        let outcome = if self.input_index < self.inputs.len() {
            let result = self
                .machine
                .step_with_input(&self.inputs[self.input_index..self.input_index + 1])
                .map_err(|error| format!("{error:?}"))?;
            self.input_index += result.inputs_consumed;
            result.outcome
        } else {
            self.machine.step()
        };
        if let StepOutcome::Executed(step) = outcome {
            if let Some(trace) =
                self.trace.as_mut().filter(|_| self.executed_count >= self.trace_from)
            {
                let mut record = [0; 16];
                record[0..4].copy_from_slice(&step.instruction_address.to_le_bytes());
                record[4..8].copy_from_slice(&step.instruction.to_le_bytes());
                record[8..12].copy_from_slice(&self.machine.read_cpsr().to_le_bytes());
                record[12] = match step.instruction_set {
                    InstructionSet::Arm => 0,
                    InstructionSet::Thumb => 1,
                };
                trace.write_all(&record)?;
            }
            self.executed_count += 1;
        }
        Ok(outcome)
    }

    /// Sleeps until a wake event, the next scheduled input or one frame,
    /// whichever comes first. Returns whether the CPU woke and whether time
    /// passed or input arrived.
    fn wait(&mut self) -> Result<(bool, bool), Box<dyn std::error::Error>> {
        let waiting_at = self.machine.now();
        let frame_deadline = waiting_at.saturating_add(Machine::CYCLES_PER_FRAME);
        let next_input = self.inputs.get(self.input_index);
        let deadline = next_input.map_or(frame_deadline, |event| frame_deadline.min(event.at));
        let waiting_inputs = next_input.map_or(&[][..], std::slice::from_ref);
        let progress = self
            .machine
            .wait_until_with_input(deadline, waiting_inputs)
            .map_err(|error| format!("{error:?}"))?;
        self.input_index += progress.inputs_consumed;
        Ok((progress.woke, self.machine.now() != waiting_at || progress.inputs_consumed != 0))
    }
}

/// `run`: a fixed instruction count, ending early when the CPU
/// sleeps with nothing left to wake it.
fn run_steps(
    execution: &mut Execution,
    steps: usize,
    quiet: bool,
    stdout: &mut impl Write,
) -> Result<Option<Fault>, Box<dyn std::error::Error>> {
    while execution.executed_count < steps {
        match execution.step()? {
            StepOutcome::Executed(step) => {
                if !quiet {
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
            }
            StepOutcome::Waiting(reason) => {
                let woke = loop {
                    let (woke, progressed) = execution.wait()?;
                    if woke || !progressed || execution.input_index == execution.inputs.len() {
                        break woke;
                    }
                };
                if woke {
                    continue;
                }
                if !quiet {
                    writeln!(
                        stdout,
                        "wait cycle={} power={}",
                        execution.machine.now(),
                        match reason {
                            evenfall_gba::cpu::WaitReason::Halted => "halted",
                            evenfall_gba::cpu::WaitReason::Stopped => "stopped",
                            evenfall_gba::cpu::WaitReason::DmaStall => "dma_stall",
                        }
                    )?;
                }
                return Ok(None);
            }
            StepOutcome::Faulted(fault) => return Ok(Some(fault)),
        }
    }
    Ok(None)
}

/// Serves one GDB client until it detaches, kills or disconnects. Returns
/// the machine's latched fault, if any.
fn debug(
    execution: &mut Execution,
    listen: &str,
    stdout: &mut impl Write,
) -> Result<Option<Fault>, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(listen)?;
    writeln!(stdout, "gdb listening={}", listener.local_addr()?)?;
    stdout.flush()?;
    let (stream, peer) = listener.accept()?;
    drop(listener);
    stream.set_nodelay(true)?;
    writeln!(stdout, "gdb connected={peer}")?;
    stdout.flush()?;

    let mut connection =
        Connection { stream, decoder: Decoder::default(), events: VecDeque::new() };
    let mut stub = Stub::default();
    let mut last_packet = Vec::new();
    while let Some(event) = connection.next_event()? {
        match event {
            Event::Packet(packet) => {
                if !stub.no_ack() {
                    connection.stream.write_all(b"+")?;
                }
                let reply = match stub.handle(&mut execution.machine, &packet) {
                    Command::Reply(reply) => reply,
                    Command::Resume(resume) => {
                        let faulted_before = execution.machine.fault().is_some();
                        let Some(stop) =
                            resume_until_stop(execution, &stub, resume, &mut connection)?
                        else {
                            break;
                        };
                        if let (Stop::Faulted, false, Some(fault)) =
                            (stop, faulted_before, execution.machine.fault())
                        {
                            fault.write_diagnostic(stdout)?;
                            stdout.flush()?;
                        }
                        stub.stop_reply(stop)
                    }
                    Command::End(reply) => {
                        if let Some(reply) = reply {
                            connection.stream.write_all(&gdb::frame(&reply))?;
                        }
                        break;
                    }
                };
                last_packet = gdb::frame(&reply);
                connection.stream.write_all(&last_packet)?;
            }
            Event::Corrupt if !stub.no_ack() => connection.stream.write_all(b"-")?,
            Event::Nack if !stub.no_ack() => connection.stream.write_all(&last_packet)?,
            _ => {}
        }
    }
    writeln!(stdout, "gdb disconnected")?;
    Ok(execution.machine.fault())
}

/// Executes whole instructions until one completes (a step), a breakpoint
/// is next, the machine faults or the client interrupts. `None` means the
/// client disconnected.
fn resume_until_stop(
    execution: &mut Execution,
    stub: &Stub,
    resume: Resume,
    connection: &mut Connection,
) -> Result<Option<Stop>, Box<dyn std::error::Error>> {
    let mut since_poll = 0;
    loop {
        match execution.step()? {
            StepOutcome::Executed(_) => {
                if resume == Resume::Step
                    || stub.breakpoint_at(execution.machine.next_instruction_address())
                {
                    return Ok(Some(Stop::Trap));
                }
                since_poll += 1;
                if since_poll < GDB_POLL_INTERVAL {
                    continue;
                }
                since_poll = 0;
                if let Some(stop) = connection.poll_interrupt()? {
                    return Ok(stop);
                }
            }
            StepOutcome::Waiting(_) => loop {
                if execution.wait()?.0 {
                    break;
                }
                if execution.machine.fault().is_some() {
                    return Ok(Some(Stop::Faulted));
                }
                if let Some(stop) = connection.poll_interrupt()? {
                    return Ok(stop);
                }
            },
            StepOutcome::Faulted(_) => return Ok(Some(Stop::Faulted)),
        }
    }
}

struct Connection {
    stream: TcpStream,
    decoder: Decoder,
    events: VecDeque<Event>,
}

impl Connection {
    /// The next client event, blocking; `None` once the client disconnects.
    fn next_event(&mut self) -> io::Result<Option<Event>> {
        let mut buffer = [0; 4096];
        while self.events.is_empty() {
            let count = match self.stream.read(&mut buffer) {
                Ok(0) => return Ok(None),
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            self.events.extend(buffer[..count].iter().filter_map(|&byte| self.decoder.feed(byte)));
        }
        Ok(self.events.pop_front())
    }

    /// Reads what the client sent while the machine ran, without waiting.
    /// `Some(Some(Interrupted))` for the interrupt byte, `Some(None)` for a
    /// disconnect; other events wait for the stop.
    fn poll_interrupt(&mut self) -> io::Result<Option<Option<Stop>>> {
        self.stream.set_nonblocking(true)?;
        let mut buffer = [0; 4096];
        let mut result = None;
        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => {
                    result = Some(None);
                    break;
                }
                Ok(count) => {
                    for &byte in &buffer[..count] {
                        match self.decoder.feed(byte) {
                            Some(Event::Interrupt) => result = Some(Some(Stop::Interrupted)),
                            Some(event) => self.events.push_back(event),
                            None => {}
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    self.stream.set_nonblocking(false)?;
                    return Err(error);
                }
            }
        }
        self.stream.set_nonblocking(false)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::frame_wyhash;

    /// Reference results for Wyhash final v4.1 with seed zero, computed with
    /// Zig 0.16's `std.hash.Wyhash`, including either side of both the 16-
    /// and 48-byte block boundaries.
    #[test]
    fn frame_hash_matches_reference_wyhash_vectors() {
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
