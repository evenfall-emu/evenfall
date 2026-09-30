//! Host-only save-file selection, loading, and atomic persistence.

use crate::cheats::{self, Cheat, Format};
use crate::{
    cartridge::{self, save::Type, tilt},
    rom_image,
    types::{CYCLES_PER_FRAME, Cycle},
};
use std::{
    fs,
    io::{self, Read, Write},
    num::{IntErrorKind, ParseIntError},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_SAVE_FILE_SIZE: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RtcOption {
    #[default]
    Auto,
    Disabled,
    Epoch(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SolarOption {
    #[default]
    Auto,
    Disabled,
    Level(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GyroOption {
    #[default]
    Auto,
    Disabled,
    Sample(u16),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TiltOption {
    #[default]
    Auto,
    Disabled,
    Sample(tilt::Sample),
}

#[derive(Clone, Debug, Default)]
pub struct Options<'a> {
    pub rom_path: &'a str,
    pub bios_path: Option<&'a str>,
    pub save_type_override: Option<Type>,
    pub save_path_override: Option<&'a str>,
    pub state_in: Option<&'a str>,
    pub state_out: Option<&'a str>,
    pub trace_path: Option<&'a str>,
    pub trace_from: usize,
    pub input_movie: Option<&'a str>,
    pub quiet: bool,
    pub frame_hash: bool,
    pub rtc: RtcOption,
    pub solar: SolarOption,
    pub gyro: GyroOption,
    pub rumble_override: Option<bool>,
    pub tilt: TiltOption,
    /// `--cheat` specs in order.
    pub cheats: Vec<&'a str>,
    pub cheat_file: Option<&'a str>,
}

pub struct Prepared {
    pub save_type: Type,
    pub save_path: PathBuf,
    pub save_data: Option<Vec<u8>>,
}

#[derive(Debug)]
pub enum Error {
    InvalidArguments,
    InvalidCharacter,
    Overflow,
    InvalidSaveType,
    InvalidBiosSize,
    StreamTooLong,
    Io(io::Error),
    Image(rom_image::Error),
}
impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
impl From<rom_image::Error> for Error {
    fn from(err: rom_image::Error) -> Self {
        Self::Image(err)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovieEvent {
    pub at: Cycle,
    pub keys: u16,
}
pub struct InputMovie {
    pub events: Vec<MovieEvent>,
}
impl InputMovie {
    pub const MAX_EVENTS: usize = 64;
    pub fn slice(&self) -> &[MovieEvent] {
        &self.events
    }
}

/// Compiles overlapping key ranges into full held-key states at frame edges.
/// A `--cheat` spec: `[FORMAT:]CODE`, where FORMAT is `auto` or a
/// [`Format`](crate::cheats::Format) name; without one the format is detected.
pub fn parse_cheat_spec(spec: &str) -> Result<Cheat, cheats::Error> {
    let (format, code) = match spec.split_once(':') {
        Some(("auto", code)) => (None, code),
        Some((name, code)) if Format::from_name(name).is_some() => (Format::from_name(name), code),
        _ => (None, spec),
    };
    Cheat::parse(code, format)
}

/// A `--cheats` file: cheats separated by blank lines. A cheat's first line
/// may be `!FORMAT`; `#` starts a comment. Errors carry the cheat's number.
pub fn parse_cheat_file(text: &str) -> Result<Vec<Cheat>, (usize, cheats::Error)> {
    let mut cheats = Vec::new();
    let mut paragraph = Vec::new();
    for line in text.lines().chain([""]) {
        let content = line.split('#').next().unwrap_or_default().trim();
        if !content.is_empty() {
            paragraph.push(content);
            continue;
        }
        if line.trim().is_empty() && !paragraph.is_empty() {
            let number = cheats.len() + 1;
            let (format, lines) = match paragraph[0].strip_prefix('!') {
                Some("auto") => (None, &paragraph[1..]),
                Some(name) => (
                    Some(Format::from_name(name.trim()).ok_or((
                        number,
                        cheats::Error { line: None, kind: cheats::ErrorKind::Syntax },
                    ))?),
                    &paragraph[1..],
                ),
                None => (None, &paragraph[..]),
            };
            cheats.push(Cheat::parse(&lines.join("\n"), format).map_err(|error| (number, error))?);
            paragraph.clear();
        }
    }
    Ok(cheats)
}

pub fn parse_input_movie(text: &str) -> Result<InputMovie, Error> {
    const KEYS: [&str; 10] = ["a", "b", "select", "start", "right", "left", "up", "down", "r", "l"];
    let mut edges = Vec::<(u64, usize, i8)>::new();
    for entry in text.split(',') {
        if entry.is_empty() || edges.len() + 2 > InputMovie::MAX_EVENTS {
            return Err(Error::InvalidArguments);
        }
        let (key, range) = entry.split_once(':').ok_or(Error::InvalidArguments)?;
        let (first, last) = range.split_once('-').ok_or(Error::InvalidArguments)?;
        let first = first.parse::<u64>().map_err(integer_error)?;
        let last = last.parse::<u64>().map_err(integer_error)?;
        if first > last || last == u64::MAX {
            return Err(Error::InvalidArguments);
        }
        let key = KEYS.iter().position(|&name| name == key).ok_or(Error::InvalidArguments)?;
        edges.push((first, key, 1));
        edges.push((last + 1, key, -1));
    }
    edges.sort_by_key(|edge| edge.0);
    let mut movie = InputMovie { events: Vec::new() };
    let mut active = [0_u8; 10];
    let mut previous_bits = 0;
    let mut index = 0;
    while index < edges.len() {
        let frame = edges[index].0;
        while index < edges.len() && edges[index].0 == frame {
            let (_, key, delta) = edges[index];
            if delta > 0 {
                active[key] += 1;
            } else {
                debug_assert_ne!(active[key], 0);
                active[key] -= 1;
            }
            index += 1;
        }
        let mut bits = 0_u16;
        for (key, &count) in active.iter().enumerate() {
            if count != 0 {
                bits |= 1 << key;
            }
        }
        if bits == previous_bits {
            continue;
        }
        let at = frame.checked_mul(CYCLES_PER_FRAME).ok_or(Error::InvalidArguments)?;
        movie.events.push(MovieEvent { at, keys: bits });
        previous_bits = bits;
    }
    Ok(movie)
}

pub fn parse_save_type(name: &str) -> Result<Type, Error> {
    match name {
        "auto" => Ok(Type::Auto),
        "none" => Ok(Type::None),
        "sram" => Ok(Type::Sram),
        "flash64" => Ok(Type::Flash64k),
        "flash64-atmel" => Ok(Type::Flash64kAtmel),
        "flash128" => Ok(Type::Flash128k),
        "eeprom512" => Ok(Type::Eeprom512),
        "eeprom8k" => Ok(Type::Eeprom8k),
        _ => Err(Error::InvalidSaveType),
    }
}

fn integer_error(error: ParseIntError) -> Error {
    match error.kind() {
        IntErrorKind::PosOverflow | IntErrorKind::NegOverflow => Error::Overflow,
        _ => Error::InvalidCharacter,
    }
}

pub fn parse_options<'a>(args: &[&'a str]) -> Result<Options<'a>, Error> {
    let mut options = Options::default();
    let mut have_rtc = false;
    let mut have_solar = false;
    let mut have_gyro = false;
    let mut have_tilt = false;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        let value = args.get(index + 1).copied();
        match arg {
            "--save-type" if options.save_type_override.is_none() => {
                options.save_type_override =
                    Some(parse_save_type(value.ok_or(Error::InvalidArguments)?)?)
            }
            "--bios" if options.bios_path.is_none() => {
                options.bios_path = Some(value.ok_or(Error::InvalidArguments)?)
            }
            "--save-file" if options.save_path_override.is_none() => {
                options.save_path_override = Some(value.ok_or(Error::InvalidArguments)?)
            }
            "--state-in" if options.state_in.is_none() => {
                options.state_in = Some(value.ok_or(Error::InvalidArguments)?)
            }
            "--state-out" if options.state_out.is_none() => {
                options.state_out = Some(value.ok_or(Error::InvalidArguments)?)
            }
            "--trace" if options.trace_path.is_none() => {
                options.trace_path = Some(value.ok_or(Error::InvalidArguments)?)
            }
            "--trace-from" => {
                options.trace_from =
                    value.ok_or(Error::InvalidArguments)?.parse().map_err(integer_error)?
            }
            "--cheat" => options.cheats.push(value.ok_or(Error::InvalidArguments)?),
            "--cheats" if options.cheat_file.is_none() => {
                options.cheat_file = Some(value.ok_or(Error::InvalidArguments)?)
            }
            "--input" if options.input_movie.is_none() => {
                options.input_movie = Some(value.ok_or(Error::InvalidArguments)?)
            }
            "--rtc-epoch" if !have_rtc => {
                options.rtc = RtcOption::Epoch(
                    value.ok_or(Error::InvalidArguments)?.parse().map_err(integer_error)?,
                );
                have_rtc = true;
            }
            "--solar-level" if !have_solar => {
                options.solar = SolarOption::Level(
                    value.ok_or(Error::InvalidArguments)?.parse().map_err(integer_error)?,
                );
                have_solar = true;
            }
            "--gyro-sample" if !have_gyro => {
                let sample = parse_u16(value.ok_or(Error::InvalidArguments)?)?;
                if sample > 0xfff {
                    return Err(Error::Overflow);
                }
                options.gyro = GyroOption::Sample(sample);
                have_gyro = true;
            }
            "--tilt" if !have_tilt => {
                let x = parse_u16(value.ok_or(Error::InvalidArguments)?)?;
                let y = parse_u16(args.get(index + 2).copied().ok_or(Error::InvalidArguments)?)?;
                if x > 0xfff || y > 0xfff {
                    return Err(Error::Overflow);
                }
                options.tilt = TiltOption::Sample(tilt::Sample { x, y });
                have_tilt = true;
            }
            "--no-rtc" if !have_rtc => {
                options.rtc = RtcOption::Disabled;
                have_rtc = true;
            }
            "--no-solar" if !have_solar => {
                options.solar = SolarOption::Disabled;
                have_solar = true;
            }
            "--no-gyro" if !have_gyro => {
                options.gyro = GyroOption::Disabled;
                have_gyro = true;
            }
            "--no-tilt" if !have_tilt => {
                options.tilt = TiltOption::Disabled;
                have_tilt = true;
            }
            "--rumble" if options.rumble_override.is_none() => options.rumble_override = Some(true),
            "--no-rumble" if options.rumble_override.is_none() => {
                options.rumble_override = Some(false)
            }
            "--quiet" => options.quiet = true,
            "--frame-hash" => options.frame_hash = true,
            _ if !arg.starts_with("--") && options.rom_path.is_empty() => options.rom_path = arg,
            _ => return Err(Error::InvalidArguments),
        }
        index += if arg == "--tilt" {
            3
        } else if matches!(
            arg,
            "--save-type"
                | "--bios"
                | "--save-file"
                | "--state-in"
                | "--state-out"
                | "--trace"
                | "--trace-from"
                | "--input"
                | "--cheat"
                | "--cheats"
                | "--rtc-epoch"
                | "--solar-level"
                | "--gyro-sample"
        ) {
            2
        } else {
            1
        };
    }
    if options.rom_path.is_empty() {
        return Err(Error::InvalidArguments);
    }
    Ok(options)
}

fn parse_u16(value: &str) -> Result<u16, Error> {
    if let Some(hex) = value.strip_prefix("0x") {
        u16::from_str_radix(hex, 16)
    } else {
        value.parse()
    }
    .map_err(integer_error)
}

pub fn resolve_rtc_epoch(rom: &[u8], option: RtcOption, host_epoch: u64) -> Option<u64> {
    match option {
        RtcOption::Auto => cartridge::detect_features(rom).rtc.then_some(host_epoch),
        RtcOption::Disabled => None,
        RtcOption::Epoch(epoch) => Some(epoch),
    }
}
pub fn resolve_solar_level(rom: &[u8], option: SolarOption) -> Option<u8> {
    match option {
        SolarOption::Auto => cartridge::detect_features(rom).solar_sensor.then_some(156),
        SolarOption::Disabled => None,
        SolarOption::Level(level) => Some(level),
    }
}
pub fn resolve_gyro_sample(rom: &[u8], option: GyroOption) -> Option<u16> {
    match option {
        GyroOption::Auto => cartridge::detect_features(rom).gyro.then_some(0x6c0),
        GyroOption::Disabled => None,
        GyroOption::Sample(sample) => Some(sample),
    }
}
pub fn resolve_rumble(rom: &[u8], override_value: Option<bool>) -> bool {
    override_value.unwrap_or_else(|| cartridge::detect_features(rom).rumble)
}
pub fn resolve_tilt(rom: &[u8], option: TiltOption) -> Option<tilt::Sample> {
    match option {
        TiltOption::Auto => cartridge::detect_features(rom).tilt.then_some(tilt::Sample::default()),
        TiltOption::Disabled => None,
        TiltOption::Sample(sample) => Some(sample),
    }
}
pub fn current_unix_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub fn prepare(rom: &[u8], options: &Options<'_>) -> Result<Prepared, Error> {
    let save_path = options
        .save_path_override
        .map_or_else(|| Path::new(options.rom_path).with_extension("sav"), PathBuf::from);
    if options.save_type_override == Some(Type::None) {
        return Ok(Prepared { save_type: Type::None, save_path, save_data: None });
    }
    let mut save_data = match read_bounded(&save_path, MAX_SAVE_FILE_SIZE) {
        Ok(data) => Some(data),
        Err(Error::Io(err)) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    let save_type =
        rom_image::choose_save_type(rom, options.save_type_override, save_data.as_deref())?;
    if let Some(data) = &mut save_data {
        data.truncate(rom_image::save_prefix(save_type, data)?.len());
    }
    Ok(Prepared { save_type, save_path, save_data })
}

pub fn load_bios_file(options: &Options<'_>) -> Result<Vec<u8>, Error> {
    let Some(path) = options.bios_path else {
        return Ok(Vec::new());
    };
    let bios = read_bounded(Path::new(path), 64 * 1024)?;
    if bios.len() != 16 * 1024 {
        return Err(Error::InvalidBiosSize);
    }
    Ok(bios)
}

pub fn load_rom_file(rom_path: impl AsRef<Path>) -> Result<Vec<u8>, Error> {
    let path = rom_path.as_ref();
    let data = read_bounded(path, rom_image::MAX_ROM_SIZE)?;
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip")) {
        return Ok(data);
    }
    rom_image::extract_single_gba(&data).map_err(Error::Image)
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, Error> {
    let file = fs::File::open(path)?;
    let mut data = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut data)?;
    if data.len() > maximum {
        return Err(Error::StreamTooLong);
    }
    Ok(data)
}

/// Replace a save in its own directory after a full write and file sync.
pub fn write_atomic(path: impl AsRef<Path>, data: &[u8]) -> Result<(), Error> {
    if data.is_empty() {
        return Ok(());
    }
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let path = path.as_ref();
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().ok_or(Error::InvalidArguments)?.to_string_lossy();
    let mut temp;
    let mut file;
    loop {
        temp = dir.join(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(created) => {
                file = created;
                break;
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::Io(err)),
        }
    }
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(Error::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cheat_specs_and_files_name_or_detect_their_formats() {
        let spec = parse_cheat_spec("action-replay:D8BAE4D9 4864DCE5+A86CDBA5 19BA49B3").unwrap();
        assert_eq!(spec.format(), Format::ActionReplay);
        assert_eq!(parse_cheat_spec("02025E90:0F423F00").unwrap().format(), Format::Raw);
        assert_eq!(parse_cheat_spec("raw:02025E90:0F").unwrap().format(), Format::Raw);
        assert_eq!(parse_cheat_spec("auto:82025BC4 E0FF").unwrap().format(), Format::CodeBreaker);
        assert!(parse_cheat_spec("codebreaker:D8BAE4D9 4864DCE5").is_err());

        let file = "# Emerald\n!action-replay\nD8BAE4D9 4864DCE5\nA86CDBA5 19BA49B3\n\n\
                    82025BC4 E0FF # money\n\n   \n02000000:01\n";
        let cheats = parse_cheat_file(file).unwrap();
        let formats: Vec<_> = cheats.iter().map(Cheat::format).collect();
        assert_eq!(formats, [Format::ActionReplay, Format::CodeBreaker, Format::Raw]);
        assert_eq!(parse_cheat_file("02000000:01\n\n!nope\n02000000:01").unwrap_err().0, 2);
        assert_eq!(parse_cheat_file("02000000:01\n\n0202XYZ0 0001").unwrap_err().0, 2);
    }

    fn test_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "evenfall-host-save-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }

    /// Zig: frontend options accept one ROM among host flags and reject ambiguity
    #[test]
    fn frontend_options_accept_one_rom_among_host_flags_and_reject_ambiguity() {
        let parsed = parse_options(&[
            "--bios",
            "replacement.bin",
            "--save-type",
            "flash64-atmel",
            "game.gba",
            "--save-file",
            "custom.sav",
            "--rtc-epoch",
            "981205445",
            "--tilt",
            "0x392",
            "0x3A0",
            "--no-rumble",
        ])
        .unwrap();
        assert_eq!(parsed.rom_path, "game.gba");
        assert_eq!(parsed.bios_path, Some("replacement.bin"));
        assert_eq!(parsed.save_type_override, Some(Type::Flash64kAtmel));
        assert_eq!(parsed.save_path_override, Some("custom.sav"));
        assert_eq!(parsed.rtc, RtcOption::Epoch(981_205_445));
        assert_eq!(parsed.tilt, TiltOption::Sample(tilt::Sample { x: 0x392, y: 0x3a0 }));
        assert_eq!(parsed.rumble_override, Some(false));
        for args in [
            &["game.gba", "--bios"][..],
            &["--bios", "a.bin", "--bios", "b.bin", "game.gba"],
            &["one.gba", "two.gba"],
            &["--no-rtc", "--rtc-epoch", "1", "game.gba"],
        ] {
            assert!(matches!(parse_options(args), Err(Error::InvalidArguments)));
        }
        assert!(matches!(
            parse_options(&["--save-type", "flash", "game.gba"]),
            Err(Error::InvalidSaveType)
        ));
        assert!(matches!(
            parse_options(&["--solar-level", "256", "game.gba"]),
            Err(Error::Overflow)
        ));
    }

    /// Zig: input movies compile frame ranges onto the canonical cycle timeline
    #[test]
    fn input_movies_compile_frame_ranges_onto_the_canonical_cycle_timeline() {
        let movie = parse_input_movie("start:2-3,a:3-4").unwrap();
        assert_eq!(
            movie.slice(),
            [
                MovieEvent { at: 2 * CYCLES_PER_FRAME, keys: 8 },
                MovieEvent { at: 3 * CYCLES_PER_FRAME, keys: 9 },
                MovieEvent { at: 4 * CYCLES_PER_FRAME, keys: 1 },
                MovieEvent { at: 5 * CYCLES_PER_FRAME, keys: 0 }
            ]
        );
        let overlap = parse_input_movie("a:2-4,a:3-5").unwrap();
        assert_eq!(
            overlap.slice(),
            [
                MovieEvent { at: 2 * CYCLES_PER_FRAME, keys: 1 },
                MovieEvent { at: 6 * CYCLES_PER_FRAME, keys: 0 }
            ]
        );
    }

    /// Zig: input movies reject malformed and descending ranges
    #[test]
    fn input_movies_reject_malformed_and_descending_ranges() {
        assert!(matches!(parse_input_movie("start:4-3"), Err(Error::InvalidArguments)));
        assert!(matches!(parse_input_movie("menu:1-2"), Err(Error::InvalidArguments)));
        assert!(matches!(parse_input_movie("a:x-2"), Err(Error::InvalidCharacter)));
    }

    /// Zig: host RTC time reaches only detected or explicitly configured cartridges
    #[test]
    fn host_rtc_time_reaches_only_detected_or_explicitly_configured_cartridges() {
        let mut ruby = [0; 0xb0];
        ruby[0xac..0xb0].copy_from_slice(b"AXVE");
        let mut fire_red = [0; 0xb0];
        fire_red[0xac..0xb0].copy_from_slice(b"BPRE");
        assert_eq!(resolve_rtc_epoch(&ruby, RtcOption::Auto, 1234), Some(1234));
        assert_eq!(resolve_rtc_epoch(&fire_red, RtcOption::Auto, 1234), None);
        assert_eq!(resolve_rtc_epoch(&ruby, RtcOption::Disabled, 1234), None);
        assert_eq!(resolve_rtc_epoch(&fire_red, RtcOption::Epoch(99), 1234), Some(99));
    }

    /// Zig: save preparation detects markers and derives a sibling path
    #[test]
    fn save_preparation_detects_markers_and_derives_a_sibling_path() {
        let dir = test_dir();
        let path = dir.join("game.gba");
        let path_string = path.to_str().unwrap();
        let prepared = prepare(
            b"xxxxFLASH1M_V123 suffix",
            &Options { rom_path: path_string, ..Options::default() },
        )
        .unwrap();
        assert_eq!(prepared.save_type, Type::Flash128k);
        assert_eq!(prepared.save_path, dir.join("game.sav"));
        assert!(prepared.save_data.is_none());
        fs::remove_dir(dir).unwrap();
    }

    /// Zig: existing save size restores unknown ROMs and atomic replacement round-trips
    #[test]
    fn existing_save_size_restores_unknown_roms_and_atomic_replacement_round_trips() {
        let dir = test_dir();
        let path = dir.join("game.sav");
        let mut initial = vec![0xff; 32768];
        initial[0x1234] = 0x5a;
        write_atomic(&path, &initial).unwrap();
        let rom_path = dir.join("game.gba");
        let prepared = prepare(
            b"no marker",
            &Options { rom_path: rom_path.to_str().unwrap(), ..Options::default() },
        )
        .unwrap();
        assert_eq!(prepared.save_type, Type::Sram);
        assert_eq!(prepared.save_data.unwrap()[0x1234], 0x5a);
        initial[0x1234] = 0xa5;
        write_atomic(&path, &initial).unwrap();
        assert_eq!(fs::read(&path).unwrap()[0x1234], 0xa5);
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    /// Zig: native save import strips a trailer before returning cartridge bytes
    #[test]
    fn native_save_import_strips_a_trailer_before_returning_cartridge_bytes() {
        let dir = test_dir();
        let path = dir.join("game.sav");
        let mut save = vec![0xff; 131072 + 16];
        save[0x1234] = 0x35;
        save[0x10042] = 0x7a;
        save[131072..].fill(0x55);
        write_atomic(&path, &save).unwrap();
        let rom_path = dir.join("game.gba");
        let prepared = prepare(
            b"....FLASH1M_V103",
            &Options { rom_path: rom_path.to_str().unwrap(), ..Options::default() },
        )
        .unwrap();
        assert_eq!(prepared.save_type, Type::Flash128k);
        assert_eq!(prepared.save_data.unwrap(), save[..131072]);
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    /// Zig: EEPROM marker starts unresolved unless an override fixes its capacity
    #[test]
    fn eeprom_marker_starts_unresolved_unless_an_override_fixes_its_capacity() {
        let dir = test_dir();
        let rom_path = dir.join("game.gba");
        let opts = Options { rom_path: rom_path.to_str().unwrap(), ..Options::default() };
        let automatic = prepare(b"EEPROM_V123", &opts).unwrap();
        assert_eq!(automatic.save_type, Type::Auto);
        assert!(automatic.save_data.is_none());
        let explicit =
            prepare(b"EEPROM_V123", &Options { save_type_override: Some(Type::Eeprom512), ..opts })
                .unwrap();
        assert_eq!(explicit.save_type, Type::Eeprom512);
        fs::remove_dir(dir).unwrap();
    }

    /// Zig: existing file size must agree with explicit types and exact ROM markers
    #[test]
    fn existing_file_size_must_agree_with_explicit_types_and_exact_rom_markers() {
        let dir = test_dir();
        let path = dir.join("game.sav");
        let rom_path = dir.join("game.gba");
        let opts = Options { rom_path: rom_path.to_str().unwrap(), ..Options::default() };
        write_atomic(&path, &vec![0xff; 512]).unwrap();
        assert_eq!(
            prepare(b"unknown", &Options { save_type_override: Some(Type::Auto), ..opts.clone() })
                .unwrap()
                .save_type,
            Type::Eeprom512
        );
        assert!(matches!(
            prepare(b"unknown", &Options { save_type_override: Some(Type::Sram), ..opts.clone() }),
            Err(Error::Image(rom_image::Error::InvalidSaveFileSize))
        ));
        write_atomic(&path, &vec![0xff; 65536]).unwrap();
        assert_eq!(
            prepare(
                b"unknown",
                &Options { save_type_override: Some(Type::Flash64kAtmel), ..opts.clone() }
            )
            .unwrap()
            .save_type,
            Type::Flash64kAtmel
        );
        assert!(matches!(
            prepare(b"xxxxSRAM_V123", &opts),
            Err(Error::Image(rom_image::Error::SaveTypeConflict))
        ));
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    /// Zig: ROM loading accepts exactly 32 MiB and rejects larger images
    #[test]
    fn rom_loading_accepts_exactly_32_mib_and_rejects_larger_images() {
        let dir = test_dir();
        let exact = dir.join("exact.gba");
        let oversized = dir.join("oversized.gba");
        fs::write(&exact, vec![0; rom_image::MAX_ROM_SIZE]).unwrap();
        assert_eq!(load_rom_file(&exact).unwrap().len(), rom_image::MAX_ROM_SIZE);
        fs::write(&oversized, vec![0; rom_image::MAX_ROM_SIZE + 1]).unwrap();
        assert!(matches!(load_rom_file(&oversized), Err(Error::StreamTooLong)));
        fs::remove_file(exact).unwrap();
        fs::remove_file(oversized).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
