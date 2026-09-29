//! Source-derived probe oracles from ZiGBA's test/probe_cases.zig.

use super::{Case, Context, Pixel, Result};
use evenfall_audio::StereoSample;
use evenfall_gba::{
    bus::{JoyBusCommand, KeyState, PowerState, SioConnection},
    cartridge::{save::Type, tilt::Sample},
    machine::{Config, LinkedStopReason, Machine, run_linked_until},
};

const IDLE_ENTRY: u32 = 0x0800_0300;
const IDLE_LOOP: u32 = 0x0800_0304;
const MAX_STEPS: usize = 10_000_000;
const BLACK: u16 = 0;
const RED: u16 = 0x001f;
const GREEN: u16 = 0x03e0;
const BLUE: u16 = 0x7c00;
const WHITE: u16 = 0x7fff;

type Hook = fn(&mut Machine, usize) -> Result;

macro_rules! report_case {
    ($name:literal, $rom:literal, $expected:expr, $config:expr, $hook:expr) => {
        Case {
            name: concat!("probe-", $name),
            roms: &[concat!($rom, ".gba")],
            run: |ctx| report(ctx, $expected, $config, $hook),
        }
    };
}
macro_rules! frame_case {
    ($name:literal, $rom:literal, $pixel:expr) => {
        Case {
            name: concat!("probe-", $name),
            roms: &[concat!($rom, ".gba")],
            run: |ctx| frame(ctx, $pixel),
        }
    };
}

pub(super) fn cases() -> Vec<Case> {
    vec![
        report_case!("timer", "timer_irq", 10 << 16 | 2 << 8 | 2, Config::default(), None),
        report_case!("display-irq", "display_irq", 456 << 16 | 2 << 8 | 2, Config::default(), None),
        report_case!(
            "keypad",
            "keypad_irq",
            3,
            Config::default(),
            Some(press_a_three_times_then_start)
        ),
        report_case!("halt-stop", "halt_stop", 2, Config::default(), Some(press_a_while_stopped)),
        report_case!("dma", "dma_copy", 0xbf80_07e0, Config::default(), None),
        frame_case!("hdma", "hdma_scroll", hdma_pixel),
        frame_case!("vdma", "vdma_upload", solid_blue),
        Case { name: "probe-eeprom", roms: &["eeprom_rw.gba"], run: eeprom },
        report_case!(
            "rtc",
            "rtc_datetime",
            0,
            Config { rtc_epoch_seconds: Some(981_205_445), ..Config::default() },
            None
        ),
        report_case!(
            "rtc-write",
            "rtc_write",
            0,
            Config { rtc_epoch_seconds: Some(981_205_445), ..Config::default() },
            None
        ),
        report_case!(
            "gyro",
            "gyro_sample",
            0x6c0,
            Config { gyro_sample: Some(0x6c0), ..Config::default() },
            None
        ),
        report_case!(
            "solar",
            "solar_count",
            4,
            Config { solar_level: Some(252), ..Config::default() },
            None
        ),
        report_case!(
            "tilt",
            "tilt_sample",
            0x03a0_8392,
            Config {
                save_type: Type::None,
                tilt_sample: Some(Sample { x: 0x392, y: 0x3a0 }),
                ..Config::default()
            },
            None
        ),
        Case { name: "probe-rumble", roms: &["rumble_flag.gba"], run: rumble },
        Case {
            name: "probe-link",
            roms: &["link_parent.gba", "link_child.gba"],
            run: |ctx| linked(ctx, SioConnection::Cable2p, &[0x5a, 0xa5]),
        },
        Case {
            name: "probe-multi4",
            roms: &[
                "multi4_parent.gba",
                "multi4_child1.gba",
                "multi4_child2.gba",
                "multi4_child3.gba",
            ],
            run: |ctx| linked(ctx, SioConnection::CableMulti, &[0x4444, 0x3333, 0x4444, 0x1111]),
        },
        Case {
            name: "probe-stop-sio",
            roms: &["multi_parent.gba", "stop_sio_child.gba"],
            run: |ctx| linked(ctx, SioConnection::CableMulti, &[0xabcd, 0x1234]),
        },
        Case {
            name: "probe-uart",
            roms: &["uart_tx.gba", "uart_rx.gba"],
            run: |ctx| linked(ctx, SioConnection::Cable2p, &[0, 0x3c_5aa5]),
        },
        report_case!(
            "joybus",
            "joybus_slave",
            0x4433_2211,
            Config::default(),
            Some(send_joybus_write)
        ),
        Case { name: "probe-joybustx", roms: &["joybus_tx.gba"], run: joybus_transmit },
        Case {
            name: "probe-mb2slave",
            roms: &[
                "multiboot_2slave_parent.gba",
                "multiboot_2slave_child.gba",
                "multiboot_2slave_child.gba",
            ],
            run: |ctx| linked(ctx, SioConnection::CableMulti, &[0, 0, 0]),
        },
        Case {
            name: "probe-mb2mhz",
            roms: &["multiboot_2mhz_parent.gba", "multiboot_normal_child.gba"],
            run: |ctx| linked(ctx, SioConnection::Cable2p, &[0, 0]),
        },
        Case { name: "probe-psg", roms: &["psg_square.gba"], run: psg },
        Case { name: "probe-fifo", roms: &["fifo_stream.gba"], run: fifo },
        Case { name: "probe-noise", roms: &["noise_poly.gba"], run: noise },
        Case { name: "probe-wave", roms: &["wave_ramp.gba"], run: wave },
        Case { name: "probe-wavechain", roms: &["wave_chain.gba"], run: wave_chain },
        report_case!(
            "wavereadback",
            "wave_readback",
            wave_readback_hash(),
            Config::default(),
            None
        ),
        Case { name: "probe-sweep", roms: &["sweep_up.gba"], run: |ctx| sweep(ctx, true) },
        Case { name: "probe-sweepdown", roms: &["sweep_down.gba"], run: |ctx| sweep(ctx, false) },
        Case { name: "probe-envelope", roms: &["envelope_down.gba"], run: envelope },
        frame_case!("obj", "obj_sprite", obj_pixel),
        frame_case!("objrot", "obj_rot", obj_rot_pixel),
        frame_case!("objdouble", "obj_doublesize", obj_double_pixel),
        frame_case!("objflip", "obj_flip", obj_flip_pixel),
        frame_case!("objmosaic", "obj_mosaic", obj_mosaic_pixel),
        frame_case!("objsemi", "obj_semitransparent", obj_semi_pixel),
        frame_case!("objwin", "obj_window", obj_window_pixel),
        frame_case!("affinerot", "affine_rot", affine_rot_pixel),
        frame_case!("affinewrap", "affine_wrap", affine_wrap_pixel),
        frame_case!("mode1", "mode1_mixed", mode1_pixel),
        frame_case!("blend", "blend_alpha", solid_blend),
        frame_case!("brighten", "brighten", solid_bright),
        frame_case!("darken", "brighten_dark", solid_dark),
        frame_case!("window", "window_rect", window_pixel),
        frame_case!("mosaic", "mosaic_bg", mosaic_pixel),
        frame_case!("text8", "text_8bpp", text8_pixel),
        frame_case!("scrollwrap", "scroll_wrap", scroll_wrap_pixel),
        frame_case!("bitmap3", "bitmap_mode3", bitmap3_pixel),
        frame_case!("bitmap4", "bitmap_mode4", bitmap4_pixel),
        frame_case!("bitmap5", "bitmap_mode5", bitmap5_pixel),
        frame_case!("greenswap", "green_swap", green_swap_pixel),
        frame_case!("forceblank", "forced_blank", solid_white),
        Case { name: "probe-midcombo", roms: &["midline_combo.gba"], run: midline_combo },
        Case {
            name: "probe-midwin",
            roms: &["midline_win.gba"],
            run: |ctx| split(ctx, solid_red, Some(solid_black_line)),
        },
        Case {
            name: "probe-midbldy",
            roms: &["midline_bldy.gba"],
            run: |ctx| split(ctx, solid_red, Some(solid_white_line)),
        },
        Case {
            name: "probe-midaffine",
            roms: &["midline_affine.gba"],
            run: |ctx| split(ctx, affine_identity_pixel, None),
        },
        Case { name: "probe-midmosaic", roms: &["midline_mosaic.gba"], run: midline_mosaic },
    ]
}

fn run_to_idle(
    ctx: &Context,
    machine: &mut Machine,
    hook: Option<Hook>,
    mut audio: Option<&mut Audio>,
) -> Result {
    let mut found = false;
    for step in 0..MAX_STEPS {
        if machine.next_instruction_address() == IDLE_ENTRY {
            found = true;
            break;
        }
        if let Some(action) = hook {
            action(machine, step)?;
        }
        ctx.advance(machine)?;
        if let Some(stream) = &mut audio {
            stream.drain(machine)?;
        }
    }
    if !found {
        return Err(format!(
            "no idle entry within {MAX_STEPS} steps: pc={:08x} r12={:08x}",
            machine.next_instruction_address(),
            machine.read_register(12)
        ));
    }
    ctx.advance(machine)?;
    if machine.next_instruction_address() != IDLE_LOOP {
        return Err("did not settle into the idle loop".into());
    }
    if let Some(stream) = &mut audio {
        stream.drain(machine)?;
    }
    Ok(())
}

fn expect_report(machine: &Machine, expected: u32) -> Result {
    let got = machine.read_register(12);
    if got != expected {
        return Err(format!("reported r12={got:08x}, expected {expected:08x}"));
    }
    Ok(())
}

fn report(ctx: &Context, expected: u32, config: Config<'_>, hook: Option<Hook>) -> Result {
    let mut machine = ctx.boot(0, config);
    run_to_idle(ctx, &mut machine, hook, None)?;
    expect_report(&machine, expected)
}

fn frame(ctx: &Context, pixel: Pixel) -> Result {
    let mut machine = ctx.boot(0, Config::default());
    run_to_idle(ctx, &mut machine, None, None)?;
    expect_report(&machine, 0)?;
    ctx.expect_frame(&machine, pixel)
}

fn linked(ctx: &Context, connection: SioConnection, expected: &[u32]) -> Result {
    let mut machines: Vec<Box<Machine>> =
        (0..ctx.roms.len()).map(|index| ctx.boot(index, Config::default())).collect();
    let mut settled = false;
    for _ in 0..2000 {
        let latest = machines.iter().map(|machine| machine.now()).max().unwrap();
        let mut peers: Vec<&mut Machine> = machines.iter_mut().map(Box::as_mut).collect();
        let result = run_linked_until(&mut peers, latest + 20_000, connection)
            .map_err(|e| format!("cable error: {e:?}"))?;
        match result.reason {
            LinkedStopReason::Deadline => {}
            LinkedStopReason::PowerStopped => {
                return Err("every machine stopped with nothing armed to wake it".into());
            }
            LinkedStopReason::Faulted(fault) => {
                return Err(format!(
                    "machine {} faulted at {:08x}: {:?}",
                    fault.machine_index, fault.fault.instruction_address, fault.fault.reason
                ));
            }
        }
        let idle = machines.iter().all(|machine| machine.next_instruction_address() == IDLE_LOOP);
        if settled {
            if !idle {
                return Err("idle loops were not stable".into());
            }
            for (machine, want) in machines.iter().zip(expected) {
                expect_report(machine, *want)?;
            }
            return Ok(());
        }
        settled = idle;
    }
    Err(format!(
        "not every machine reached its idle loop within 2000 chunks: pc {:?}",
        machines.iter().map(|m| m.next_instruction_address()).collect::<Vec<_>>()
    ))
}

fn press_a_three_times_then_start(machine: &mut Machine, step: usize) -> Result {
    if step != 0 && step.is_multiple_of(2000) && step <= 14_000 {
        machine.set_keys(if step == 14_000 {
            KeyState { start: true, ..KeyState::default() }
        } else if step % 4000 == 2000 {
            KeyState { a: true, ..KeyState::default() }
        } else {
            KeyState::default()
        });
    }
    Ok(())
}

fn press_a_while_stopped(machine: &mut Machine, _: usize) -> Result {
    if machine.bus.power_state() == PowerState::Stopped {
        machine.set_keys(KeyState { a: true, ..KeyState::default() });
    }
    Ok(())
}

fn send_joybus_write(machine: &mut Machine, step: usize) -> Result {
    if step == 2000 {
        machine
            .bus
            .schedule_joy_bus(JoyBusCommand::WriteToGba(0x4433_2211), machine.now() + 1000)
            .map_err(|e| format!("JoyBus: {e:?}"))?;
    }
    Ok(())
}

fn eeprom(ctx: &Context) -> Result {
    let mut machine = ctx.boot(0, Config::default());
    run_to_idle(ctx, &mut machine, None, None)?;
    if machine.read_register(12) != 0 {
        return Err(format!("failed at stage {}", machine.read_register(12)));
    }
    if machine.save_type() != Type::Eeprom512 {
        return Err(format!("save type {:?}, expected eeprom_512", machine.save_type()));
    }
    let data = machine.save_data();
    if data.len() != 512 || data[40..48] != [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef] {
        return Err("save block 5 differs from 01 23 45 67 89 ab cd ef".into());
    }
    Ok(())
}

fn rumble(ctx: &Context) -> Result {
    let mut machine = ctx.boot(0, Config { rumble: true, ..Config::default() });
    run_to_idle(ctx, &mut machine, None, None)?;
    expect_report(&machine, 0)?;
    if !machine.rumble_active() {
        return Err("rumble motor flag is not active".into());
    }
    Ok(())
}

fn joybus_transmit(ctx: &Context) -> Result {
    let mut machine = ctx.boot(0, Config::default());
    run_to_idle(
        ctx,
        &mut machine,
        Some(|machine, step| {
            if step == 2000 {
                machine
                    .bus
                    .schedule_joy_bus(JoyBusCommand::ReadFromGba, machine.now() + 1000)
                    .map_err(|e| format!("JoyBus: {e:?}"))?;
            }
            Ok(())
        }),
        None,
    )?;
    expect_report(&machine, 0)?;
    let response = machine.bus.take_joy_bus_response().ok_or("no master response")?;
    if response.data[..response.len as usize] != [0x11, 0x22, 0x33, 0x44, 0x08] {
        return Err(format!(
            "master response {:?}, expected 11 22 33 44 08",
            &response.data[..response.len as usize]
        ));
    }
    Ok(())
}

struct Audio {
    samples: Vec<StereoSample>,
}
impl Audio {
    fn drain(&mut self, machine: &mut Machine) -> Result {
        let mut buffer = [StereoSample::default(); 1024];
        let len = machine.drain_audio_samples(&mut buffer);
        self.samples.extend_from_slice(&buffer[..len]);
        if self.samples.len() >= 16_384 {
            return Err("audio stream exceeds 16384 samples".into());
        }
        Ok(())
    }
}

// Drain after every instruction: the machine's bounded queue is not the full stream.
fn record(ctx: &Context) -> Result<Vec<i16>> {
    let mut machine = ctx.boot(0, Config::default());
    let mut audio = Audio { samples: Vec::new() };
    run_to_idle(ctx, &mut machine, None, Some(&mut audio))?;
    expect_report(&machine, 0)?;
    if machine.audio_sample_rate() != 32_768 {
        return Err(format!("sample rate {}, expected 32768", machine.audio_sample_rate()));
    }
    let ticks = machine.now() / 512;
    if audio.samples.len() as u64 != ticks && audio.samples.len() as u64 != ticks + 1 {
        return Err(format!(
            "{} samples in {} cycles, expected one per 512",
            audio.samples.len(),
            machine.now()
        ));
    }
    let mut stream = Vec::with_capacity(audio.samples.len());
    for (index, sample) in audio.samples.iter().enumerate() {
        if sample.left != sample.right {
            return Err(format!("left/right diverge at sample {index}"));
        }
        stream.push(sample.left);
    }
    Ok(stream)
}

#[derive(Clone, Copy)]
struct Run {
    value: i16,
    len: usize,
}
fn runs(stream: &[i16]) -> Result<Vec<Run>> {
    let mut all = Vec::new();
    let mut index = 0;
    while index < stream.len() {
        let end = index + stream[index..].iter().take_while(|&&v| v == stream[index]).count();
        all.push(Run { value: stream[index], len: end - index });
        index = end;
    }
    if all.len() < 3 {
        return Err(format!("stream has only {} runs", all.len()));
    }
    Ok(all)
}

fn expect_levels(stream: &[i16], allowed: &[i16]) -> Result {
    for (index, sample) in stream.iter().enumerate() {
        if !allowed.contains(sample) {
            return Err(format!("sample {index} is {sample}, outside {allowed:?}"));
        }
    }
    Ok(())
}
fn expect_nibble_levels(stream: &[i16]) -> Result {
    for (index, &sample) in stream.iter().enumerate() {
        if !(0..=240).contains(&sample) || sample % 16 != 0 {
            return Err(format!("sample {index} is {sample}, not a nibble times 16"));
        }
    }
    Ok(())
}

fn psg(ctx: &Context) -> Result {
    // Duty steps last eight DAC ticks, four high steps per cycle: 32-sample runs.
    let stream = record(ctx)?;
    expect_levels(&stream, &[0, 240])?;
    let all = runs(&stream)?;
    if all[0].value != 240 {
        return Err("head run is not high".into());
    }
    let middle = &all[1..all.len() - 1];
    for (index, run) in middle.iter().enumerate() {
        if run.len != 32 || (run.value == 240) != (index % 2 == 1) {
            return Err(format!("run {} breaks the 32-sample alternation", index + 1));
        }
    }
    if middle.len() < 16 {
        return Err(format!("only {} steady runs", middle.len()));
    }
    Ok(())
}

fn fifo(ctx: &Context) -> Result {
    let stream = record(ctx)?;
    expect_levels(&stream, &[0, 256, -256])?;
    let all = runs(&stream)?;
    let mut positive = all[0].value != 256;
    for (index, run) in all.iter().enumerate().skip(1) {
        let tail = index == all.len() - 1;
        if (run.value == 256) != positive
            || run.value == 0
            || (!tail && run.len != 16)
            || run.len > 16
        {
            return Err(format!("run {index} breaks the 16-sample alternation"));
        }
        positive = !positive;
    }
    if all.len() < 14 {
        return Err(format!("only {} steady runs", all.len() - 2));
    }
    Ok(())
}

fn find_lfsr(stream: &[i16], wide: bool) -> Option<usize> {
    let mut reference = [false; 64];
    let mut state: u16 = if wide { 0x4000 } else { 0x40 };
    for high in &mut reference {
        *high = state & 1 != 0;
        state >>= 1;
        if *high {
            state ^= if wide { 0x6000 } else { 0x60 };
        }
    }
    (0..=stream.len().saturating_sub(reference.len() * 2)).find(|&start| {
        stream.len() >= start + reference.len() * 2
            && reference.iter().enumerate().all(|(step, high)| {
                let want = if *high { 240 } else { 0 };
                stream[start + step * 2] == want && stream[start + step * 2 + 1] == want
            })
    })
}

fn noise(ctx: &Context) -> Result {
    let stream = record(ctx)?;
    expect_levels(&stream, &[0, 240])?;
    let narrow = find_lfsr(&stream, false).ok_or("no 7-bit GBATEK tap match")?;
    let wide = find_lfsr(&stream, true).ok_or("no 15-bit GBATEK tap match")?;
    if narrow >= wide {
        return Err(format!("7-bit match at {narrow} does not precede 15-bit match at {wide}"));
    }
    Ok(())
}

fn wave(ctx: &Context) -> Result {
    let stream = record(ctx)?;
    expect_nibble_levels(&stream)?;
    let all = runs(&stream)?;
    for index in 1..all.len() {
        let run = all[index];
        let next = (all[index - 1].value / 16 + 1) % 16;
        if run.value / 16 != next || (index != all.len() - 1 && run.len != 4) || run.len > 4 {
            return Err(format!("run {index} breaks the 4-sample ramp order"));
        }
    }
    if all.len() < 10 {
        return Err(format!("only {} steady runs", all.len() - 2));
    }
    Ok(())
}

fn wave_chain(ctx: &Context) -> Result {
    let stream = record(ctx)?;
    expect_nibble_levels(&stream)?;
    let all = runs(&stream)?;
    let all = &all[1..];
    let mut index = all
        .iter()
        .position(|run| run.value == 112 && run.len == 64)
        .ok_or("no 64-sample bank marker")?;
    let (mut markers, mut ramps) = (0, 0);
    while index < all.len() {
        if all[index].value == 112 && all[index].len == 64 {
            markers += 1;
            index += 1;
            continue;
        }
        if index + 1 == all.len() && all[index].value == 112 {
            break;
        }
        let mut nibble = 0;
        if all[index].value == 0 && all[index].len >= 128 {
            index += 1;
            nibble = 1;
        }
        while nibble < 16 && index < all.len() {
            let last = index + 1 == all.len();
            if all[index].value != nibble * 16 || (all[index].len != 4 && !last) {
                break;
            }
            nibble += 1;
            index += 1;
        }
        if nibble < 16 {
            if index == all.len() {
                break;
            }
            return Err(format!("ramp order breaks at run {index}"));
        }
        ramps += 1;
    }
    if markers == 0 || ramps == 0 {
        return Err(format!("only {markers} markers and {ramps} ramp cycles"));
    }
    Ok(())
}

// GBATEK's shift-register rotation, hashed as in wave_readback.c.
fn wave_readback_hash() -> u32 {
    fn bytes(add: usize) -> [u8; 16] {
        std::array::from_fn(|i| ((((2 * i + add) % 16) << 4) | ((2 * i + add + 1) % 16)) as u8)
    }
    fn rotate(bytes: &[u8], steps: usize) -> Vec<u8> {
        let nibble =
            |q: usize| if q.is_multiple_of(2) { bytes[q / 2] >> 4 } else { bytes[q / 2] & 15 };
        (0..bytes.len())
            .map(|i| {
                nibble((2 * i + steps) % (bytes.len() * 2)) << 4
                    | nibble((2 * i + 1 + steps) % (bytes.len() * 2))
            })
            .collect()
    }
    let phase1 = rotate(&bytes(0), 3);
    let phase2 = rotate(&[bytes(5), bytes(11)].concat(), 67);
    phase1
        .iter()
        .chain(&phase2[16..32])
        .chain(&phase2[..16])
        .fold(2_166_136_261_u32, |hash, byte| (hash ^ u32::from(*byte)).wrapping_mul(16_777_619))
}

fn sweep(ctx: &Context, rising: bool) -> Result {
    let stream = record(ctx)?;
    expect_levels(&stream, &[0, 240])?;
    let all = runs(&stream)?;
    if all.len() < 8 {
        return Err(format!("only {} complete runs", all.len() - 2));
    }
    let complete = &all[1..all.len() - 1];
    for (index, pair) in complete.windows(2).enumerate() {
        let against =
            if rising { pair[1].len > pair[0].len + 1 } else { pair[1].len + 1 < pair[0].len };
        if against {
            return Err(format!(
                "run {} of {} samples goes against the sweep after {}",
                index + 1,
                pair[1].len,
                pair[0].len
            ));
        }
    }
    let first = complete[0].len;
    let last = complete[complete.len() - 1].len;
    let changed = if rising { first >= last + last / 4 } else { first + first / 4 <= last };
    if !changed {
        return Err(format!("runs moved only from {first} to {last} samples"));
    }
    Ok(())
}

fn envelope(ctx: &Context) -> Result {
    let stream = record(ctx)?;
    expect_nibble_levels(&stream)?;
    let all = runs(&stream)?;
    let mut first = None;
    let mut previous = 240;
    let mut levels = 0;
    for run in &all[1..all.len() - 1] {
        if run.value == 0 {
            continue;
        }
        if first.is_none() {
            first = Some(run.value);
            levels = 1;
        } else if run.value > previous {
            return Err(format!("level {} rises after {previous}", run.value));
        } else if run.value < previous {
            levels += 1;
        }
        previous = run.value;
    }
    if first != Some(240) {
        return Err(format!("first level {first:?}, expected 240"));
    }
    if levels < 3 {
        return Err(format!("only {levels} distinct levels"));
    }
    if previous > 208 {
        return Err(format!("lowest level {previous}, expected 208 or below"));
    }
    Ok(())
}

fn solid_red(_: usize, _: usize) -> u16 {
    RED
}
fn solid_blue(_: usize, _: usize) -> u16 {
    BLUE
}
fn solid_white(_: usize, _: usize) -> u16 {
    WHITE
}
fn solid_blend(_: usize, _: usize) -> u16 {
    0x01ef
}
fn solid_bright(_: usize, _: usize) -> u16 {
    0x3dff
}
fn solid_dark(_: usize, _: usize) -> u16 {
    0x0010
}
fn solid_black_line(_: usize) -> u16 {
    BLACK
}
fn solid_white_line(_: usize) -> u16 {
    WHITE
}

fn hdma_pixel(x: usize, y: usize) -> u16 {
    let line = if y == 0 { 0 } else { y - 1 };
    let hofs = ((line + 1) / 16) * 2;
    if (x + hofs) % 8 < 4 { RED } else { GREEN }
}
fn obj_pixel(x: usize, y: usize) -> u16 {
    if !(8..16).contains(&y) {
        return BLACK;
    }
    if (8..16).contains(&x) {
        return if x < 12 { RED } else { GREEN };
    }
    if (16..20).contains(&x) { BLUE } else { BLACK }
}
fn obj_rot_pixel(x: usize, y: usize) -> u16 {
    if !(8..16).contains(&y) || x <= 8 || x >= 16 {
        return BLACK;
    }
    if y < 12 { RED } else { GREEN }
}
fn quadrant(lx: usize, ly: usize) -> u16 {
    match (ly < 4, lx < 4) {
        (true, true) => RED,
        (true, false) => GREEN,
        (false, true) => BLUE,
        (false, false) => WHITE,
    }
}
fn obj_double_pixel(x: usize, y: usize) -> u16 {
    if !(24..32).contains(&x) || !(24..32).contains(&y) { BLACK } else { quadrant(x - 24, y - 24) }
}
fn obj_flip_pixel(x: usize, y: usize) -> u16 {
    if !(20..28).contains(&y) {
        return BLACK;
    }
    for (sx, h, v) in [(20, false, false), (60, true, false), (100, false, true), (140, true, true)]
    {
        if (sx..sx + 8).contains(&x) {
            return quadrant(if h { sx + 7 - x } else { x - sx }, if v { 27 - y } else { y - 20 });
        }
    }
    BLACK
}
fn mosaic_sprite(sx: usize, x: usize, y: usize) -> Option<u16> {
    let last = sx + 7;
    if !(8..16).contains(&y) || x < sx || x >= last - last % 2 + 2 {
        return None;
    }
    let lx = ((x - x % 2) as isize - sx as isize).clamp(0, 7) as usize;
    let ly = (y - y % 2 - 8).min(7);
    Some(if (lx < 4) == (ly < 4) { RED } else { GREEN })
}
fn obj_mosaic_pixel(x: usize, y: usize) -> u16 {
    mosaic_sprite(8, x, y).or_else(|| mosaic_sprite(13, x, y)).unwrap_or(BLACK)
}
fn obj_semi_pixel(x: usize, y: usize) -> u16 {
    if (8..16).contains(&x) && (8..16).contains(&y) { 0x01ef } else { GREEN }
}
fn obj_window_pixel(x: usize, y: usize) -> u16 {
    if (80..88).contains(&x) && (60..68).contains(&y) { RED } else { BLACK }
}
fn affine_tile(map_x: usize, map_y: usize) -> u16 {
    match ((map_y / 8) * 16 + map_x / 8) % 4 {
        0 => RED,
        1 => GREEN,
        2 => BLUE,
        _ => BLACK,
    }
}
fn affine_rot_pixel(x: usize, y: usize) -> u16 {
    if x >= 128 || y >= 128 { BLACK } else { affine_tile(127 - y, x) }
}
fn affine_wrap_pixel(x: usize, y: usize) -> u16 {
    affine_tile((127 - y as isize).rem_euclid(128) as usize, x % 128)
}
fn mode1_pixel(x: usize, y: usize) -> u16 {
    if x < 120 {
        RED
    } else if x < 128 && y < 128 {
        GREEN
    } else {
        BLACK
    }
}
fn window_pixel(x: usize, y: usize) -> u16 {
    if (80..160).contains(&x) && (40..120).contains(&y) { RED } else { BLACK }
}
fn mosaic_pixel(x: usize, y: usize) -> u16 {
    let index = ((y - y % 4) % 8 * 8 + (x - x % 4) % 8) as u32;
    if index == 0 { BLACK } else { ((index * 501) & 0x7fff) as u16 }
}
fn text8_pixel(x: usize, y: usize) -> u16 {
    if y < 96 && x < 192 { RED } else { BLUE }
}
fn scroll_wrap_pixel(x: usize, y: usize) -> u16 {
    let row = (y + 100) % 256 / 8;
    let column = (x + 200) % 256 / 8;
    match (row * 32 + column) % 4 {
        0 => RED,
        1 => GREEN,
        2 => BLUE,
        _ => WHITE,
    }
}
fn bitmap3_pixel(x: usize, _: usize) -> u16 {
    if x < 120 { RED } else { BLUE }
}
fn bitmap4_pixel(_: usize, y: usize) -> u16 {
    if y.is_multiple_of(2) { BLUE } else { WHITE }
}
fn bitmap5_pixel(x: usize, y: usize) -> u16 {
    if x >= 160 || y >= 128 {
        BLACK
    } else if (x / 8 + y / 8).is_multiple_of(2) {
        RED
    } else {
        BLUE
    }
}
fn green_swap_pixel(x: usize, _: usize) -> u16 {
    if x.is_multiple_of(2) { 0x03ff } else { BLACK }
}

const SPLIT_LINE: usize = 40;
fn stripe(phase: usize, x: usize) -> u16 {
    if (x + phase) % 8 < 4 { RED } else { GREEN }
}

fn run_split_probe(ctx: &Context) -> Result<Box<Machine>> {
    let mut machine = ctx.boot(0, Config::default());
    run_to_idle(ctx, &mut machine, None, None)?;
    expect_report(&machine, 0)?;
    Ok(machine)
}
fn split(ctx: &Context, baseline: Pixel, alternate: Option<fn(usize) -> u16>) -> Result {
    let machine = run_split_probe(ctx)?;
    let pixels = machine.framebuffer();
    let mut count = 0;
    for y in 0..160 {
        for x in 0..240 {
            let got = pixels[y * 240 + x];
            if got == baseline(x, y) {
                continue;
            }
            if y != SPLIT_LINE {
                return Err(format!("pixel ({x},{y}) is {got:04x} off the split line"));
            }
            if let Some(other) = alternate
                && got != other(x)
            {
                return Err(format!(
                    "split pixel ({x},{y}) is {got:04x}, expected {:04x}",
                    other(x)
                ));
            }
            count += 1;
        }
    }
    if !(20..=220).contains(&count) {
        return Err(format!("split spans {count} pixels, expected a mid-line span"));
    }
    Ok(())
}
fn affine_identity_pixel(x: usize, y: usize) -> u16 {
    if x < 128 && y < 128 { affine_tile(x, y) } else { BLACK }
}
fn midline_combo(ctx: &Context) -> Result {
    let machine = run_split_probe(ctx)?;
    let mut shifted = 0;
    let mut reds = 0;
    for y in 0..160 {
        for x in 0..240 {
            let got = machine.framebuffer()[y * 240 + x];
            let base = if x < 120 { stripe(0, x) } else { BLUE };
            if got == base {
                continue;
            }
            if y != SPLIT_LINE {
                return Err(format!("pixel ({x},{y}) is {got:04x} off the split line"));
            }
            if x < 120 {
                if got != stripe(4, x) {
                    return Err(format!("pixel ({x},40) is {got:04x}, in neither stripe phase"));
                }
                shifted += 1;
            } else {
                if got != RED {
                    return Err(format!("pixel ({x},40) is {got:04x}, not a backdrop colour"));
                }
                reds += 1;
            }
        }
    }
    if !(10..=110).contains(&shifted) || !(10..=110).contains(&reds) {
        return Err(format!(
            "split spans {shifted} scrolled and {reds} backdrop pixels, expected mid-line spans"
        ));
    }
    Ok(())
}

fn midline_mosaic(ctx: &Context) -> Result {
    let machine = run_split_probe(ctx)?;
    let row = |y: usize| &machine.framebuffer()[y * 240..(y + 1) * 240];
    let run = |row: &[u16], start: usize| {
        1 + row[start + 1..].iter().take_while(|&&pixel| pixel == row[start]).count()
    };
    let blocky = row(10);
    for x in (0..240).step_by(4) {
        if run(blocky, x) != 4 {
            return Err(format!("line 10 has a {}-wide run at x={x}", run(blocky, x)));
        }
    }
    let sharp = row(100);
    for x in 0..240 {
        if run(sharp, x) != 1 {
            return Err(format!("line 100 has a {}-wide run at x={x}", run(sharp, x)));
        }
    }
    let split = row(40);
    let mut x = 0;
    while x < 240 && x % 4 == 0 && run(split, x) == 4 {
        x += 4;
    }
    if !(20..=220).contains(&x) {
        return Err(format!("line 40 switches at x={x}, outside [20,220]"));
    }
    x += run(split, x);
    if 240 - x < 10 {
        return Err(format!("line 40 has only {} sharp pixels", 240 - x));
    }
    while x < 240 {
        if run(split, x) != 1 {
            return Err(format!("line 40 has a {}-wide run at x={x}", run(split, x)));
        }
        x += 1;
    }
    Ok(())
}
