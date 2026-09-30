//! Cheat codes: GameShark v1/v2, Action Replay v3, CodeBreaker and raw
//! address/value patches.
//!
//! A [`Cheat`] is parsed once into operations. The machine runs every active
//! cheat at the first instruction boundary at or after each VBlank start, so
//! applications depend only on emulated time. The active set is a host
//! setting, not machine state: save states never carry it, and loading one
//! keeps the current set. See `docs/design/cheats.md`.

mod crypt;
mod formats;
mod program;
#[cfg(test)]
mod tests;

use crate::bus::{AccessWidth, Bus};
use program::{Op, Width};
use std::fmt;

/// Most cheats one machine holds.
pub const MAX_CHEATS: usize = 256;
/// Most code lines of one cheat.
pub const MAX_LINES: usize = 1024;
/// Most code lines of all a machine's cheats together.
pub const MAX_TOTAL_LINES: usize = 4096;
/// Most bytes one code may write per application: the size of EWRAM.
const MAX_WRITE_BYTES: u64 = 0x4_0000;

/// A code format. `GameShark` is the GameShark Advance and Action Replay
/// v1/v2; `ActionReplay` is the Action Replay v3, also sold as GameShark v3.
/// The decrypted variants take the raw codes their devices encrypt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Raw,
    GameShark,
    GameSharkDecrypted,
    ActionReplay,
    ActionReplayDecrypted,
    CodeBreaker,
}

impl Format {
    pub const ALL: [Self; 6] = [
        Self::Raw,
        Self::GameShark,
        Self::GameSharkDecrypted,
        Self::ActionReplay,
        Self::ActionReplayDecrypted,
        Self::CodeBreaker,
    ];

    /// The name hosts use: `raw`, `gameshark`, `gameshark-decrypted`,
    /// `action-replay`, `action-replay-decrypted` or `codebreaker`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::GameShark => "gameshark",
            Self::GameSharkDecrypted => "gameshark-decrypted",
            Self::ActionReplay => "action-replay",
            Self::ActionReplayDecrypted => "action-replay-decrypted",
            Self::CodeBreaker => "codebreaker",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.name() == name)
    }

    fn shape(self) -> Shape {
        match self {
            Self::Raw => Shape::Raw,
            Self::CodeBreaker => Shape::Short,
            _ => Shape::Long,
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One code line as entered: an address word and a value of `digits` hex
/// digits (8 for GameShark and Action Replay, 4 for CodeBreaker, 2, 4 or 8
/// for raw writes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub address: u32,
    pub value: u32,
    pub digits: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// `AAAAAAAA:VV`
    Raw,
    /// `AAAAAAAA VVVV`
    Short,
    /// `AAAAAAAA VVVVVVVV`
    Long,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// No code lines.
    Empty,
    /// Text that is not a code line of any format.
    Syntax,
    /// Lines of different shapes in one cheat.
    MixedLines,
    /// Lines of a shape the chosen format does not use.
    WrongLineShape,
    TooManyLines,
    TooManyCheats,
    /// No format decodes the lines to a plausible code.
    UndetectedFormat,
    /// Several formats decode the lines to plausible codes.
    AmbiguousFormat(Vec<Format>),
    /// An unknown code type or malformed fields.
    InvalidCode,
    /// A multi-line code without its parameter lines.
    MissingParameters,
    /// ROM patches are not supported: the Game Pak is shared, immutable media.
    RomPatch,
    /// GameShark v1/v2 `DEADFACE` reseeding is not supported.
    GameSharkReseed,
    /// A code that writes more than 256 KiB per application.
    TooLarge,
}

/// A parse error, with the one-based code line it concerns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub line: Option<usize>,
    pub kind: ErrorKind,
}

impl Error {
    const fn new(kind: ErrorKind) -> Self {
        Self { line: None, kind }
    }

    const fn at(line: usize, kind: ErrorKind) -> Self {
        Self { line: Some(line), kind }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line {
            write!(f, "line {line}: ")?;
        }
        match &self.kind {
            ErrorKind::Empty => f.write_str("no code lines"),
            ErrorKind::Syntax => f.write_str("not a code line"),
            ErrorKind::MixedLines => f.write_str("lines of different formats"),
            ErrorKind::WrongLineShape => f.write_str("line shape does not match the format"),
            ErrorKind::TooManyLines => f.write_str("too many code lines"),
            ErrorKind::TooManyCheats => f.write_str("too many cheats"),
            ErrorKind::UndetectedFormat => f.write_str("no format decodes this code; name one"),
            ErrorKind::AmbiguousFormat(formats) => {
                f.write_str("several formats decode this code; name one of")?;
                for format in formats {
                    write!(f, " {format}")?;
                }
                Ok(())
            }
            ErrorKind::InvalidCode => f.write_str("invalid code"),
            ErrorKind::MissingParameters => f.write_str("missing parameter lines"),
            ErrorKind::RomPatch => f.write_str("ROM patch codes are not supported"),
            ErrorKind::GameSharkReseed => {
                f.write_str("GameShark v1/v2 DEADFACE reseeding is not supported")
            }
            ErrorKind::TooLarge => f.write_str("code writes more than 256 KiB"),
        }
    }
}

impl std::error::Error for Error {}

/// A parsed cheat: its format, its code lines as entered and the operations
/// they decode to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cheat {
    format: Format,
    lines: Vec<Line>,
    ops: Vec<Op>,
}

impl Cheat {
    /// Parses a cheat's code text. `None` detects the format: raw lines have
    /// a colon, 8+4-digit lines are CodeBreaker, and 8+8-digit lines take the
    /// only encrypted or decrypted GameShark or Action Replay reading whose
    /// addresses are plausible.
    ///
    /// Lines may be separated by newlines, `+`, `,` or `;`; the halves of a
    /// line by spaces or nothing. `#` starts a comment.
    pub fn parse(text: &str, format: Option<Format>) -> Result<Self, Error> {
        let (shape, lines) = tokenize(text)?;
        match format {
            Some(format) if format.shape() != shape => Err(Error::new(ErrorKind::WrongLineShape)),
            Some(format) => Self::from_lines(format, lines),
            None => match shape {
                Shape::Raw => Self::from_lines(Format::Raw, lines),
                Shape::Short => Self::from_lines(Format::CodeBreaker, lines),
                Shape::Long => {
                    let mut found: Vec<Self> = [
                        Format::ActionReplay,
                        Format::GameShark,
                        Format::ActionReplayDecrypted,
                        Format::GameSharkDecrypted,
                    ]
                    .into_iter()
                    .filter_map(|format| Self::from_lines(format, lines.clone()).ok())
                    .filter(|cheat| program::plausible(&cheat.ops))
                    .collect();
                    match found.len() {
                        0 => Err(Error::new(ErrorKind::UndetectedFormat)),
                        1 => Ok(found.remove(0)),
                        _ => Err(Error::new(ErrorKind::AmbiguousFormat(
                            found.iter().map(|cheat| cheat.format).collect(),
                        ))),
                    }
                }
            },
        }
    }

    /// Decodes code lines of a known format.
    pub fn from_lines(format: Format, lines: Vec<Line>) -> Result<Self, Error> {
        if lines.is_empty() {
            return Err(Error::new(ErrorKind::Empty));
        }
        if lines.len() > MAX_LINES {
            return Err(Error::new(ErrorKind::TooManyLines));
        }
        let valid_digits = |line: &Line| match format.shape() {
            Shape::Raw => matches!(line.digits, 2 | 4 | 8),
            Shape::Short => line.digits == 4,
            Shape::Long => line.digits == 8,
        };
        if let Some(index) = lines.iter().position(|line| !valid_digits(line)) {
            return Err(Error::at(index + 1, ErrorKind::WrongLineShape));
        }
        if let Some(index) = lines
            .iter()
            .position(|line| line.digits < 8 && line.value >> (4 * u32::from(line.digits)) != 0)
        {
            return Err(Error::at(index + 1, ErrorKind::Syntax));
        }
        let ops = match format {
            Format::Raw => formats::raw(&lines),
            Format::GameShark => formats::gameshark(&lines, true)?,
            Format::GameSharkDecrypted => formats::gameshark(&lines, false)?,
            Format::ActionReplay => formats::action_replay(&lines, true)?,
            Format::ActionReplayDecrypted => formats::action_replay(&lines, false)?,
            Format::CodeBreaker => formats::codebreaker(&lines)?,
        };
        if ops.iter().any(|op| op.written_bytes() > MAX_WRITE_BYTES) {
            return Err(Error::new(ErrorKind::TooLarge));
        }
        Ok(Self { format, lines, ops })
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Canonical text: one uppercase line per code line, as its format
    /// writes it. Parsing it with the same format gives an equal cheat.
    pub fn code(&self) -> String {
        let mut text = String::new();
        for (index, line) in self.lines.iter().enumerate() {
            if index != 0 {
                text.push('\n');
            }
            let separator = if self.format == Format::Raw { ':' } else { ' ' };
            text.push_str(&format!(
                "{:08X}{separator}{:0width$X}",
                line.address,
                line.value,
                width = usize::from(line.digits)
            ));
        }
        text
    }

    pub(crate) fn run(&self, memory: &mut impl program::Memory) {
        program::run(&self.ops, memory);
    }
}

fn tokenize(text: &str) -> Result<(Shape, Vec<Line>), Error> {
    let tokens: Vec<&str> = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .flat_map(|line| {
            line.split(|c: char| c.is_whitespace() || matches!(c, '+' | ',' | ';'))
                .filter(|token| !token.is_empty())
        })
        .collect();
    if tokens.is_empty() {
        return Err(Error::new(ErrorKind::Empty));
    }
    let hex = |token: &str, line: usize| {
        if token.is_empty() || token.len() > 8 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::at(line, ErrorKind::Syntax));
        }
        Ok(u32::from_str_radix(token, 16).expect("validated hex digits"))
    };
    let mut lines = Vec::new();
    if tokens.iter().any(|token| token.contains(':')) {
        for (index, token) in tokens.iter().enumerate() {
            let (address, value) =
                token.split_once(':').ok_or(Error::at(index + 1, ErrorKind::MixedLines))?;
            lines.push(Line {
                address: hex(address, index + 1)?,
                value: hex(value, index + 1)?,
                digits: value.len() as u8,
            });
        }
        return Ok((Shape::Raw, lines));
    }
    let mut shape = None;
    let mut index = 0;
    while index < tokens.len() {
        let line = lines.len() + 1;
        let token = tokens[index];
        let (address, value, next) = match token.len() {
            16 | 12 => (&token[..8], &token[8..], index + 1),
            8 => {
                let value = tokens.get(index + 1).ok_or(Error::at(line, ErrorKind::Syntax))?;
                (token, *value, index + 2)
            }
            _ => return Err(Error::at(line, ErrorKind::Syntax)),
        };
        let line_shape = match value.len() {
            4 => Shape::Short,
            8 => Shape::Long,
            _ => return Err(Error::at(line, ErrorKind::Syntax)),
        };
        if *shape.get_or_insert(line_shape) != line_shape {
            return Err(Error::at(line, ErrorKind::MixedLines));
        }
        lines.push(Line {
            address: hex(address, line)?,
            value: hex(value, line)?,
            digits: value.len() as u8,
        });
        index = next;
    }
    Ok((shape.expect("at least one line"), lines))
}

/// Checks a machine's cheat limits.
pub fn check_set(cheats: &[Cheat]) -> Result<(), Error> {
    if cheats.len() > MAX_CHEATS {
        return Err(Error::new(ErrorKind::TooManyCheats));
    }
    if cheats.iter().map(|cheat| cheat.lines.len()).sum::<usize>() > MAX_TOTAL_LINES {
        return Err(Error::new(ErrorKind::TooManyLines));
    }
    Ok(())
}

/// Dots in one video frame, and before the first VBlank line.
const FRAME_DOTS: u64 = 228 * 1232;
const VBLANK_DOTS: u64 = 160 * 1232;

/// The first VBlank start strictly after a video position.
fn next_vblank_after(progress: u64) -> u64 {
    let start = progress - progress % FRAME_DOTS + VBLANK_DOTS;
    if start > progress { start } else { start + FRAME_DOTS }
}

/// A machine's cheat device: the active cheats and the video position of the
/// next VBlank start they apply at. At every instruction boundary a host can
/// observe, that position is the first VBlank start after the current one,
/// so a device moved onto another machine state re-arms from its position.
#[derive(Clone, Debug)]
pub(crate) struct Device {
    cheats: Vec<Cheat>,
    next_vblank: u64,
}

impl Device {
    pub(crate) fn new(cheats: Vec<Cheat>, bus: &Bus) -> Self {
        Self { cheats, next_vblank: next_vblank_after(bus.video_progress()) }
    }

    /// Re-arms for a machine whose video position changed without running,
    /// such as one that loaded a state.
    pub(crate) fn rearm(&mut self, bus: &Bus) {
        self.next_vblank = next_vblank_after(bus.video_progress());
    }

    pub(crate) fn cheats(&self) -> &[Cheat] {
        &self.cheats
    }

    /// Master cycles until the next application while the video runs; the
    /// video never runs faster than the master clock, so it is never late.
    pub(crate) fn cycles_until_due(&self, bus: &Bus) -> u64 {
        self.next_vblank.saturating_sub(bus.video_progress())
    }

    /// Runs the cheats if the video has reached the next VBlank start since
    /// the last call. A video position behind the last application (a reset
    /// video counter) re-arms without running.
    pub(crate) fn poll(&mut self, bus: &mut Bus) {
        let progress = bus.video_progress();
        if progress >= self.next_vblank {
            for cheat in &self.cheats {
                cheat.run(bus);
            }
            self.next_vblank = next_vblank_after(progress);
        } else if self.next_vblank - progress > FRAME_DOTS {
            self.next_vblank = next_vblank_after(progress);
        }
    }
}

impl program::Memory for Bus {
    fn read(&self, address: u32, width: Width) -> u32 {
        let address = address & !(width.bytes() - 1);
        (0..width.bytes()).fold(0, |value, offset| {
            value | u32::from(self.inspect_byte(address.wrapping_add(offset))) << (8 * offset)
        })
    }

    fn write(&mut self, address: u32, width: Width, value: u32) {
        let width = match width {
            Width::Byte => AccessWidth::Byte,
            Width::Half => AccessWidth::Half,
            Width::Word => AccessWidth::Word,
        };
        self.cheat_write(address, width, value);
    }
}
