//! Device-path throughput; construction and save-state setup are outside timed loops.
use divan::black_box;
use evenfall_gba::apu::Apu;
use evenfall_gba::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use evenfall_gba::cartridge::save::{Device, Type};
use evenfall_gba::dma::Channel;
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

// Data reads use Bus's scalar ROM path, not the instruction-fetch pointer view.
#[divan::bench]
fn uncached_rom_read(bencher: divan::Bencher) {
    let mut rom = vec![0; 256 * 1024];
    for (i, byte) in rom.iter_mut().enumerate() {
        *byte = i.wrapping_mul(37) as u8;
    }
    let mut bus = Box::new(Bus::init_with_save(&rom, Type::None));
    let mut offset = 0x100;
    bencher.bench_local(|| {
        let result = bus.read(Access {
            address: black_box(0x0800_0000 + offset),
            width: AccessWidth::Word,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        });
        black_box(result.value);
        offset = (offset + 4) & 0x3_ffc;
    });
}

// A 16-word ROM-to-RAM burst isolates DMA's scalar ROM source accesses.
#[divan::bench]
fn dma_from_rom(bencher: divan::Bencher) {
    let mut rom = vec![0; 256 * 1024];
    for (i, byte) in rom.iter_mut().enumerate() {
        *byte = i.wrapping_mul(37) as u8;
    }
    let mut bus = Box::new(Bus::init_with_save(&rom, Type::None));
    bencher.bench_local(|| {
        bus.dma_channels[3] = Channel {
            source: 0x0800_0100,
            destination: 0x0200_0000,
            word_count: 16,
            control: 0x8400,
            latched_source: 0x0800_0100,
            latched_destination: 0x0200_0000,
            latched_word_count: 16,
        };
        bus.pending_dma = 1 << 3;
        bus.elapse(1);
        black_box(&bus.ewram[..64]);
    });
    assert_eq!(&bus.ewram[..64], &rom[0x100..0x140]);
}
