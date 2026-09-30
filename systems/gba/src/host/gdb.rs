//! GDB remote serial protocol for the native runner: packet framing and the
//! commands of an all-stop, single-thread ARMv4T target.
//!
//! The stub only inspects and, on request, writes machine state; it never
//! runs the machine. `c` and `s` become [`Command::Resume`], and the caller
//! executes whole instructions, asking [`Stub::breakpoint_at`] after each one,
//! so guest results do not depend on where a debugger stops execution.

use crate::machine::{DebugWriteError, Machine};

/// The largest packet body either side sends, advertised in `qSupported`.
pub const PACKET_SIZE: usize = 0x4000;

/// ARM core registers r0–r15 and CPSR; gdb's `org.gnu.gdb.arm.core` numbers
/// CPSR 25, but `g` carries it directly after PC because only described
/// registers occupy the packet.
pub const TARGET_XML: &str = concat!(
    r#"<?xml version="1.0"?><!DOCTYPE target SYSTEM "gdb-target.dtd">"#,
    r#"<target version="1.0"><architecture>armv4t</architecture><osabi>none</osabi>"#,
    r#"<feature name="org.gnu.gdb.arm.core">"#,
    r#"<reg name="r0" bitsize="32" type="uint32"/><reg name="r1" bitsize="32" type="uint32"/>"#,
    r#"<reg name="r2" bitsize="32" type="uint32"/><reg name="r3" bitsize="32" type="uint32"/>"#,
    r#"<reg name="r4" bitsize="32" type="uint32"/><reg name="r5" bitsize="32" type="uint32"/>"#,
    r#"<reg name="r6" bitsize="32" type="uint32"/><reg name="r7" bitsize="32" type="uint32"/>"#,
    r#"<reg name="r8" bitsize="32" type="uint32"/><reg name="r9" bitsize="32" type="uint32"/>"#,
    r#"<reg name="r10" bitsize="32" type="uint32"/><reg name="r11" bitsize="32" type="uint32"/>"#,
    r#"<reg name="r12" bitsize="32" type="uint32"/>"#,
    r#"<reg name="sp" bitsize="32" type="data_ptr"/><reg name="lr" bitsize="32"/>"#,
    r#"<reg name="pc" bitsize="32" type="code_ptr"/>"#,
    r#"<reg name="cpsr" bitsize="32" regnum="25"/>"#,
    r#"</feature></target>"#,
);

/// The GBA's decoded address regions without mirrors. Clients refuse
/// accesses outside them and use hardware breakpoints in `rom` regions,
/// which this stub treats like software breakpoints.
pub const MEMORY_MAP_XML: &str = concat!(
    r#"<?xml version="1.0"?><!DOCTYPE memory-map SYSTEM "memory-map.dtd">"#,
    r#"<memory-map>"#,
    r#"<memory type="rom" start="0x00000000" length="0x4000"/>"#,
    r#"<memory type="ram" start="0x02000000" length="0x40000"/>"#,
    r#"<memory type="ram" start="0x03000000" length="0x8000"/>"#,
    r#"<memory type="ram" start="0x04000000" length="0x400"/>"#,
    r#"<memory type="ram" start="0x05000000" length="0x400"/>"#,
    r#"<memory type="ram" start="0x06000000" length="0x18000"/>"#,
    r#"<memory type="ram" start="0x07000000" length="0x400"/>"#,
    r#"<memory type="rom" start="0x08000000" length="0x6000000"/>"#,
    r#"<memory type="ram" start="0x0e000000" length="0x10000"/>"#,
    r#"</memory-map>"#,
);

const CPSR_REGISTER: usize = 25;
const SIGINT: u8 = 2;
const SIGTRAP: u8 = 5;
const SIGABRT: u8 = 6;

/// One unit of client input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A packet body with a valid checksum, still escaped.
    Packet(Vec<u8>),
    /// A packet whose checksum failed or whose body exceeded [`PACKET_SIZE`].
    Corrupt,
    /// The interrupt byte 0x03 between packets.
    Interrupt,
    Ack,
    Nack,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DecodeState {
    #[default]
    Idle,
    Body,
    ChecksumHigh,
    ChecksumLow(u8),
}

/// Incremental `$body#xx` decoder; bytes outside packets other than acks and
/// interrupts are ignored.
#[derive(Debug, Default)]
pub struct Decoder {
    state: DecodeState,
    body: Vec<u8>,
    sum: u8,
    overflowed: bool,
}

impl Decoder {
    pub fn feed(&mut self, byte: u8) -> Option<Event> {
        match self.state {
            DecodeState::Idle => match byte {
                b'$' => {
                    self.state = DecodeState::Body;
                    self.body.clear();
                    self.sum = 0;
                    self.overflowed = false;
                    None
                }
                0x03 => Some(Event::Interrupt),
                b'+' => Some(Event::Ack),
                b'-' => Some(Event::Nack),
                _ => None,
            },
            DecodeState::Body => {
                if byte == b'#' {
                    self.state = DecodeState::ChecksumHigh;
                } else {
                    self.sum = self.sum.wrapping_add(byte);
                    if self.body.len() < PACKET_SIZE {
                        self.body.push(byte);
                    } else {
                        self.overflowed = true;
                    }
                }
                None
            }
            DecodeState::ChecksumHigh => {
                self.state = match hex_digit(byte) {
                    Some(high) => DecodeState::ChecksumLow(high),
                    None => DecodeState::Idle,
                };
                if self.state == DecodeState::Idle { Some(Event::Corrupt) } else { None }
            }
            DecodeState::ChecksumLow(high) => {
                self.state = DecodeState::Idle;
                match hex_digit(byte) {
                    Some(low) if high << 4 | low == self.sum && !self.overflowed => {
                        Some(Event::Packet(std::mem::take(&mut self.body)))
                    }
                    _ => Some(Event::Corrupt),
                }
            }
        }
    }
}

/// Frames a reply body as `$body#xx`, escaping `#`, `$`, `}` and `*`.
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(body.len() + 4);
    packet.push(b'$');
    let mut sum = 0u8;
    for &byte in body {
        let escaped: &[u8] = if matches!(byte, b'#' | b'$' | b'}' | b'*') {
            &[b'}', byte ^ 0x20]
        } else {
            std::slice::from_ref(&byte)
        };
        for &byte in escaped {
            sum = sum.wrapping_add(byte);
            packet.push(byte);
        }
    }
    packet.push(b'#');
    push_hex(&mut packet, &[sum]);
    packet
}

/// Why execution stopped, reported to the client as a signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A completed single step or a breakpoint: SIGTRAP.
    Trap,
    /// The client's interrupt byte: SIGINT.
    Interrupted,
    /// A latched emulation fault; the machine cannot run on: SIGABRT.
    Faulted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    Step,
    Continue,
}

/// What the caller does with a packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Reply(Vec<u8>),
    /// Run, then send [`Stub::stop_reply`].
    Resume(Resume),
    /// Send the reply, if any, and end the session.
    End(Option<Vec<u8>>),
}

#[derive(Debug)]
pub struct Stub {
    breakpoints: Vec<u32>,
    no_ack: bool,
    last_stop: Stop,
}

impl Default for Stub {
    fn default() -> Self {
        Self { breakpoints: Vec::new(), no_ack: false, last_stop: Stop::Trap }
    }
}

impl Stub {
    /// Whether `QStartNoAckMode` ended acknowledgements in both directions.
    pub fn no_ack(&self) -> bool {
        self.no_ack
    }

    /// Whether a breakpoint sits at the next instruction's address.
    pub fn breakpoint_at(&self, address: u32) -> bool {
        self.breakpoints.contains(&address)
    }

    pub fn stop_reply(&mut self, stop: Stop) -> Vec<u8> {
        self.last_stop = stop;
        let signal = match stop {
            Stop::Trap => SIGTRAP,
            Stop::Interrupted => SIGINT,
            Stop::Faulted => SIGABRT,
        };
        let mut reply = vec![b'S'];
        push_hex(&mut reply, &[signal]);
        reply
    }

    pub fn handle(&mut self, machine: &mut Machine, packet: &[u8]) -> Command {
        let Some((&kind, arguments)) = packet.split_first() else {
            return Command::Reply(Vec::new());
        };
        let reply = match kind {
            b'?' => {
                let stop = self.last_stop;
                self.stop_reply(stop)
            }
            b'g' => {
                let mut reply = Vec::with_capacity(17 * 8);
                for index in (0..16).chain([CPSR_REGISTER]) {
                    push_hex(&mut reply, &read_register(machine, index).unwrap().to_le_bytes());
                }
                reply
            }
            b'G' => write_all_registers(machine, arguments),
            b'p' => match parse_hex(arguments).and_then(|index| read_register(machine, index)) {
                Some(value) => hex(&value.to_le_bytes()),
                None => error(0),
            },
            b'P' => write_one_register(machine, arguments),
            b'm' => read_memory(machine, arguments),
            b'M' => write_memory(machine, arguments),
            b'c' | b's' | b'C' | b'S' => return self.resume(machine, kind, arguments),
            b'Z' | b'z' => self.breakpoint(kind == b'Z', arguments),
            b'D' => return Command::End(Some(b"OK".to_vec())),
            b'k' => return Command::End(None),
            b'H' | b'T' => b"OK".to_vec(),
            b'q' | b'Q' | b'v' => return self.query(packet),
            _ => Vec::new(),
        };
        Command::Reply(reply)
    }

    fn resume(&mut self, machine: &mut Machine, kind: u8, arguments: &[u8]) -> Command {
        // `c addr`, `s addr`, `C sig;addr` and `S sig;addr`; signals are ignored.
        let address = if kind.is_ascii_uppercase() {
            arguments.iter().position(|&byte| byte == b';').map(|at| &arguments[at + 1..])
        } else {
            (!arguments.is_empty()).then_some(arguments)
        };
        if let Some(address) = address {
            let Some(address) = parse_hex(address).and_then(|value| u32::try_from(value).ok())
            else {
                return Command::Reply(error(1));
            };
            if machine.write_register(15, address).is_err() {
                return Command::Reply(error(2));
            }
        }
        Command::Resume(if kind.eq_ignore_ascii_case(&b's') {
            Resume::Step
        } else {
            Resume::Continue
        })
    }

    fn breakpoint(&mut self, insert: bool, arguments: &[u8]) -> Vec<u8> {
        // Z0 software and Z1 hardware breakpoints behave alike: both are
        // checked between instructions and never touch guest memory.
        let mut fields = arguments.split(|&byte| byte == b',');
        let (Some(b"0" | b"1"), Some(address)) = (fields.next(), fields.next()) else {
            return Vec::new();
        };
        let Some(address) = parse_hex(address).and_then(|value| u32::try_from(value).ok()) else {
            return error(1);
        };
        let existing = self.breakpoints.iter().position(|&known| known == address);
        match (insert, existing) {
            (true, None) => self.breakpoints.push(address),
            (false, Some(index)) => {
                self.breakpoints.swap_remove(index);
            }
            _ => {}
        }
        b"OK".to_vec()
    }

    fn query(&mut self, packet: &[u8]) -> Command {
        let reply: Vec<u8> = if packet.starts_with(b"qSupported") {
            format!(
                "PacketSize={PACKET_SIZE:x};qXfer:features:read+;qXfer:memory-map:read+;QStartNoAckMode+"
            )
            .into_bytes()
        } else if packet == b"QStartNoAckMode" {
            self.no_ack = true;
            b"OK".to_vec()
        } else if let Some(range) = packet.strip_prefix(b"qXfer:features:read:target.xml:") {
            transfer(TARGET_XML, range)
        } else if let Some(range) = packet.strip_prefix(b"qXfer:memory-map:read::") {
            transfer(MEMORY_MAP_XML, range)
        } else if packet.starts_with(b"qXfer:") {
            error(0)
        } else if packet == b"qAttached" {
            b"1".to_vec()
        } else if packet == b"qC" {
            b"QC1".to_vec()
        } else if packet == b"qfThreadInfo" {
            b"m1".to_vec()
        } else if packet == b"qsThreadInfo" {
            b"l".to_vec()
        } else if packet.starts_with(b"vKill") {
            return Command::End(Some(b"OK".to_vec()));
        } else {
            Vec::new()
        };
        Command::Reply(reply)
    }
}

/// Register `index` in gdb's ARM numbering: r0–r15 (r15 is the next
/// instruction's address) and CPSR at 25.
fn read_register(machine: &Machine, index: usize) -> Option<u32> {
    match index {
        0..15 => Some(machine.read_register(index as u8)),
        15 => Some(machine.next_instruction_address()),
        CPSR_REGISTER => Some(machine.read_cpsr()),
        _ => None,
    }
}

fn write_register(machine: &mut Machine, index: usize, value: u32) -> Result<(), DebugWriteError> {
    match index {
        0..16 => machine.write_register(index as u8, value),
        _ => machine.write_cpsr(value),
    }
}

fn write_all_registers(machine: &mut Machine, arguments: &[u8]) -> Vec<u8> {
    let Some(bytes) = decode_hex(arguments).filter(|bytes| bytes.len() == 17 * 4) else {
        return error(1);
    };
    let value = |slot: usize| u32::from_le_bytes(bytes[slot * 4..][..4].try_into().unwrap());
    // CPSR before PC, so PC aligns to the new instruction set; only changed
    // registers are written, so an unchanged PC keeps its pipeline.
    for (slot, index) in (0..15).map(|index| (index, index)).chain([(16, CPSR_REGISTER), (15, 15)])
    {
        if read_register(machine, index) != Some(value(slot))
            && write_register(machine, index, value(slot)).is_err()
        {
            return error(2);
        }
    }
    b"OK".to_vec()
}

fn write_one_register(machine: &mut Machine, arguments: &[u8]) -> Vec<u8> {
    let mut fields = arguments.splitn(2, |&byte| byte == b'=');
    let (Some(index), Some(value)) = (fields.next().and_then(parse_hex), fields.next()) else {
        return error(1);
    };
    let Some(value) = decode_hex(value).and_then(|bytes| <[u8; 4]>::try_from(bytes).ok()) else {
        return error(1);
    };
    if read_register(machine, index).is_none() {
        return error(1);
    }
    match write_register(machine, index, u32::from_le_bytes(value)) {
        Ok(()) => b"OK".to_vec(),
        Err(_) => error(2),
    }
}

/// `addr,length` with both fields hexadecimal and the length bounded so the
/// hex reply fits one packet.
fn parse_range(arguments: &[u8]) -> Option<(u32, usize)> {
    let mut fields = arguments.splitn(2, |&byte| byte == b',');
    let address = u32::try_from(parse_hex(fields.next()?)?).ok()?;
    let length = parse_hex(fields.next()?)?;
    (length <= PACKET_SIZE / 2).then_some((address, length))
}

fn read_memory(machine: &Machine, arguments: &[u8]) -> Vec<u8> {
    let Some((address, length)) = parse_range(arguments) else {
        return error(1);
    };
    let mut bytes = vec![0; length];
    match machine.inspect_memory(address, &mut bytes) {
        Ok(()) => hex(&bytes),
        Err(_) => error(1),
    }
}

fn write_memory(machine: &mut Machine, arguments: &[u8]) -> Vec<u8> {
    let Some(colon) = arguments.iter().position(|&byte| byte == b':') else {
        return error(1);
    };
    let (Some((address, length)), Some(bytes)) =
        (parse_range(&arguments[..colon]), decode_hex(&arguments[colon + 1..]))
    else {
        return error(1);
    };
    if bytes.len() != length {
        return error(1);
    }
    match machine.poke_memory(address, &bytes) {
        Ok(()) => b"OK".to_vec(),
        Err(_) => error(2),
    }
}

/// A `qXfer` read of `offset,length` from a document: `m` and a chunk while
/// more remains, `l` and the rest at the end.
fn transfer(document: &str, range: &[u8]) -> Vec<u8> {
    let Some((offset, length)) = parse_range(range) else {
        return error(0);
    };
    let document = document.as_bytes();
    let start = (offset as usize).min(document.len());
    let end = start.saturating_add(length).min(document.len());
    let mut reply = vec![if end == document.len() { b'l' } else { b'm' }];
    reply.extend_from_slice(&document[start..end]);
    reply
}

fn error(code: u8) -> Vec<u8> {
    let mut reply = vec![b'E'];
    push_hex(&mut reply, &[code]);
    reply
}

fn hex(bytes: &[u8]) -> Vec<u8> {
    let mut text = Vec::with_capacity(bytes.len() * 2);
    push_hex(&mut text, bytes);
    text
}

fn push_hex(text: &mut Vec<u8>, bytes: &[u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for &byte in bytes {
        text.extend_from_slice(&[DIGITS[usize::from(byte >> 4)], DIGITS[usize::from(byte & 15)]]);
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

/// A big-endian hexadecimal number of at most eight digits.
fn parse_hex(text: &[u8]) -> Option<usize> {
    if text.is_empty() || text.len() > 8 {
        return None;
    }
    text.iter().try_fold(0, |value, &byte| Some(value << 4 | usize::from(hex_digit(byte)?)))
}

fn decode_hex(text: &[u8]) -> Option<Vec<u8>> {
    let (pairs, []) = text.as_chunks::<2>() else {
        return None;
    };
    pairs.iter().map(|&[high, low]| Some(hex_digit(high)? << 4 | hex_digit(low)?)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{Config, StepOutcome};

    fn decode(bytes: &[u8]) -> Vec<Event> {
        let mut decoder = Decoder::default();
        bytes.iter().filter_map(|&byte| decoder.feed(byte)).collect()
    }

    /// ARM code at 0x08000000: `mov r0, #42`, `add r1, r0, #1`, then `b .`.
    fn machine() -> Box<Machine> {
        let mut rom = Vec::new();
        for word in [0xe3a0_002a_u32, 0xe280_1001, 0xeaff_fffe] {
            rom.extend_from_slice(&word.to_le_bytes());
        }
        Machine::new(Config { rom: &rom, ..Config::default() })
    }

    fn reply(stub: &mut Stub, machine: &mut Machine, packet: &str) -> String {
        match stub.handle(machine, packet.as_bytes()) {
            Command::Reply(reply) => String::from_utf8(reply).unwrap(),
            command => panic!("{packet}: {command:?}"),
        }
    }

    fn step(machine: &mut Machine) {
        assert!(matches!(machine.step(), StepOutcome::Executed(_)));
    }

    #[test]
    fn decoder_checks_checksums_and_separates_acks_and_interrupts() {
        assert_eq!(
            decode(b"+$g#67-\x03$m0,4#fd"),
            [
                Event::Ack,
                Event::Packet(b"g".to_vec()),
                Event::Nack,
                Event::Interrupt,
                Event::Packet(b"m0,4".to_vec()),
            ]
        );
        assert_eq!(decode(b"$g#68"), [Event::Corrupt]);
        assert_eq!(decode(b"$g#zz"), [Event::Corrupt]);
        // Bytes between packets are noise; 0x03 inside a packet is data.
        assert_eq!(decode(b"junk$\x03#03"), [Event::Packet(vec![3])]);
        // Checksum digits are case-insensitive.
        assert_eq!(decode(b"$qC#B4"), [Event::Packet(b"qC".to_vec())]);
    }

    #[test]
    fn decoder_rejects_an_oversized_packet_and_recovers() {
        let mut bytes = vec![b'$'];
        bytes.resize(PACKET_SIZE + 2, b'0');
        let sum = bytes[1..].iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte));
        bytes.extend_from_slice(format!("#{sum:02x}$g#67").as_bytes());
        assert_eq!(decode(&bytes), [Event::Corrupt, Event::Packet(b"g".to_vec())]);
    }

    #[test]
    fn frames_escape_reserved_bytes_and_checksum_the_escaped_form() {
        assert_eq!(frame(b"OK"), b"$OK#9a");
        assert_eq!(frame(b""), b"$#00");
        assert_eq!(frame(b"a#$}*"), b"$a}\x03}\x04}]}\x0a#c3");
        // A framed body decodes to its escaped form with a valid checksum.
        assert_eq!(decode(&frame(b"x}y")), [Event::Packet(b"x}]y".to_vec())]);
    }

    #[test]
    fn reports_registers_with_the_next_instruction_as_pc_and_cpsr_last() {
        let mut machine = machine();
        let mut stub = Stub::default();
        step(&mut machine);
        let registers = reply(&mut stub, &mut machine, "g");
        assert_eq!(registers.len(), 17 * 8);
        assert_eq!(&registers[..8], "2a000000");
        assert_eq!(&registers[15 * 8..16 * 8], "04000008");
        assert_eq!(&registers[16 * 8..], "1f000000");
        assert_eq!(reply(&mut stub, &mut machine, "pf"), "04000008");
        assert_eq!(reply(&mut stub, &mut machine, "p19"), "1f000000");
        assert_eq!(reply(&mut stub, &mut machine, "p10"), "E00");
        assert_eq!(reply(&mut stub, &mut machine, "pz"), "E00");
    }

    #[test]
    fn writes_registers_and_moves_pc_without_spending_cycles() {
        let mut machine = machine();
        let mut stub = Stub::default();
        step(&mut machine);
        let before = machine.now();
        assert_eq!(reply(&mut stub, &mut machine, "P0=07000000"), "OK");
        // Back to the `add`, which now reads the written r0.
        assert_eq!(reply(&mut stub, &mut machine, "Pf=04000008"), "OK");
        assert_eq!(machine.now(), before);
        step(&mut machine);
        assert_eq!(machine.read_register(1), 8);
        assert_eq!(reply(&mut stub, &mut machine, "P19=00000000"), "E02");
        assert_eq!(reply(&mut stub, &mut machine, "P11=00000000"), "E01");
        assert_eq!(reply(&mut stub, &mut machine, "P0=0000"), "E01");

        // G writes only changed registers; an unchanged PC keeps the pipeline.
        let mut registers = reply(&mut stub, &mut machine, "g");
        registers.replace_range(2 * 8..3 * 8, "efbeadde");
        registers.replace_range(16 * 8.., "d3000080");
        assert_eq!(reply(&mut stub, &mut machine, &format!("G{registers}")), "OK");
        assert_eq!(machine.read_register(2), 0xdead_beef);
        assert_eq!(machine.read_cpsr(), 0x8000_00d3);
        // Supervisor mode banks in its own stack pointer.
        assert_eq!(machine.read_register(13), 0x0300_7fe0);
        assert_eq!(reply(&mut stub, &mut machine, "G00"), "E01");
    }

    #[test]
    fn a_thumb_bit_write_realigns_pc_to_the_new_instruction_set() {
        let mut machine = machine();
        let mut stub = Stub::default();
        assert_eq!(reply(&mut stub, &mut machine, "Pf=02000008"), "OK");
        assert_eq!(machine.next_instruction_address(), 0x0800_0000);
        assert_eq!(reply(&mut stub, &mut machine, "P19=3f000000"), "OK");
        assert_eq!(reply(&mut stub, &mut machine, "Pf=02000008"), "OK");
        assert_eq!(machine.next_instruction_address(), 0x0800_0002);
        // Thumb `movs r0, #0x2a` at 0x08000002 is the upper half of `mov r0, #42`.
        assert_eq!(reply(&mut stub, &mut machine, "P19=1f000000"), "OK");
        assert_eq!(machine.next_instruction_address(), 0x0800_0000);
    }

    #[test]
    fn reads_memory_and_writes_only_work_ram() {
        let mut machine = machine();
        let mut stub = Stub::default();
        assert_eq!(reply(&mut stub, &mut machine, "m8000000,4"), "2a00a0e3");
        assert_eq!(reply(&mut stub, &mut machine, "mffffffff,2"), "E01");
        assert_eq!(reply(&mut stub, &mut machine, "m0,4001"), "E01");
        assert_eq!(reply(&mut stub, &mut machine, "M3000000,3:010203"), "OK");
        assert_eq!(reply(&mut stub, &mut machine, "m3000000,4"), "01020300");
        assert_eq!(reply(&mut stub, &mut machine, "M2000000,1:ff"), "OK");
        assert_eq!(reply(&mut stub, &mut machine, "m2000000,1"), "ff");
        assert_eq!(reply(&mut stub, &mut machine, "M8000000,1:00"), "E02");
        assert_eq!(reply(&mut stub, &mut machine, "M4000000,2:0000"), "E02");
        // All or nothing across the end of IWRAM's region.
        assert_eq!(reply(&mut stub, &mut machine, "M3ffffff,2:5555"), "E02");
        assert_eq!(reply(&mut stub, &mut machine, "m3ffffff,1"), "00");
        assert_eq!(reply(&mut stub, &mut machine, "M3000000,2:00"), "E01");
        assert_eq!(reply(&mut stub, &mut machine, "M3000000,1"), "E01");
    }

    #[test]
    fn patching_prefetched_code_takes_effect_on_the_next_step() {
        // IWRAM code: `mov r0, #1`, `mov r0, #2`; both are prefetched once
        // the first executes from a jump there.
        let mut machine = machine();
        let mut stub = Stub::default();
        assert_eq!(reply(&mut stub, &mut machine, "M3000000,8:0100a0e30200a0e3"), "OK");
        assert_eq!(reply(&mut stub, &mut machine, "Pf=00000003"), "OK");
        step(&mut machine);
        assert_eq!(machine.read_register(0), 1);
        // Replace the prefetched `mov r0, #2` with `mov r0, #3`.
        assert_eq!(reply(&mut stub, &mut machine, "M3000004,1:03"), "OK");
        step(&mut machine);
        assert_eq!(machine.read_register(0), 3);
    }

    #[test]
    fn breakpoints_are_a_set_of_addresses_for_both_kinds() {
        let mut machine = machine();
        let mut stub = Stub::default();
        assert_eq!(reply(&mut stub, &mut machine, "Z0,8000004,4"), "OK");
        assert_eq!(reply(&mut stub, &mut machine, "Z1,8000008,2"), "OK");
        assert_eq!(reply(&mut stub, &mut machine, "Z0,8000004,4"), "OK");
        assert!(stub.breakpoint_at(0x0800_0004) && stub.breakpoint_at(0x0800_0008));
        assert_eq!(reply(&mut stub, &mut machine, "z0,8000004,4"), "OK");
        assert!(!stub.breakpoint_at(0x0800_0004));
        assert_eq!(reply(&mut stub, &mut machine, "z1,8000004,4"), "OK");
        // Watchpoints are unsupported, so gdb falls back to single-stepping.
        assert_eq!(reply(&mut stub, &mut machine, "Z2,3000000,4"), "");
        assert_eq!(reply(&mut stub, &mut machine, "Z0,x,4"), "E01");
    }

    #[test]
    fn resume_packets_leave_execution_to_the_caller() {
        let mut machine = machine();
        let mut stub = Stub::default();
        assert_eq!(stub.handle(&mut machine, b"c"), Command::Resume(Resume::Continue));
        assert_eq!(stub.handle(&mut machine, b"s"), Command::Resume(Resume::Step));
        assert_eq!(stub.handle(&mut machine, b"C05"), Command::Resume(Resume::Continue));
        assert_eq!(stub.handle(&mut machine, b"S05;8000004"), Command::Resume(Resume::Step));
        assert_eq!(machine.next_instruction_address(), 0x0800_0004);
        assert_eq!(stub.handle(&mut machine, b"c8000008"), Command::Resume(Resume::Continue));
        assert_eq!(machine.next_instruction_address(), 0x0800_0008);
        assert_eq!(machine.now(), 0);
    }

    #[test]
    fn stop_replies_carry_signals_and_the_last_one_answers_the_status_query() {
        let mut machine = machine();
        let mut stub = Stub::default();
        assert_eq!(reply(&mut stub, &mut machine, "?"), "S05");
        assert_eq!(stub.stop_reply(Stop::Interrupted), b"S02");
        assert_eq!(reply(&mut stub, &mut machine, "?"), "S02");
        assert_eq!(stub.stop_reply(Stop::Faulted), b"S06");
        assert_eq!(stub.stop_reply(Stop::Trap), b"S05");
    }

    #[test]
    fn serves_features_memory_map_and_thread_queries() {
        let mut machine = machine();
        let mut stub = Stub::default();
        let supported = reply(&mut stub, &mut machine, "qSupported:multiprocess+;swbreak+");
        assert_eq!(
            supported,
            "PacketSize=4000;qXfer:features:read+;qXfer:memory-map:read+;QStartNoAckMode+"
        );
        let whole = reply(&mut stub, &mut machine, "qXfer:features:read:target.xml:0,2000");
        assert_eq!(whole, format!("l{TARGET_XML}"));
        let head = reply(&mut stub, &mut machine, "qXfer:features:read:target.xml:0,10");
        assert_eq!(head, format!("m{}", &TARGET_XML[..16]));
        let tail = format!("qXfer:features:read:target.xml:{:x},100", TARGET_XML.len() - 4);
        assert_eq!(reply(&mut stub, &mut machine, &tail), "lget>");
        let past = format!("qXfer:features:read:target.xml:{:x},10", TARGET_XML.len() + 9);
        assert_eq!(reply(&mut stub, &mut machine, &past), "l");
        assert_eq!(reply(&mut stub, &mut machine, "qXfer:features:read:other.xml:0,10"), "E00");
        let map = reply(&mut stub, &mut machine, "qXfer:memory-map:read::0,2000");
        assert_eq!(map, format!("l{MEMORY_MAP_XML}"));
        assert_eq!(reply(&mut stub, &mut machine, "qAttached"), "1");
        assert_eq!(reply(&mut stub, &mut machine, "qC"), "QC1");
        assert_eq!(reply(&mut stub, &mut machine, "qfThreadInfo"), "m1");
        assert_eq!(reply(&mut stub, &mut machine, "qsThreadInfo"), "l");
        assert_eq!(reply(&mut stub, &mut machine, "Hg0"), "OK");
        assert_eq!(reply(&mut stub, &mut machine, "vMustReplyEmpty"), "");
        assert_eq!(reply(&mut stub, &mut machine, "vCont?"), "");
        assert_eq!(reply(&mut stub, &mut machine, "X3000000,0:"), "");
        assert!(!stub.no_ack());
        assert_eq!(reply(&mut stub, &mut machine, "QStartNoAckMode"), "OK");
        assert!(stub.no_ack());
    }

    #[test]
    fn detach_and_kill_end_the_session() {
        let mut machine = machine();
        let mut stub = Stub::default();
        assert_eq!(stub.handle(&mut machine, b"D"), Command::End(Some(b"OK".to_vec())));
        assert_eq!(stub.handle(&mut machine, b"D;1"), Command::End(Some(b"OK".to_vec())));
        assert_eq!(stub.handle(&mut machine, b"vKill;1"), Command::End(Some(b"OK".to_vec())));
        assert_eq!(stub.handle(&mut machine, b"k"), Command::End(None));
    }
}
