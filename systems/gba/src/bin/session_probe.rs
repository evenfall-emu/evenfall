//! Native side of the native/WASM bounded-session parity oracle.

use evenfall_gba::machine::{Machine, StopReason};
use evenfall_gba::session::{Config, Session};
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("expected ROM and frame count".into());
    }
    let image = std::fs::read(&args[1])?;
    let mut session = Session::create(Config {
        image: &image,
        zipped: args[1].to_ascii_lowercase().ends_with(".zip"),
        ..Config::default()
    })?;
    let frames: u64 = args[2].parse()?;
    let mut audio_hash = Sha256::new();
    let mut samples = [evenfall_audio::StereoSample::default(); 8192];
    let mut target = 0;
    while target < frames * Machine::CYCLES_PER_FRAME {
        target += Machine::CYCLES_PER_FRAME / 4;
        let result = session.advance(target)?;
        if matches!(result.reason, StopReason::Faulted(_) | StopReason::PowerStopped) {
            return Err("early stop".into());
        }
        let count = session.drain_audio(&mut samples)?;
        for sample in &samples[..count] {
            audio_hash.update(sample.left.to_le_bytes());
            audio_hash.update(sample.right.to_le_bytes());
        }
    }
    let snapshot = session.write_state()?;
    let mut memory = vec![0; 256 * 1024];
    session
        .machine
        .inspect_memory(0x0200_0000, &mut memory)
        .map_err(|_| "EWRAM inspection address overflow")?;
    let mut frame = Vec::with_capacity(240 * 160 * 2);
    for pixel in session.machine.framebuffer() {
        frame.extend_from_slice(&pixel.to_le_bytes());
    }
    println!(
        "{{\"cycle\":\"{}\",\"frames\":\"{}\",\"pc\":{},\"cpsr\":{},\"state\":\"{}\",\"ewram\":\"{}\",\"frame\":\"{}\",\"audio\":\"{:x}\",\"save\":\"{}\"}}",
        session.machine.now(),
        session.machine.completed_frames(),
        session.machine.next_instruction_address(),
        session.machine.read_cpsr(),
        hash(&snapshot),
        hash(&memory),
        hash(&frame),
        audio_hash.finalize(),
        hash(session.machine.save_data()),
    );
    Ok(())
}
