//! Decoders from each device's code lines to operations. Code types and
//! fields follow GBATEK's "GBA Cheat Codes" pages; see `docs/design/cheats.md`
//! for the choices where sources are silent or disagree.

use super::crypt::{CodeBreakerCipher, Tea};
use super::program::{Cond, Op, Skip, Update, Width};
use super::{Error, ErrorKind, Line};

/// The value word of an enable code that identifies the game.
const ID_CODE: u32 = 0x001d_c0de;
const DEADFACE: u32 = 0xdead_face;

/// Code lines as `(line number, address, value)`, decrypted when a cipher is
/// active. Line numbers count from one.
struct Pairs<'a> {
    lines: &'a [Line],
    next: usize,
    cipher: Option<Tea>,
}

impl<'a> Pairs<'a> {
    fn new(lines: &'a [Line], cipher: Option<Tea>) -> Self {
        Self { lines, next: 0, cipher }
    }

    fn next(&mut self) -> Option<(usize, u32, u32)> {
        let line = self.lines.get(self.next)?;
        self.next += 1;
        let (address, value) = match &self.cipher {
            Some(cipher) => cipher.decrypt(line.address, line.value),
            None => (line.address, line.value),
        };
        Some((self.next, address, value))
    }

    /// The parameter line of a multi-line code starting at `line`.
    fn parameter(&mut self, line: usize) -> Result<(u32, u32), Error> {
        self.next().map(|(_, a, v)| (a, v)).ok_or(Error::at(line, ErrorKind::MissingParameters))
    }
}

fn narrow(value: u32, width: Width, line: usize) -> Result<u32, Error> {
    if u64::from(value) >> (8 * width.bytes()) == 0 {
        Ok(value)
    } else {
        Err(Error::at(line, ErrorKind::InvalidCode))
    }
}

/// GameShark v1/v2 (GameShark Advance, Action Replay v1/v2).
pub(super) fn gameshark(lines: &[Line], encrypted: bool) -> Result<Vec<Op>, Error> {
    let mut pairs = Pairs::new(lines, encrypted.then_some(Tea::GAMESHARK));
    let mut ops = Vec::new();
    while let Some((line, a, v)) = pairs.next() {
        let invalid = Error::at(line, ErrorKind::InvalidCode);
        let address = a & 0x0fff_ffff;
        let op = if v == ID_CODE {
            Op::GameId(a)
        } else if a == DEADFACE {
            // The v1/v2 reseeding tables are not published outside emulators.
            if encrypted {
                return Err(Error::at(line, ErrorKind::GameSharkReseed));
            }
            Op::Nop
        } else {
            match a >> 28 {
                0x0 => Op::write(address, Width::Byte, narrow(v, Width::Byte, line)?),
                0x1 => Op::write(address, Width::Half, narrow(v, Width::Half, line)?),
                0x2 => Op::write(address, Width::Word, v),
                // `3000cccc xxxxxxxx`: the value, then cccc-1 listed words,
                // two per line. The device also writes the value to the
                // address equal to itself (EnHacklopedia; the Hacking Text).
                0x3 if a & 0x0fff_0000 == 0 && a & 0xffff != 0 => {
                    let count = (a & 0xffff) as usize;
                    let mut addresses = Vec::with_capacity(count);
                    addresses.push(v);
                    while addresses.len() < count {
                        let (first, second) = pairs.parameter(line)?;
                        addresses.push(first);
                        if addresses.len() < count {
                            addresses.push(second);
                        }
                    }
                    Op::WriteList { addresses: addresses.into(), value: v }
                }
                0x6 => return Err(Error::at(line, ErrorKind::RomPatch)),
                // GS-button writes and slowdown: the button is never pressed.
                0x8 if a == 0x80f0_0000 || matches!((a >> 20) & 0xf, 1 | 2) => Op::DeviceOnly,
                0xd => Op::If {
                    address,
                    width: Width::Half,
                    cond: Cond::Equal,
                    value: narrow(v, Width::Half, line)?,
                    skip: Skip::Next(1),
                },
                0xe if a & 0x0f00_0000 == 0 && v >> 28 == 0 => Op::If {
                    address: v,
                    width: Width::Half,
                    cond: Cond::Equal,
                    value: a & 0xffff,
                    skip: Skip::Next((a >> 16) as u8),
                },
                0xf => Op::Hook(address),
                _ => return Err(invalid),
            }
        };
        ops.push(op);
    }
    Ok(ops)
}

/// The `a0aaaaa` address of an Action Replay v3 code: GBATEK's address mask
/// 003FFFFF, whose bits 20-21 select the region.
fn action_replay_address(word: u32) -> u32 {
    (((word >> 20) & 3) << 24) | (word & 0x000f_ffff)
}

const WIDTHS: [Width; 3] = [Width::Byte, Width::Half, Width::Word];

/// Action Replay v3 (and GameShark v3). The address word is `ttaaaaaa`:
/// bits 30-31 sub-type, 27-29 type (zero for writes, else a condition),
/// 25-26 size, 24 an extra bit.
pub(super) fn action_replay(lines: &[Line], encrypted: bool) -> Result<Vec<Op>, Error> {
    let mut pairs = Pairs::new(lines, encrypted.then_some(Tea::ACTION_REPLAY));
    let mut ops = Vec::new();
    while let Some((line, a, v)) = pairs.next() {
        let invalid = Error::at(line, ErrorKind::InvalidCode);
        let op = if v == ID_CODE {
            Op::GameId(a)
        } else if a == DEADFACE {
            let key = narrow(v, Width::Half, line)? as u16;
            if encrypted {
                pairs.cipher = Some(Tea::action_replay_reseed(key));
            }
            Op::Nop
        } else if a == 0 {
            match v >> 24 {
                _ if v == 0 => Op::Stop,
                0x08 if v & 0xffff_00ff == 0x0800_0000 => Op::DeviceOnly,
                // AR-button writes: the button is never pressed.
                0x10 | 0x12 | 0x14 => {
                    pairs.parameter(line)?;
                    Op::DeviceOnly
                }
                0x18 | 0x1a | 0x1c | 0x1e => return Err(Error::at(line, ErrorKind::RomPatch)),
                0x40 if v == 0x4000_0000 => Op::EndIf,
                0x60 if v == 0x6000_0000 => Op::Else,
                0x80 | 0x82 | 0x84 => {
                    let width = WIDTHS[((v >> 25) & 3) as usize];
                    let (value, steps) = pairs.parameter(line)?;
                    Op::Slide {
                        address: action_replay_address(v),
                        width,
                        value: narrow(value, width, line)?,
                        count: (steps >> 16) & 0xff,
                        address_step: (steps & 0xffff) * width.bytes(),
                        value_step: steps >> 24,
                    }
                }
                _ => return Err(invalid),
            }
        } else {
            let sub = a >> 30;
            let kind = (a >> 27) & 7;
            let size = ((a >> 25) & 3) as usize;
            let extra = (a >> 24) & 1;
            let address = action_replay_address(a);
            if kind == 0 {
                match (sub, size) {
                    (0, 0) => Op::fill(address, Width::Byte, v, (v >> 8) + 1),
                    (0, 1) => Op::fill(address, Width::Half, v, (v >> 16) + 1),
                    (0, 2) => Op::write(address, Width::Word, v),
                    (1, 0) => Op::Pointer {
                        pointer: address,
                        offset: v >> 8,
                        width: Width::Byte,
                        value: v & 0xff,
                    },
                    (1, 1) => Op::Pointer {
                        pointer: address,
                        offset: (v >> 16) * 2,
                        width: Width::Half,
                        value: v & 0xffff,
                    },
                    (1, 2) => {
                        Op::Pointer { pointer: address, offset: 0, width: Width::Word, value: v }
                    }
                    (2, 0..=2) => Op::Update {
                        address,
                        width: WIDTHS[size],
                        update: Update::Add,
                        value: narrow(v, WIDTHS[size], line)?,
                    },
                    (3, 2) => Op::Hook(0x0800_0000 | (a & 0x01ff_ffff)),
                    (3, 3) => {
                        let width = if extra == 0 { Width::Half } else { Width::Word };
                        Op::write(0x0400_0000 | (a & 0x00ff_ffff), width, narrow(v, width, line)?)
                    }
                    _ => return Err(invalid),
                }
            } else {
                const CONDS: [Cond; 8] = [
                    Cond::Never,
                    Cond::Equal,
                    Cond::NotEqual,
                    Cond::SignedLess,
                    Cond::SignedGreater,
                    Cond::UnsignedLess,
                    Cond::UnsignedGreater,
                    Cond::AnySet,
                ];
                const SKIPS: [Skip; 4] = [Skip::Next(1), Skip::Next(2), Skip::Block, Skip::Rest];
                // Size 3 is GBATEK's "always false".
                let (width, cond) = match size {
                    3 => (Width::Word, Cond::Never),
                    _ => (WIDTHS[size], CONDS[kind as usize]),
                };
                Op::If { address, width, cond, value: v, skip: SKIPS[sub as usize] }
            }
        };
        ops.push(op);
    }
    Ok(ops)
}

/// CodeBreaker lines as `(line number, address, value)`, decrypted once a
/// `9xxxxxxx xxxx` code has selected a cipher.
struct CodeBreakerLines<'a> {
    lines: &'a [Line],
    next: usize,
    cipher: Option<CodeBreakerCipher>,
}

impl CodeBreakerLines<'_> {
    fn next(&mut self) -> Option<(usize, u32, u16)> {
        let line = self.lines.get(self.next)?;
        self.next += 1;
        let (address, value) = match &self.cipher {
            Some(cipher) => cipher.decrypt(line.address, line.value as u16),
            None => (line.address, line.value as u16),
        };
        Some((self.next, address, value))
    }

    fn parameter(&mut self, line: usize) -> Result<(u32, u16), Error> {
        self.next().map(|(_, a, v)| (a, v)).ok_or(Error::at(line, ErrorKind::MissingParameters))
    }
}

/// CodeBreaker (and GameShark SP, Xploder). A `9xxxxxxx xxxx` code encrypts
/// every later line.
pub(super) fn codebreaker(lines: &[Line]) -> Result<Vec<Op>, Error> {
    let mut source = CodeBreakerLines { lines, next: 0, cipher: None };
    let mut ops = Vec::new();
    while let Some((line, a, v)) = source.next() {
        let address = a & 0x0fff_ffff;
        let value = u32::from(v);
        let condition =
            |cond| Op::If { address, width: Width::Half, cond, value, skip: Skip::Next(1) };
        let update = |update| Op::Update { address, width: Width::Half, update, value };
        let op = match a >> 28 {
            0x0 => Op::Nop,
            0x1 => Op::Hook(0x0800_0000 | (a & 0x01ff_ffff)),
            0x2 => update(Update::Or),
            0x3 => Op::write(address, Width::Byte, value),
            // `iiiicccc ssss`: value step, count, address step.
            0x4 => {
                let (counts, address_step) = source.parameter(line)?;
                Op::Slide {
                    address,
                    width: Width::Half,
                    value,
                    count: counts & 0xffff,
                    address_step: address_step.into(),
                    value_step: counts >> 16,
                }
            }
            // `cccc` bytes follow, six per line in digit order.
            0x5 => {
                let count = usize::from(v);
                let mut bytes = Vec::with_capacity(count + 5);
                while bytes.len() < count {
                    let (word, half) = source.parameter(line)?;
                    bytes.extend_from_slice(&word.to_be_bytes());
                    bytes.extend_from_slice(&half.to_be_bytes());
                }
                bytes.truncate(count);
                Op::WriteBytes { address, bytes: bytes.into() }
            }
            0x6 => update(Update::And),
            0x7 => condition(Cond::Equal),
            0x8 => Op::write(address, Width::Half, value),
            0x9 => {
                source.cipher = Some(CodeBreakerCipher::new(a, v));
                Op::Nop
            }
            0xa => condition(Cond::NotEqual),
            0xb => condition(Cond::SignedGreater),
            0xc => condition(Cond::SignedLess),
            0xd if a == 0xd000_0020 => Op::If {
                address: 0x0400_0130,
                width: Width::Half,
                cond: Cond::NoneSet,
                value,
                skip: Skip::Next(1),
            },
            0xe => update(Update::Add),
            0xf => condition(Cond::AnySet),
            _ => return Err(Error::at(line, ErrorKind::InvalidCode)),
        };
        ops.push(op);
    }
    Ok(ops)
}

/// Raw `AAAAAAAA:VV`, `:VVVV` or `:VVVVVVVV` writes.
pub(super) fn raw(lines: &[Line]) -> Vec<Op> {
    lines
        .iter()
        .map(|line| {
            let width = match line.digits {
                2 => Width::Byte,
                4 => Width::Half,
                _ => Width::Word,
            };
            Op::write(line.address, width, line.value)
        })
        .collect()
}
