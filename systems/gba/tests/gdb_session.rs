//! `evenfall-gba gdb` over TCP with a minimal remote-protocol client: stops,
//! interrupts and faults, and the guest state a session leaves compared with
//! `run` for the same instruction count.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

struct Session {
    child: Child,
    stdout: BufReader<ChildStdout>,
    stream: TcpStream,
    acks: bool,
}

impl Session {
    fn start(rom: &Path, extra: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_evenfall-gba"))
            .args(["gdb", "--listen", "127.0.0.1:0", "--save-type", "none"])
            .args(extra)
            .arg(rom)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let address = line.trim_end().strip_prefix("gdb listening=").expect(&line).to_owned();
        let stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        Self { child, stdout, stream, acks: true }
    }

    fn byte(&mut self) -> u8 {
        let mut byte = [0];
        self.stream.read_exact(&mut byte).unwrap();
        byte[0]
    }

    fn send(&mut self, body: &str) {
        let sum = body.bytes().fold(0u8, |sum, byte| sum.wrapping_add(byte));
        self.stream.write_all(format!("${body}#{sum:02x}").as_bytes()).unwrap();
        if self.acks {
            assert_eq!(self.byte(), b'+', "ack for {body}");
        }
    }

    /// Reads one reply packet, checks its checksum and acknowledges it.
    fn receive(&mut self) -> String {
        assert_eq!(self.byte(), b'$');
        let mut body = Vec::new();
        loop {
            match self.byte() {
                b'#' => break,
                byte => body.push(byte),
            }
        }
        let checksum = [self.byte(), self.byte()];
        let sum = body.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte));
        assert_eq!(std::str::from_utf8(&checksum).unwrap(), format!("{sum:02x}"));
        if self.acks {
            self.stream.write_all(b"+").unwrap();
        }
        String::from_utf8(body).unwrap()
    }

    fn request(&mut self, body: &str) -> String {
        self.send(body);
        self.receive()
    }

    /// Waits for the runner to exit and returns the rest of its output.
    fn finish(mut self) -> String {
        let mut rest = String::new();
        self.stdout.read_to_string(&mut rest).unwrap();
        assert!(self.child.wait().unwrap().success(), "{rest}");
        rest
    }
}

fn scratch() -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "evenfall-gdb-session-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    directory
}

fn rom(directory: &Path, name: &str, words: &[u32]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, words.iter().flat_map(|word| word.to_le_bytes()).collect::<Vec<_>>())
        .unwrap();
    path
}

/// Breakpoints, steps, reads and an interrupt leave the machine exactly as
/// `run` leaves it after the same number of instructions.
#[test]
fn a_debugged_run_matches_an_uninterrupted_run_of_the_same_length() {
    let directory = scratch();
    // mov r0, #0; loop: add r0, r0, #1; b loop
    let rom = rom(&directory, "count.gba", &[0xe3a0_0000, 0xe280_0001, 0xeaff_fffd]);
    let (debug_trace, debug_state) = (directory.join("debug.trace"), directory.join("debug.state"));
    let mut session = Session::start(
        &rom,
        &["--trace", debug_trace.to_str().unwrap(), "--state-out", debug_state.to_str().unwrap()],
    );

    assert!(session.request("qSupported:swbreak+").starts_with("PacketSize="));
    assert_eq!(session.request("?"), "S05");
    let registers = session.request("g");
    assert_eq!(&registers[15 * 8..], "000000081f000000");
    assert_eq!(session.request("Z0,8000004,4"), "OK");
    assert_eq!(session.request("c"), "S05");
    assert_eq!(session.request("pf"), "04000008");
    assert_eq!(session.request("p0"), "00000000");
    assert_eq!(session.request("c"), "S05");
    assert_eq!(session.request("p0"), "01000000");
    assert_eq!(session.request("z0,8000004,4"), "OK");
    assert_eq!(session.request("s"), "S05");
    assert_eq!(session.request("pf"), "08000008");

    session.send("c");
    std::thread::sleep(Duration::from_millis(50));
    session.stream.write_all(b"\x03").unwrap();
    assert_eq!(session.receive(), "S02");
    // A negative acknowledgement asks for the last packet again.
    session.stream.write_all(b"-").unwrap();
    assert_eq!(session.receive(), "S02");
    assert_eq!(session.request("QStartNoAckMode"), "OK");
    session.acks = false;
    assert_eq!(session.request("m8000000,4"), "0000a0e3");
    assert_eq!(session.request("D"), "OK");
    let debugged = session.finish();
    assert!(debugged.starts_with("gdb connected="), "{debugged}");

    let trace = std::fs::read(&debug_trace).unwrap();
    let steps = trace.len() / 16;
    assert!(steps > 3);
    let (run_trace, run_state) = (directory.join("run.trace"), directory.join("run.state"));
    let output = Command::new(env!("CARGO_BIN_EXE_evenfall-gba"))
        .args(["run", "--steps", &steps.to_string(), "--quiet", "--save-type", "none"])
        .args(["--trace", run_trace.to_str().unwrap(), "--state-out", run_state.to_str().unwrap()])
        .arg(&rom)
        .output()
        .unwrap();
    assert!(output.status.success());
    let ran = String::from_utf8(output.stdout).unwrap();
    assert!(debugged.ends_with(&ran), "debugged: {debugged}\nran: {ran}");
    assert_eq!(std::fs::read(&run_trace).unwrap(), trace);
    assert_eq!(std::fs::read(&run_state).unwrap(), std::fs::read(&debug_state).unwrap());
    std::fs::remove_dir_all(directory).unwrap();
}

/// A stopped CPU keeps waiting for a wake event until the client interrupts.
#[test]
fn an_interrupt_reaches_a_sleeping_cpu_and_kill_ends_the_session() {
    let directory = scratch();
    // swi 0x03 (Stop) with no wake source.
    let rom = rom(&directory, "stop.gba", &[0xef03_0000]);
    let mut session = Session::start(&rom, &[]);
    session.send("c");
    std::thread::sleep(Duration::from_millis(50));
    session.stream.write_all(b"\x03").unwrap();
    assert_eq!(session.receive(), "S02");
    assert_eq!(session.request("pf"), "04000008");
    session.send("k");
    let output = session.finish();
    assert!(output.contains("\nstop cycle="), "{output}");
    assert!(output.contains(" next=08000004 "), "{output}");
    std::fs::remove_dir_all(directory).unwrap();
}

/// An emulation fault stops with SIGABRT, prints its diagnostic once and
/// keeps the machine from running on; the state is not saved.
#[test]
fn a_fault_stops_the_session_with_sigabrt() {
    let directory = scratch();
    // msr cpsr_c, #0x3f: an illegal T-bit change.
    let rom = rom(&directory, "fault.gba", &[0xe321_f03f]);
    let state = directory.join("fault.state");
    let mut session = Session::start(&rom, &["--state-out", state.to_str().unwrap()]);
    assert_eq!(session.request("c"), "S06");
    assert_eq!(session.request("s"), "S06");
    assert_eq!(session.request("?"), "S06");
    assert_eq!(session.request("D"), "OK");
    let output = session.finish();
    assert_eq!(output.matches("emulation fault").count(), 1, "{output}");
    assert!(output.contains("state save skipped: machine faulted\n"), "{output}");
    assert!(!state.exists());
    std::fs::remove_dir_all(directory).unwrap();
}
