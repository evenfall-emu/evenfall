//! Device-path throughput; construction and save-state setup are outside timed loops.
use divan::black_box;
use evenfall_gba::apu::Apu;
use evenfall_gba::bus::Bus;
use evenfall_gba::cartridge::save::{Device, Type};
use evenfall_state::{Sink, Source};

fn main() {
    divan::main();
}

// An active square channel forces sampled mixing, unlike the silent constant run.
#[divan::bench]
fn apu_audible(bencher: divan::Bencher) {
    let mut apu = Apu::default();
    apu.write_byte(0x0400_0084, 0x80);
    apu.write_byte(0x0400_0080, 0x77);
    apu.write_byte(0x0400_0062, 0x80);
    apu.write_byte(0x0400_0063, 0xf3);
    apu.write_byte(0x0400_0065, 0x80);
    bencher.bench_local(|| {
        apu.elapse(black_box(280_896));
        black_box(apu.current_sample());
    });
}

#[divan::bench]
fn apu_constant(bencher: divan::Bencher) {
    let mut apu = Apu::default();
    bencher.bench_local(|| {
        apu.elapse(black_box(280_896));
        black_box(apu.current_sample());
    });
}

#[divan::bench]
fn apu_encode(bencher: divan::Bencher) {
    let mut apu = Apu::default();
    apu.elapse(280_896);
    bencher.bench_local(|| {
        let mut output = Vec::with_capacity(1024);
        apu.write_state(&mut Sink::new(Some(&mut output), None)).unwrap();
        black_box(output);
    });
}

#[divan::bench]
fn apu_decode(bencher: divan::Bencher) {
    let apu = Apu::default();
    let mut bytes = Vec::new();
    apu.write_state(&mut Sink::new(Some(&mut bytes), None)).unwrap();
    let mut destination = Apu::default();
    bencher.bench_local(|| {
        destination.read_state(&mut Source::new(black_box(&bytes))).unwrap();
        black_box(&destination);
    });
}

#[divan::bench]
fn save_encode(bencher: divan::Bencher) {
    let save = Device::init(Type::Sram);
    bencher.bench_local(|| {
        let mut bytes = Vec::with_capacity(65_536);
        save.write_state(&mut Sink::new(Some(&mut bytes), None)).unwrap();
        black_box(bytes);
    });
}

#[divan::bench]
fn save_decode(bencher: divan::Bencher) {
    let save = Device::init(Type::Sram);
    let mut bytes = Vec::new();
    save.write_state(&mut Sink::new(Some(&mut bytes), None)).unwrap();
    let mut destination = Device::init(Type::Sram);
    bencher.bench_local(|| {
        destination.read_state(&mut Source::new(black_box(&bytes))).unwrap();
        black_box(&destination);
    });
}

// A whole mode-3 frame drives 160 compositor lines (plus timing and audio)
// through the public bus clock, without adding a benchmark-only renderer API.
#[divan::bench]
fn renderer_frame(bencher: divan::Bencher) {
    let mut bus = Box::new(Bus::init_with_save(&[], Type::None));
    bus.dispcnt = 3 | (1 << 10);
    for (i, pixel) in bus.vram[..240 * 160 * 2].as_chunks_mut::<2>().0.iter_mut().enumerate() {
        pixel.copy_from_slice(&((i as u16) & 0x7fff).to_le_bytes());
    }
    bencher.bench_local(|| {
        bus.elapse(black_box(280_896));
        black_box(bus.framebuffer());
    });
}
