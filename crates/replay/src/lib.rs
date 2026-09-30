//! Deterministic scenario execution and byte-exact differential observations.
//! Media configuration is data, not an emulator dependency. The caller builds
//! the system and supplies the CPU inspector; [`evenfall_host::System`] deliberately
//! does not prescribe a CPU architecture.

use evenfall_host::{PixelFormat, StereoSample, System, Timed};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;
const MAGIC: &[u8; 8] = b"EFPAR01\0";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootMode {
    Hle,
    SkipBios,
    ExternalBios,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveType {
    Auto,
    None,
    Sram,
    Flash64k,
    Flash64kAtmel,
    Flash128k,
    Eeprom512,
    Eeprom8k,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub boot: BootMode,
    pub save_type: SaveType,
    #[serde(default)]
    pub rtc_epoch: Option<u64>,
    #[serde(default)]
    pub solar_level: Option<u8>,
    #[serde(default)]
    pub gyro_sample: Option<u16>,
    #[serde(default)]
    pub tilt: Option<[u16; 2]>,
    #[serde(default)]
    pub rumble: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    pub rom: String,
    #[serde(default)]
    pub bios: Option<String>,
    #[serde(default)]
    pub save: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    #[serde(rename = "cable_2p")]
    Cable2p,
    CableMulti,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Peer {
    pub assets: Assets,
    pub config: Config,
    #[serde(default)]
    pub terminal_pc: Option<u32>,
    #[serde(default)]
    pub terminal_r12: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub connection: Connection,
    /// Additional machines in cable order; the top-level machine is index zero.
    pub peers: Vec<Peer>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputChange {
    Keys(u16),
    SolarLevel(u8),
    GyroSample(u16),
    Tilt([u16; 2]),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input<I> {
    #[serde(with = "decimal")]
    pub at: u64,
    pub change: I,
}

/// External host command (not a cartridge input). Applied at the first
/// instruction boundary at or after `at`, independently of host slice size.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostAction {
    #[serde(with = "decimal")]
    pub at: u64,
    pub command: HostCommand,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCommand {
    JoybusWrite(u32),
    JoybusRead(bool),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde(bound(deserialize = "I: Deserialize<'de>"))]
pub struct Scenario<I> {
    pub name: String,
    pub assets: Assets,
    pub config: Config,
    #[serde(default)]
    pub link: Option<Link>,
    #[serde(default)]
    pub inputs: Vec<Input<I>>,
    #[serde(default)]
    pub host_actions: Vec<HostAction>,
    #[serde(with = "decimal")]
    pub end_cycle: u64,
    #[serde(default, with = "decimal_vec")]
    pub checkpoints: Vec<u64>,
    /// Nonzero host slice lengths. Run once per length and compare the traces.
    #[serde(with = "decimal_vec")]
    pub slice_cycles: Vec<u64>,
    /// Optional known terminal self-loop, checked at the final checkpoint.
    #[serde(default)]
    pub terminal_pc: Option<u32>,
    /// Optional probe report value in r12 at the final checkpoint.
    #[serde(default)]
    pub terminal_r12: Option<u32>,
}

mod decimal {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

mod decimal_vec {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(value: &[u64], serializer: S) -> Result<S::Ok, S::Error> {
        value.iter().map(ToString::to_string).collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u64>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .map(|v| v.parse().map_err(serde::de::Error::custom))
            .collect()
    }
}

impl<I> Scenario<I> {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(link) = &self.link {
            if !(1..=3).contains(&link.peers.len())
                || (matches!(link.connection, Connection::Cable2p) && link.peers.len() != 1)
            {
                return Err("link requires two to four machines (cable_2p exactly two)");
            }
            if !self.inputs.is_empty() || !self.host_actions.is_empty() {
                return Err("linked scenarios cannot use timed inputs or host actions");
            }
        }
        if self.slice_cycles.is_empty() || self.slice_cycles.contains(&0) {
            return Err("slice lengths must be nonzero");
        }
        if self.end_cycle == 0
            || self.checkpoints.len() > 10_000
            || self.checkpoints.first().copied() == Some(0)
            || self.checkpoints.last().copied() != Some(self.end_cycle)
            || self.checkpoints.windows(2).any(|p| p[0] >= p[1])
        {
            return Err("checkpoints must increase strictly and end at endCycle");
        }
        if self.inputs.iter().any(|i| i.at > self.end_cycle)
            || self.inputs.windows(2).any(|p| p[0].at > p[1].at)
        {
            return Err("inputs must be ordered and within endCycle");
        }
        if self.host_actions.iter().any(|a| a.at > self.end_cycle)
            || self.host_actions.windows(2).any(|p| p[0].at >= p[1].at)
            || self.host_actions.iter().any(|a| matches!(a.command, HostCommand::JoybusRead(false)))
        {
            return Err("host actions must be valid, ordered and within endCycle");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpu {
    pub registers: [u32; 16],
    pub pc: u32,
    pub cpsr: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub requested_cycle: u64,
    pub cycle: u64,
    pub frames: u64,
    pub cpu: Cpu,
    pub audio_rate: u32,
    /// BGR555, little-endian u16, row-major, 240×160.
    pub frame: Vec<u8>,
    /// Signed little-endian i16 L/R pairs, native DAC rate, since last checkpoint.
    pub audio: Vec<u8>,
    pub save: Vec<u8>,
    pub state: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trace(pub Vec<Observation>);

/// Binary `EFPAR01\0`: u32 checkpoint count, then for each checkpoint:
/// requested cycle, actual cycle, frames (u64 LE); 16 registers, PC, CPSR,
/// audio rate (u32 LE); four blobs (frame, audio, save, state), each as
/// u32 length, 32-byte BLAKE3 digest of raw bytes, and exactly length bytes.
/// No ROM/BIOS is embedded. Files with truncated, trailing, or bad-hash data
/// are rejected. State bytes include the system's own envelope and schema
/// digest.
impl Trace {
    pub fn write(&self, mut out: impl Write) -> io::Result<()> {
        out.write_all(MAGIC)?;
        out.write_all(&size(self.0.len())?.to_le_bytes())?;
        for o in &self.0 {
            for n in [o.requested_cycle, o.cycle, o.frames] {
                out.write_all(&n.to_le_bytes())?;
            }
            for n in o.cpu.registers.into_iter().chain([o.cpu.pc, o.cpu.cpsr, o.audio_rate]) {
                out.write_all(&n.to_le_bytes())?;
            }
            for blob in [&o.frame, &o.audio, &o.save, &o.state] {
                out.write_all(&size(blob.len())?.to_le_bytes())?;
                out.write_all(blake3::hash(blob).as_bytes())?;
                out.write_all(blob)?;
            }
        }
        Ok(())
    }

    pub fn read(mut input: impl Read) -> io::Result<Self> {
        let mut magic = [0; 8];
        input.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return invalid("trace magic/version");
        }
        let count = read_u32(&mut input)?;
        if count > 10_000 {
            return invalid("too many checkpoints");
        }
        let mut entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let requested_cycle = read_u64(&mut input)?;
            let cycle = read_u64(&mut input)?;
            let frames = read_u64(&mut input)?;
            let mut registers = [0; 16];
            for r in &mut registers {
                *r = read_u32(&mut input)?;
            }
            let cpu = Cpu { registers, pc: read_u32(&mut input)?, cpsr: read_u32(&mut input)? };
            let audio_rate = read_u32(&mut input)?;
            let mut blob = |limit: u32| -> io::Result<Vec<u8>> {
                let len = read_u32(&mut input)?;
                if len > limit {
                    return invalid("oversized trace blob");
                }
                let mut digest = [0; 32];
                input.read_exact(&mut digest)?;
                let mut bytes = vec![0; len as usize];
                input.read_exact(&mut bytes)?;
                if blake3::hash(&bytes).as_bytes() != &digest {
                    return invalid("trace blob hash");
                }
                Ok(bytes)
            };
            let frame = blob((WIDTH * HEIGHT * 2) as u32)?;
            if frame.len() != WIDTH * HEIGHT * 2 {
                return invalid("frame size");
            }
            let audio = blob(256 * 1024 * 1024)?;
            if audio.len() % 4 != 0 {
                return invalid("audio alignment");
            }
            let save = blob(128 * 1024)?;
            let state = blob(2 * 1024 * 1024)?;
            entries.push(Observation {
                requested_cycle,
                cycle,
                frames,
                cpu,
                audio_rate,
                frame,
                audio,
                save,
                state,
            });
        }
        let mut trailing = [0];
        if input.read(&mut trailing)? != 0 {
            return invalid("trailing data");
        }
        Ok(Self(entries))
    }
}

fn invalid<T>(reason: &'static str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, reason))
}
fn size(len: usize) -> io::Result<u32> {
    u32::try_from(len).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "blob too large"))
}
fn read_u32(input: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_u64(input: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Caller supplies a fresh, configured machine and architecture-specific CPU
/// inspector. `input_deadline` remains the checkpoint across *all* slices,
/// including an instruction crossing a yield point. Drain on every slice so
/// a bounded host audio queue cannot truncate earlier samples.
pub fn run<S, I>(
    system: &mut S,
    scenario: &Scenario<I>,
    slice: u64,
    inspect_cpu: impl Fn(&S) -> Cpu,
) -> Result<Trace, String>
where
    S: System<Pixel = u16>,
    I: Copy,
    S::Input: From<Timed<I>>,
{
    run_with_host_actions(system, scenario, slice, inspect_cpu, |_, _| {
        Err("scenario requires a host action handler".into())
    })
}

/// As [`run`], with a system-specific handler for external host commands
/// such as a JoyBus master. The handler schedules the completion relative to
/// the system's *actual* cycle at the crossed instruction boundary.
pub fn run_with_host_actions<S, I>(
    system: &mut S,
    scenario: &Scenario<I>,
    slice: u64,
    inspect_cpu: impl Fn(&S) -> Cpu,
    mut host_action: impl FnMut(&mut S, &HostCommand) -> Result<(), String>,
) -> Result<Trace, String>
where
    S: System<Pixel = u16>,
    I: Copy,
    S::Input: From<Timed<I>>,
{
    scenario.validate().map_err(str::to_owned)?;
    if scenario.link.is_some() {
        return Err("linked scenario requires a system-specific linked runner".into());
    }
    if slice == 0
        || S::VIDEO.width != WIDTH as u32
        || S::VIDEO.height != HEIGHT as u32
        || S::VIDEO.pixel != PixelFormat::Bgr555
    {
        return Err("invalid slice or video format".into());
    }
    let mut trace = Trace::default();
    let mut cursor = 0;
    let mut action_cursor = 0;
    let mut audio = Vec::new();
    let mut buffer = [StereoSample::default(); 4096];
    let inputs: Vec<S::Input> =
        scenario.inputs.iter().map(|i| Timed::new(i.at, i.change).into()).collect();
    for &checkpoint in &scenario.checkpoints {
        while action_cursor < scenario.host_actions.len()
            && scenario.host_actions[action_cursor].at <= system.now()
        {
            host_action(system, &scenario.host_actions[action_cursor].command)?;
            action_cursor += 1;
        }
        while system.now() < checkpoint
            || (cursor < scenario.inputs.len() && scenario.inputs[cursor].at <= checkpoint)
            || (action_cursor < scenario.host_actions.len()
                && scenario.host_actions[action_cursor].at <= checkpoint)
        {
            let before = system.now();
            let target = before
                .saturating_add(slice)
                .min(checkpoint)
                .min(scenario.host_actions.get(action_cursor).map_or(u64::MAX, |a| a.at))
                .max(before);
            let result = system
                .run(target, checkpoint, &inputs[cursor..])
                .map_err(|e| format!("run: {e:?}"))?;
            cursor += result.inputs_consumed;
            loop {
                let n = system.drain_audio(&mut buffer);
                for s in &buffer[..n] {
                    audio.extend_from_slice(&s.left.to_le_bytes());
                    audio.extend_from_slice(&s.right.to_le_bytes());
                }
                if n < buffer.len() {
                    break;
                }
            }
            if matches!(
                result.reason,
                evenfall_host::Stop::Faulted(_) | evenfall_host::Stop::PowerStopped
            ) {
                return Err(format!("guest stopped at {}: {:?}", result.stopped_at, result.reason));
            }
            while action_cursor < scenario.host_actions.len()
                && scenario.host_actions[action_cursor].at <= system.now()
                && scenario.host_actions[action_cursor].at <= checkpoint
            {
                host_action(system, &scenario.host_actions[action_cursor].command)?;
                action_cursor += 1;
            }
            if result.stopped_at == before && result.inputs_consumed == 0 && target > before {
                return Err("run made no progress".into());
            }
        }
        let mut frame = Vec::with_capacity(WIDTH * HEIGHT * 2);
        for pixel in system.framebuffer() {
            frame.extend_from_slice(&pixel.to_le_bytes());
        }
        if frame.len() != WIDTH * HEIGHT * 2 {
            return Err("wrong frame length".into());
        }
        let cpu = inspect_cpu(system);
        if checkpoint == scenario.end_cycle {
            if scenario.terminal_pc.is_some_and(|pc| cpu.pc != pc) {
                return Err(format!(
                    "terminal PC {:08x}, expected {:?}",
                    cpu.pc, scenario.terminal_pc
                ));
            }
            if scenario.terminal_r12.is_some_and(|r12| cpu.registers[12] != r12) {
                return Err(format!(
                    "terminal r12 {:08x}, expected {:?}",
                    cpu.registers[12], scenario.terminal_r12
                ));
            }
        }
        trace.0.push(Observation {
            requested_cycle: checkpoint,
            cycle: system.now(),
            frames: system.completed_frames(),
            cpu,
            audio_rate: system.audio_sample_rate(),
            frame,
            audio: std::mem::take(&mut audio),
            save: system.save_media().to_vec(),
            state: system.write_state().map_err(|e| format!("state: {e:?}"))?,
        });
    }
    Ok(trace)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    pub checkpoint: usize,
    pub kind: &'static str,
    pub detail: String,
}

/// First mismatch per observation kind, in checkpoint order. Pixel and audio
/// offsets are reported in pixels and stereo sample pairs, not byte offsets.
pub fn compare(a: &Trace, b: &Trace) -> Vec<Difference> {
    let mut found = Vec::new();
    if a.0.len() != b.0.len() {
        found.push(Difference {
            checkpoint: a.0.len().min(b.0.len()),
            kind: "checkpoint count",
            detail: format!("{} vs {}", a.0.len(), b.0.len()),
        });
    }
    for (checkpoint, (a, b)) in a.0.iter().zip(&b.0).enumerate() {
        let mut add = |kind, detail: String| {
            if !found.iter().any(|d: &Difference| d.kind == kind) {
                found.push(Difference { checkpoint, kind, detail });
            }
        };
        for (kind, left, right) in [
            ("requested cycle", a.requested_cycle, b.requested_cycle),
            ("cycle", a.cycle, b.cycle),
            ("frames", a.frames, b.frames),
            ("audio rate", u64::from(a.audio_rate), u64::from(b.audio_rate)),
        ] {
            if left != right {
                add(kind, format!("{left} vs {right}"));
            }
        }
        if a.cpu != b.cpu {
            add("cpu", format!("{:?} vs {:?}", a.cpu, b.cpu));
        }
        for (kind, left, right, unit) in [
            ("pixel", &a.frame, &b.frame, 2),
            ("audio", &a.audio, &b.audio, 4),
            ("save", &a.save, &b.save, 1),
            ("state", &a.state, &b.state, 1),
        ] {
            if left != right {
                let index = left
                    .chunks(unit)
                    .zip(right.chunks(unit))
                    .position(|(x, y)| x != y)
                    .unwrap_or(left.len().min(right.len()) / unit);
                let detail = match kind {
                    "pixel" => format!(
                        "({},{}) {:?} vs {:?}",
                        index % WIDTH,
                        index / WIDTH,
                        left.get(index * 2..index * 2 + 2)
                            .map(|v| u16::from_le_bytes([v[0], v[1]])),
                        right
                            .get(index * 2..index * 2 + 2)
                            .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    ),
                    "audio" => format!(
                        "sample {index}: {:?} vs {:?}",
                        left.get(index * 4..index * 4 + 4).map(|v| (
                            i16::from_le_bytes([v[0], v[1]]),
                            i16::from_le_bytes([v[2], v[3]])
                        )),
                        right.get(index * 4..index * 4 + 4).map(|v| (
                            i16::from_le_bytes([v[0], v[1]]),
                            i16::from_le_bytes([v[2], v[3]])
                        ))
                    ),
                    _ => format!("byte {index}: {:?} vs {:?}", left.get(index), right.get(index)),
                };
                add(kind, detail);
            }
        }
    }
    found
}
