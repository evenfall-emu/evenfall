//! Pinned jsmolka/gba-tests images; hashes and oracles from ZiGBA's test/jsmolka_cases.zig.

use super::{Case, Context, Pixel, Result};
use evenfall_gba::{
    cartridge::save::Type,
    machine::{BootMode, Config, Machine},
};
use sha2::{Digest, Sha256};

struct Report {
    hash: &'static str,
    evaluation: u32,
    idle: u32,
    register: u8,
    save_type: Type,
    max_steps: usize,
}

const REPORTS: [Report; 8] = [
    Report {
        hash: "77ee88662552bdc885c1080c0172ff119d54db791bd73b21808cf1ff1fe5b40e",
        evaluation: 0x0800_1ec4,
        idle: 0x0800_1ec4,
        register: 12,
        save_type: Type::Auto,
        max_steps: 250_000,
    },
    Report {
        hash: "b5cb2291df4ab314b31c598acd9bff2ccfa0b38efff29daadfe97422ce369b67",
        evaluation: 0x0800_0934,
        idle: 0x0800_0aac,
        register: 7,
        save_type: Type::Auto,
        max_steps: 250_000,
    },
    Report {
        hash: "21024fb6aae6343f5f0466dd54e3149de1fbeb23f78e7d85a015c983684d2f87",
        evaluation: 0x0800_0350,
        idle: 0x0800_04c8,
        register: 12,
        save_type: Type::Auto,
        max_steps: 250_000,
    },
    Report {
        hash: "9d7b369fa1aa661ff03692b3d79c6f644b623d72983d0fc890e6d87a0409a3c9",
        evaluation: 0x0800_0248,
        idle: 0x0800_03c0,
        register: 12,
        save_type: Type::Auto,
        max_steps: 250_000,
    },
    Report {
        hash: "edb34ba6590d070c8a50cf0f3566b1e3cc679377b978224ff1b872d27f2b1630",
        evaluation: 0x0800_0130,
        idle: 0x0800_02a8,
        register: 12,
        save_type: Type::None,
        max_steps: 10_000_000,
    },
    Report {
        hash: "a37ad99c31e3f805eb05a00e498b65bd78e6f43a0a139cd695bea1f88229af2c",
        evaluation: 0x0800_02f8,
        idle: 0x0800_0470,
        register: 12,
        save_type: Type::Sram,
        max_steps: 10_000_000,
    },
    Report {
        hash: "7e2aa32e943aedde88bd750eadcdbf55152d3a1ec61385011b7f15cd85b07c02",
        evaluation: 0x0800_0950,
        idle: 0x0800_0ac8,
        register: 12,
        save_type: Type::Flash64k,
        max_steps: 10_000_000,
    },
    Report {
        hash: "9ac50e51d3ce4209dbdf85e472e70c067d5827e9af1bb3e707f6bd9059d5f0c6",
        evaluation: 0x0800_0ad4,
        idle: 0x0800_0c4c,
        register: 12,
        save_type: Type::Flash128k,
        max_steps: 10_000_000,
    },
];

pub(super) fn cases() -> Vec<Case> {
    vec![
        Case { name: "jsmolka-arm", roms: &["arm/arm.gba"], run: report::<0> },
        Case { name: "jsmolka-thumb", roms: &["thumb/thumb.gba"], run: report::<1> },
        Case { name: "jsmolka-memory", roms: &["memory/memory.gba"], run: report::<2> },
        Case { name: "jsmolka-bios", roms: &["bios/bios.gba"], run: report::<3> },
        Case { name: "jsmolka-save-none", roms: &["save/none.gba"], run: report::<4> },
        Case { name: "jsmolka-save-sram", roms: &["save/sram.gba"], run: report::<5> },
        Case { name: "jsmolka-save-flash64", roms: &["save/flash64.gba"], run: report::<6> },
        Case { name: "jsmolka-save-flash128", roms: &["save/flash128.gba"], run: report::<7> },
        Case { name: "jsmolka-unsafe", roms: &["unsafe/unsafe.gba"], run: screen::<false> },
        Case { name: "jsmolka-nes", roms: &["nes/nes.gba"], run: screen::<true> },
        Case { name: "jsmolka-ppu-hello", roms: &["ppu/hello.gba"], run: still::<0> },
        Case { name: "jsmolka-ppu-shades", roms: &["ppu/shades.gba"], run: still::<1> },
        Case { name: "jsmolka-ppu-stripes", roms: &["ppu/stripes.gba"], run: still::<2> },
    ]
}

fn boot(ctx: &Context, hash: &str, save_type: Type) -> Result<Box<Machine>> {
    let actual = format!("{:x}", Sha256::digest(&ctx.roms[0]));
    if actual != hash {
        return Err(format!("ROM hash mismatch; expected sha256 {hash}, found {actual}"));
    }
    Ok(ctx.boot(
        0,
        Config {
            bios: &ctx.bios,
            boot: if ctx.bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
            save_type,
            ..Config::default()
        },
    ))
}

fn budget(ctx: &Context, hle: usize) -> usize {
    hle + if ctx.bios.is_empty() { 0 } else { 20_000_000 }
}

fn report<const N: usize>(ctx: &Context) -> Result {
    let spec = &REPORTS[N];
    let mut machine = boot(ctx, spec.hash, spec.save_type)?;
    let limit = budget(ctx, spec.max_steps);
    let mut evaluated = false;
    for _ in 0..limit {
        let address = machine.next_instruction_address();
        if address == spec.evaluation {
            evaluated = true;
            let result = machine.read_register(spec.register);
            if result != 0 {
                return Err(format!("failed at test {result}"));
            }
        }
        if address == spec.idle {
            if !evaluated {
                return Err(format!("reached {address:08x} before evaluating"));
            }
            ctx.advance(&mut machine)?;
            if machine.next_instruction_address() != spec.idle {
                return Err("terminal address is not a stable self-loop".into());
            }
            return Ok(());
        }
        ctx.advance(&mut machine)?;
    }
    Err(format!(
        "no terminal self-loop within {limit} steps: pc={:08x} r12={}",
        machine.next_instruction_address(),
        machine.read_register(12)
    ))
}

fn screen<const NES: bool>(ctx: &Context) -> Result {
    let (hash, save) = if NES {
        ("d990df112763087d0415b3785c1b4d31c0237794a704d0446fc5f5e474a44f98", Type::None)
    } else {
        ("bb727d59fa81915a5f5c609f4befb64872d3a3d830bc6ae0149e26410e648f85", Type::Sram)
    };
    let mut machine = boot(ctx, hash, save)?;
    let limit = budget(ctx, 1_000_000);
    let mut steps = 0;
    let idle = loop {
        if steps == limit {
            return Err(format!("no terminal self-loop within {limit} steps"));
        }
        let address = machine.next_instruction_address();
        if !in_bios(address) && machine.read_register(12) != 0 {
            return Err(format!("failed at test {}", machine.read_register(12)));
        }
        ctx.advance(&mut machine)?;
        if !in_bios(address) && machine.next_instruction_address() == address {
            break address;
        }
        steps += 1;
    };
    let target = machine.completed_frames() + 2;
    while machine.completed_frames() < target && steps < limit {
        ctx.advance(&mut machine)?;
        steps += 1;
    }
    if machine.completed_frames() < target || machine.next_instruction_address() != idle {
        return Err("terminal self-loop did not hold for two frames".into());
    }
    ctx.expect_frame(&machine, all_tests_passed_pixel)
}

fn still<const N: usize>(ctx: &Context) -> Result {
    let hashes = [
        "38aed48b67bc0f701e8aa222b0c3334bd306bd29888707bb7224d81f5576c264",
        "127ce348f17e4d5fd8a0821a7af757eda6109c66c9f7b8f7d1e0fa31831b407d",
        "80d6cc05b5f289b9a2392621fc245270f08e9f0d77fd33982120cef240f5a504",
    ];
    let pixel: [Pixel; 3] = [hello_pixel, shades_pixel, stripes_pixel];
    let mut machine = boot(ctx, hashes[N], Type::Auto)?;
    let limit = budget(ctx, 500_000);
    let mut steps = 0;
    while !in_rom(machine.next_instruction_address()) && steps < limit {
        ctx.advance(&mut machine)?;
        steps += 1;
    }
    let target = machine.completed_frames() + 2;
    while machine.completed_frames() < target && steps < limit {
        ctx.advance(&mut machine)?;
        steps += 1;
    }
    if machine.completed_frames() < target {
        return Err(format!("two frames did not complete within {limit} steps"));
    }
    ctx.expect_frame(&machine, pixel[N])
}

fn in_bios(address: u32) -> bool {
    address < 0x4000
}
fn in_rom(address: u32) -> bool {
    (0x0800_0000..0x0e00_0000).contains(&address)
}

fn all_tests_passed_pixel(x: usize, y: usize) -> u16 {
    if glyph_pixel("All tests passed", 56, 76, x, y) { 0 } else { 0x7fff }
}
fn hello_pixel(x: usize, y: usize) -> u16 {
    if glyph_pixel("Hello world!", 72, 76, x, y) { 0x7fff } else { 0 }
}
fn shades_pixel(x: usize, _: usize) -> u16 {
    ((x / 16) as u16) << 11
}
fn stripes_pixel(x: usize, _: usize) -> u16 {
    if (x / 8) & 1 == 0 { 0x560b } else { 0x6290 }
}

// Glyphs from jsmolka/gba-tests lib/glyphs.asm (MIT, Julian Smolka).
fn glyph_pixel(text: &str, origin_x: usize, origin_y: usize, x: usize, y: usize) -> bool {
    if x < origin_x || x >= origin_x + text.len() * 8 || y < origin_y || y >= origin_y + 8 {
        return false;
    }
    let glyph: u64 = match text.as_bytes()[(x - origin_x) / 8] {
        b' ' => 0x0000_0000_0000_0000,
        b'!' => 0x0018_0018_1818_1818,
        b'A' => 0x0066_6666_7e66_663c,
        b'H' => 0x0066_6666_7e66_6666,
        b'a' => 0x007c_667c_603c_0000,
        b'd' => 0x007c_6666_667c_6060,
        b'e' => 0x003c_067e_663c_0000,
        b'l' => 0x0030_1818_1818_1818,
        b'o' => 0x003c_6666_663c_0000,
        b'p' => 0x0606_3e66_663e_0000,
        b'r' => 0x0006_0606_663e_0000,
        b's' => 0x003e_603c_063c_0000,
        b't' => 0x0038_0c0c_0c3e_0c0c,
        b'w' => 0x0036_7f6b_6363_0000,
        _ => unreachable!(),
    };
    let bit = (y - origin_y) * 8 + (x - origin_x) % 8;
    glyph & (1 << bit) != 0
}
