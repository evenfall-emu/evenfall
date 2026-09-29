//! Native harness for the same contract as ZiGBA's emu-bench adapter.
use evenfall_gba::cartridge::save::Type as SaveType;
use evenfall_gba::machine::{BootMode, Config, Machine, StopReason};
use std::fs;
use std::io::{self, Write};
use std::time::Instant;

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: evenfall-harness ROM FRAMES [--bios=PATH] [--dump-frame=PATH]",
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut positional = Vec::new();
    let (mut bios_path, mut dump_path) = (None, None);
    for arg in std::env::args().skip(1) {
        if let Some(path) = arg.strip_prefix("--bios=") {
            if path.is_empty() || bios_path.replace(path.to_owned()).is_some() {
                return Err(usage().into());
            }
        } else if let Some(path) = arg.strip_prefix("--dump-frame=") {
            if path.is_empty() || dump_path.replace(path.to_owned()).is_some() {
                return Err(usage().into());
            }
        } else if arg.starts_with("--") || positional.len() == 2 {
            return Err(usage().into());
        } else {
            positional.push(arg);
        }
    }
    if positional.len() != 2 {
        return Err(usage().into());
    }
    let frames: u64 = positional[1].parse()?;
    if frames == 0 {
        return Err(usage().into());
    }
    let rom = fs::read(&positional[0])?;
    if rom.len() > 33 * 1024 * 1024 {
        return Err(usage().into());
    }
    let bios = bios_path.as_ref().map(fs::read).transpose()?.unwrap_or_default();
    if bios_path.is_some() && bios.len() != 16 * 1024 {
        return Err(usage().into());
    }
    let mut machine = Machine::new(Config {
        rom: &rom,
        bios: &bios,
        boot: if bios_path.is_some() { BootMode::SkipBios } else { BootMode::Hle },
        save_type: SaveType::None,
        ..Config::default()
    });
    let target = frames.checked_mul(Machine::CYCLES_PER_FRAME).ok_or_else(usage)?;
    let started = Instant::now();
    for frame in 1..=frames {
        let result = machine
            .run_until(frame * Machine::CYCLES_PER_FRAME)
            .map_err(|error| format!("run failed: {error:?} at cycle {}", machine.now()))?;
        if matches!(result.reason, StopReason::Faulted(_) | StopReason::PowerStopped) {
            return Err(
                format!("run stopped: {:?} at cycle {}", result.reason, machine.now()).into()
            );
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    if seconds <= 0.0 || machine.now() < target {
        return Err("benchmark timer did not advance or machine stopped early".into());
    }
    #[cfg(feature = "kernel-census")]
    eprintln!("kernel-census: {:?}", evenfall_gba::cpu::kernel_census());
    #[cfg(feature = "kernel-census")]
    eprintln!("kernel-classes: {:?}", evenfall_gba::cpu::kernel_classes());
    let pixels = machine.framebuffer();
    let mut hash: u64 = 14_695_981_039_346_656_037;
    for &pixel in pixels {
        for byte in [(pixel & 0xff) as u8, ((pixel & 0x7fff) >> 8) as u8] {
            hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
        }
    }
    println!(
        "benchmark: {{\"schema\":1,\"emulator\":\"evenfall\",\"frames\":{frames},\"seconds\":{seconds:.9},\"cycle\":\"{}\",\"boot\":\"{}\",\"video\":\"full\",\"audio\":\"exact\",\"framebufferHash\":\"{hash:016x}\",\"settings\":{{}}}}",
        machine.now(),
        if bios_path.is_some() { "skip-bios" } else { "hle" },
    );
    if let Some(path) = dump_path {
        let mut ppm = fs::File::create(path)?;
        ppm.write_all(b"P6\n240 160\n255\n")?;
        for &pixel in pixels {
            let rgb = std::array::from_fn::<u8, 3, _>(|channel| {
                let value = ((pixel >> (5 * channel)) & 31) as u8;
                (value << 3) | (value >> 2)
            });
            ppm.write_all(&rgb)?;
        }
    }
    Ok(())
}
