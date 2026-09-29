//! Suspendable BIOS copy, affine and decompression services.
//!
//! Each loop stops only at ZiGBA's guest-cycle iteration boundaries. The
//! caller retains `HleBulkWork` without changing the BIOS latch or taking an
//! interrupt until the loop finishes.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::{
    Cpu, HleBgAffineSetWork, HleBitUnpackWork, HleBulkWork, HleCpuFastSetWork, HleCpuSetWork,
    HleDifferentialFilterWork, HleHuffmanWork, HleLz77Work, HleObjAffineSetWork, HleRunLengthWork,
};

fn access(address: u32, width: AccessWidth, kind: AccessKind) -> Access {
    Access { address, width, kind, source: AccessSource::CpuData }
}

fn hle_read_u8(bus: &mut Bus, address: u32) -> u8 {
    bus.read(access(address, AccessWidth::Byte, AccessKind::Nonsequential)).value as u8
}

fn hle_read_u16(bus: &mut Bus, address: u32) -> u16 {
    bus.read(access(address, AccessWidth::Half, AccessKind::Nonsequential)).value as u16
}

fn hle_read_i16(bus: &mut Bus, address: u32) -> i16 {
    hle_read_u16(bus, address) as i16
}

fn hle_read_i32(bus: &mut Bus, address: u32) -> i32 {
    bus.read(access(address, AccessWidth::Word, AccessKind::Nonsequential)).value as i32
}

fn hle_write_i16(bus: &mut Bus, address: u32, value: i16) {
    bus.write(access(address, AccessWidth::Half, AccessKind::Nonsequential), value as u16 as u32);
}

fn hle_write_u32(bus: &mut Bus, address: u32, value: u32) {
    bus.write(access(address, AccessWidth::Word, AccessKind::Nonsequential), value);
}

fn hle_write_i32(bus: &mut Bus, address: u32, value: i32) {
    hle_write_u32(bus, address, value as u32);
}

/// The integer operation retains the low bits of the truncated f32 value.
fn hle_wrap_float(value: f32, bits: u32) -> u32 {
    debug_assert!(value.abs() < 2_f32.powi(62));
    let whole = value as i64 as u64;
    if bits == 32 { whole as u32 } else { (whole as u32) & ((1 << bits) - 1) }
}

struct HleAffineMatrix {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
}

fn hle_affine_matrix(scale_x_raw: i16, scale_y_raw: i16, angle: u16) -> HleAffineMatrix {
    let scale_x = f32::from(scale_x_raw) / 256.0;
    let scale_y = f32::from(scale_y_raw) / 256.0;
    let angle_index = (angle >> 8) as u8;
    let theta = f32::from(angle_index) / 128.0 * std::f32::consts::PI;
    let sine = theta.sin();
    let cosine = theta.cos();
    HleAffineMatrix {
        a: cosine * scale_x,
        b: -sine * scale_x,
        c: sine * scale_y,
        d: cosine * scale_y,
    }
}

impl Cpu {
    pub fn begin_hle_cpu_set(&mut self, bus: &mut Bus) -> Option<HleBulkWork> {
        let control = self.registers[2];
        let fill = control & (1 << 24) != 0;
        let word = control & (1 << 26) != 0;
        if self.registers[0] < 0x0200_0000 {
            self.registers[3] = 0x170;
            bus.elapse(1);
            return None;
        }

        let alignment = if word { 3 } else { 1 };
        let mut work = HleCpuSetWork {
            source: self.registers[0] & !alignment,
            destination: self.registers[1] & !alignment,
            remaining: control & 0x001F_FFFF,
            fill_value: 0,
            fill,
            word,
        };
        if fill {
            work.fill_value = bus
                .read(access(
                    work.source,
                    if word { AccessWidth::Word } else { AccessWidth::Half },
                    AccessKind::Nonsequential,
                ))
                .value;
        }
        Some(HleBulkWork::CpuSet(work))
    }

    pub fn resume_hle_cpu_set(
        &mut self,
        bus: &mut Bus,
        work: &mut HleCpuSetWork,
        stop_at: u64,
    ) -> bool {
        let width = if work.word { AccessWidth::Word } else { AccessWidth::Half };
        let unit = if work.word { 4_u32 } else { 2 };
        while work.remaining != 0 {
            if bus.now() >= stop_at {
                return false;
            }
            let value = if work.fill {
                work.fill_value
            } else {
                let copied = bus.read(access(work.source, width, AccessKind::Nonsequential)).value;
                work.source = work.source.wrapping_add(unit);
                copied
            };
            bus.write(access(work.destination, width, AccessKind::Nonsequential), value);
            work.destination = work.destination.wrapping_add(unit);
            work.remaining -= 1;
        }
        if work.word {
            let unit_count = self.registers[2] & 0x001F_FFFF;
            self.registers[0] =
                self.registers[0].wrapping_add(if work.fill { 4 } else { unit_count * 4 });
            self.registers[1] = self.registers[1].wrapping_add(unit_count * 4);
        }
        self.registers[3] = 0x170;
        true
    }

    /// Runs fixed-size CpuSet work inside the sound driver's init routine.
    pub fn execute_hle_cpu_set(&mut self, bus: &mut Bus) {
        let Some(HleBulkWork::CpuSet(mut work)) = self.begin_hle_cpu_set(bus) else { return };
        let finished = self.resume_hle_cpu_set(bus, &mut work, u64::MAX);
        debug_assert!(finished);
    }

    pub fn begin_hle_cpu_fast_set(&mut self, bus: &mut Bus) -> Option<HleBulkWork> {
        let control = self.registers[2];
        let requested_words = control & 0x001F_FFFF;
        let source = self.registers[0] & !3;
        if source < 0x0200_0000 {
            self.registers[2] = (self.registers[1] & !3).wrapping_add(requested_words * 4);
            bus.elapse(1);
            return None;
        }
        let fill = control & (1 << 24) != 0;
        Some(HleBulkWork::CpuFastSet(HleCpuFastSetWork {
            source,
            destination: self.registers[1] & !3,
            remaining_blocks: (requested_words + 7) / 8,
            last_word: if fill {
                bus.read(access(source, AccessWidth::Word, AccessKind::Nonsequential)).value
            } else {
                self.registers[3]
            },
            fill,
        }))
    }

    pub fn resume_hle_cpu_fast_set(
        &mut self,
        bus: &mut Bus,
        work: &mut HleCpuFastSetWork,
        stop_at: u64,
    ) -> bool {
        while work.remaining_blocks != 0 {
            if bus.now() >= stop_at {
                return false;
            }
            let mut values = [work.last_word; 8];
            if !work.fill {
                for (index, value) in values.iter_mut().enumerate() {
                    *value = bus
                        .read(access(
                            work.source,
                            AccessWidth::Word,
                            if index == 0 {
                                AccessKind::Nonsequential
                            } else {
                                AccessKind::Sequential
                            },
                        ))
                        .value;
                    work.source = work.source.wrapping_add(4);
                }
                work.last_word = values[0];
            }
            for (index, value) in values.into_iter().enumerate() {
                bus.write(
                    access(
                        work.destination,
                        AccessWidth::Word,
                        if index == 0 { AccessKind::Nonsequential } else { AccessKind::Sequential },
                    ),
                    value,
                );
                work.destination = work.destination.wrapping_add(4);
            }
            work.remaining_blocks -= 1;
        }
        let requested_words = self.registers[2] & 0x001F_FFFF;
        let transferred_words = (requested_words + 7) & !7;
        self.registers[2] = (self.registers[1] & !3).wrapping_add(requested_words * 4);
        if !work.fill {
            self.registers[0] = self.registers[0].wrapping_add(transferred_words * 4);
        }
        self.registers[1] = self.registers[1].wrapping_add(transferred_words * 4);
        self.registers[3] = work.last_word;
        true
    }

    pub fn begin_hle_bg_affine_set(&self, bus: &mut Bus) -> HleBulkWork {
        bus.elapse(45);
        HleBulkWork::BgAffineSet(HleBgAffineSetWork {
            source: self.registers[0],
            destination: self.registers[1],
            remaining: self.registers[2],
        })
    }

    pub fn begin_hle_obj_affine_set(&self, bus: &mut Bus) -> HleBulkWork {
        bus.elapse(45);
        HleBulkWork::ObjAffineSet(HleObjAffineSetWork {
            source: self.registers[0],
            destination: self.registers[1],
            remaining: self.registers[2],
            stride: self.registers[3],
        })
    }
}

fn resume_hle_bg_affine_set(bus: &mut Bus, work: &mut HleBgAffineSetWork, stop_at: u64) -> bool {
    while work.remaining != 0 {
        if bus.now() >= stop_at {
            return false;
        }
        let source = work.source;
        let destination = work.destination;
        let original_x = hle_read_i32(bus, source);
        let original_y = hle_read_i32(bus, source.wrapping_add(4));
        let center_x = hle_read_i16(bus, source.wrapping_add(8));
        let center_y = hle_read_i16(bus, source.wrapping_add(10));
        let scale_x = hle_read_i16(bus, source.wrapping_add(12));
        let scale_y = hle_read_i16(bus, source.wrapping_add(14));
        let angle = hle_read_u16(bus, source.wrapping_add(16));
        let matrix = hle_affine_matrix(scale_x, scale_y, angle);
        hle_write_i16(bus, destination, hle_wrap_float(matrix.a * 256.0, 16) as i16);
        hle_write_i16(
            bus,
            destination.wrapping_add(2),
            hle_wrap_float(matrix.b * 256.0, 16) as i16,
        );
        hle_write_i16(
            bus,
            destination.wrapping_add(4),
            hle_wrap_float(matrix.c * 256.0, 16) as i16,
        );
        hle_write_i16(
            bus,
            destination.wrapping_add(6),
            hle_wrap_float(matrix.d * 256.0, 16) as i16,
        );
        let origin_x = original_x as f32 / 256.0;
        let origin_y = original_y as f32 / 256.0;
        let display_x = f32::from(center_x);
        let display_y = f32::from(center_y);
        let start_x = origin_x - (matrix.a * display_x + matrix.b * display_y);
        let start_y = origin_y - (matrix.c * display_x + matrix.d * display_y);
        hle_write_i32(bus, destination.wrapping_add(8), hle_wrap_float(start_x * 256.0, 32) as i32);
        hle_write_i32(
            bus,
            destination.wrapping_add(12),
            hle_wrap_float(start_y * 256.0, 32) as i32,
        );
        work.source = source.wrapping_add(20);
        work.destination = destination.wrapping_add(16);
        work.remaining -= 1;
    }
    true
}

fn resume_hle_obj_affine_set(bus: &mut Bus, work: &mut HleObjAffineSetWork, stop_at: u64) -> bool {
    let stride = work.stride;
    while work.remaining != 0 {
        if bus.now() >= stop_at {
            return false;
        }
        let source = work.source;
        let destination = work.destination;
        let scale_x = hle_read_i16(bus, source);
        let scale_y = hle_read_i16(bus, source.wrapping_add(2));
        let angle = hle_read_u16(bus, source.wrapping_add(4));
        let matrix = hle_affine_matrix(scale_x, scale_y, angle);
        hle_write_i16(bus, destination, hle_wrap_float(matrix.a * 256.0, 16) as i16);
        hle_write_i16(
            bus,
            destination.wrapping_add(stride),
            hle_wrap_float(matrix.b * 256.0, 16) as i16,
        );
        hle_write_i16(
            bus,
            destination.wrapping_add(stride.wrapping_mul(2)),
            hle_wrap_float(matrix.c * 256.0, 16) as i16,
        );
        hle_write_i16(
            bus,
            destination.wrapping_add(stride.wrapping_mul(3)),
            hle_wrap_float(matrix.d * 256.0, 16) as i16,
        );
        work.source = source.wrapping_add(8);
        work.destination = destination.wrapping_add(stride.wrapping_mul(4));
        work.remaining -= 1;
    }
    true
}

impl Cpu {
    /// `Ok(false)` suspends at a guest-cycle boundary; `Err` gives the first
    /// faulting word (service number, address), before its MMIO write.
    pub fn resume_hle_bulk_work(
        &mut self,
        bus: &mut Bus,
        work: &mut HleBulkWork,
        stop_at: u64,
    ) -> Result<bool, (u8, u32)> {
        match work {
            HleBulkWork::CpuSet(state) => Ok(self.resume_hle_cpu_set(bus, state, stop_at)),
            HleBulkWork::CpuFastSet(state) => Ok(self.resume_hle_cpu_fast_set(bus, state, stop_at)),
            HleBulkWork::BgAffineSet(state) => Ok(resume_hle_bg_affine_set(bus, state, stop_at)),
            HleBulkWork::ObjAffineSet(state) => Ok(resume_hle_obj_affine_set(bus, state, stop_at)),
            HleBulkWork::BitUnpack(state) => {
                self.resume_hle_bit_unpack(bus, state, stop_at).map_err(|address| (0x10, address))
            }
            HleBulkWork::Lz77(state) => Ok(self.resume_hle_lz77(bus, state, stop_at)),
            HleBulkWork::Huffman(state) => {
                self.resume_hle_huffman(bus, state, stop_at).map_err(|address| (0x13, address))
            }
            HleBulkWork::RunLength(state) => Ok(self.resume_hle_run_length(bus, state, stop_at)),
            HleBulkWork::DifferentialFilter(state) => {
                Ok(self.resume_hle_differential_filter(bus, state, stop_at))
            }
        }
    }

    pub fn begin_hle_bit_unpack(&self, bus: &mut Bus) -> Option<HleBulkWork> {
        let info = self.registers[2];
        bus.elapse(20);
        let source_length = u32::from(hle_read_u16(bus, info));
        let source_width = hle_read_u8(bus, info.wrapping_add(2));
        let destination_width = hle_read_u8(bus, info.wrapping_add(3));
        let bias = bus
            .read(access(info.wrapping_add(4), AccessWidth::Word, AccessKind::Nonsequential))
            .value;
        if !matches!(source_width, 1 | 2 | 4 | 8)
            || !matches!(destination_width, 1 | 2 | 4 | 8 | 16 | 32)
        {
            return None;
        }
        Some(HleBulkWork::BitUnpack(HleBitUnpackWork {
            source: self.registers[0],
            destination: self.registers[1],
            source_length,
            bias,
            input: 0,
            output: 0,
            source_width,
            destination_width,
            input_bits_remaining: 0,
            output_bits: 0,
        }))
    }

    /// Faults at the output word, after earlier byte reads and writes.
    pub fn resume_hle_bit_unpack(
        &mut self,
        bus: &mut Bus,
        work: &mut HleBitUnpackWork,
        stop_at: u64,
    ) -> Result<bool, u32> {
        let source_mask = (1_u32 << work.source_width) - 1;
        while work.source_length > 0 || work.input_bits_remaining != 0 {
            if bus.now() >= stop_at {
                return Ok(false);
            }
            bus.elapse(8);
            if work.input_bits_remaining == 0 {
                work.input = u32::from(hle_read_u8(bus, work.source));
                work.input_bits_remaining = 8;
                work.source = work.source.wrapping_add(1);
                work.source_length -= 1;
            }
            let mut scaled = work.input & source_mask;
            work.input >>= work.source_width;
            if scaled != 0 || work.bias & 0x8000_0000 != 0 {
                scaled = scaled.wrapping_add(work.bias & 0x7FFF_FFFF);
            }
            work.input_bits_remaining -= work.source_width;
            work.output |= scaled << work.output_bits;
            work.output_bits += work.destination_width;
            if work.output_bits == 32 {
                if work.destination >> 24 == 4 && work.destination & 3 != 0 {
                    return Err(work.destination);
                }
                hle_write_u32(bus, work.destination, work.output);
                work.output_bits = 0;
                work.output = 0;
                work.destination = work.destination.wrapping_add(4);
            }
        }
        self.registers[0] = work.source;
        self.registers[1] = work.destination;
        Ok(true)
    }

    pub fn begin_hle_lz77(&self, bus: &mut Bus, vram_output: bool) -> HleBulkWork {
        let source = self.registers[0];
        bus.elapse(20);
        let header = bus.read(access(source, AccessWidth::Word, AccessKind::Nonsequential)).value;
        HleBulkWork::Lz77(HleLz77Work {
            source: source.wrapping_add(4),
            destination: self.registers[1],
            remaining: header >> 8,
            pending_halfword: 0,
            block_header: 0,
            blocks_remaining: 0,
            vram_output,
        })
    }

    pub fn resume_hle_lz77(&mut self, bus: &mut Bus, work: &mut HleLz77Work, stop_at: u64) -> bool {
        let vram_output = work.vram_output;
        while work.remaining > 0 {
            if bus.now() >= stop_at {
                return false;
            }
            bus.elapse(14);
            if work.blocks_remaining == 0 {
                work.block_header = hle_read_u8(bus, work.source);
                work.source = work.source.wrapping_add(1);
                work.blocks_remaining = 8;
                continue;
            }
            bus.elapse(18);
            if work.block_header & 0x80 == 0 {
                let byte = hle_read_u8(bus, work.source);
                work.source = work.source.wrapping_add(1);
                write_decompression_byte(
                    bus,
                    work.destination,
                    byte,
                    vram_output,
                    &mut work.pending_halfword,
                );
                work.destination = work.destination.wrapping_add(1);
                work.remaining -= 1;
            } else {
                let first = hle_read_u8(bus, work.source);
                let second = hle_read_u8(bus, work.source.wrapping_add(1));
                work.source = work.source.wrapping_add(2);
                let mut displacement = work
                    .destination
                    .wrapping_sub((u32::from(first & 0x0F) << 8) | u32::from(second))
                    .wrapping_sub(1);
                let mut bytes = (first >> 4) + 3;
                while bytes > 0 && work.remaining > 0 {
                    bus.elapse(10 + 4 * u64::from(vram_output));
                    let byte = read_lz77_byte(bus, displacement, vram_output);
                    write_decompression_byte(
                        bus,
                        work.destination,
                        byte,
                        vram_output,
                        &mut work.pending_halfword,
                    );
                    displacement = displacement.wrapping_add(1);
                    work.destination = work.destination.wrapping_add(1);
                    work.remaining -= 1;
                    bytes -= 1;
                }
            }
            work.block_header <<= 1;
            work.blocks_remaining -= 1;
        }
        self.registers[0] = work.source;
        self.registers[1] = work.destination;
        self.registers[3] = 0;
        true
    }

    pub fn begin_hle_huffman(&self, bus: &mut Bus) -> Option<HleBulkWork> {
        let input_start = self.registers[0] & !3;
        bus.elapse(45);
        let header =
            bus.read(access(input_start, AccessWidth::Word, AccessKind::Nonsequential)).value;
        let mut symbol_bits = (header & 0xF) as u8;
        if symbol_bits == 0 {
            symbol_bits = 8;
        }
        if symbol_bits != 4 && symbol_bits != 8 {
            return None;
        }
        let tree_size = hle_read_u8(bus, input_start.wrapping_add(4));
        let tree_base = input_start.wrapping_add(5);
        Some(HleBulkWork::Huffman(HleHuffmanWork {
            tree_base,
            bitstream_address: input_start
                .wrapping_add(4)
                .wrapping_add((u32::from(tree_size) + 1) * 2),
            destination: self.registers[1],
            remaining: header >> 8,
            node_address: tree_base,
            output: 0,
            node: hle_read_u8(bus, tree_base),
            symbol_bits,
            output_bits: 0,
        }))
    }

    /// Suspend only between bitstream words: one word traverses at most 32 nodes.
    pub fn resume_hle_huffman(
        &mut self,
        bus: &mut Bus,
        work: &mut HleHuffmanWork,
        stop_at: u64,
    ) -> Result<bool, u32> {
        let symbol_mask = (1_u32 << work.symbol_bits) - 1;
        while work.remaining != 0 {
            if bus.now() >= stop_at {
                return Ok(false);
            }
            let aligned_word = bus
                .read(access(
                    work.bitstream_address & !3,
                    AccessWidth::Word,
                    AccessKind::Nonsequential,
                ))
                .value;
            let mut bitstream = aligned_word.rotate_right((work.bitstream_address & 3) * 8);
            work.bitstream_address = work.bitstream_address.wrapping_add(4);
            let mut input_bits = 32;
            while input_bits != 0 && work.remaining != 0 {
                let right = bitstream & 0x8000_0000 != 0;
                bitstream <<= 1;
                let node = work.node;
                let child_base = (work.node_address & !1)
                    .wrapping_add(u32::from(node & 0x3F) * 2)
                    .wrapping_add(2);
                let child_address = child_base.wrapping_add(u32::from(right));
                let terminal = if right { node & 0x40 != 0 } else { node & 0x80 != 0 };
                if !terminal {
                    work.node_address = child_address;
                    work.node = hle_read_u8(bus, child_address);
                    input_bits -= 1;
                    continue;
                }
                let symbol = u32::from(hle_read_u8(bus, child_address)) & symbol_mask;
                work.output |= symbol << work.output_bits;
                work.output_bits += work.symbol_bits;
                work.node_address = work.tree_base;
                work.node = hle_read_u8(bus, work.tree_base);
                if work.output_bits == 32 {
                    if work.destination >> 24 == 4 && work.destination & 3 != 0 {
                        return Err(work.destination);
                    }
                    hle_write_u32(bus, work.destination, work.output);
                    work.destination = work.destination.wrapping_add(4);
                    work.remaining = work.remaining.saturating_sub(4);
                    work.output = 0;
                    work.output_bits = 0;
                }
                input_bits -= 1;
            }
        }
        self.registers[0] = work.bitstream_address;
        self.registers[1] = work.destination;
        Ok(true)
    }

    pub fn begin_hle_run_length(&self, bus: &mut Bus, vram_output: bool) -> HleBulkWork {
        let source = self.registers[0];
        bus.elapse(20);
        let header =
            bus.read(access(source & !3, AccessWidth::Word, AccessKind::Nonsequential)).value;
        let remaining = header >> 8;
        HleBulkWork::RunLength(HleRunLengthWork {
            source: source.wrapping_add(4),
            destination: self.registers[1],
            remaining,
            padding: 4_u32.wrapping_sub(remaining) & 3,
            pending_halfword: 0,
            vram_output,
        })
    }

    pub fn resume_hle_run_length(
        &mut self,
        bus: &mut Bus,
        work: &mut HleRunLengthWork,
        stop_at: u64,
    ) -> bool {
        let vram_output = work.vram_output;
        while work.remaining > 0 {
            if bus.now() >= stop_at {
                return false;
            }
            bus.elapse(12);
            let block_header = hle_read_u8(bus, work.source);
            work.source = work.source.wrapping_add(1);
            let compressed = block_header & 0x80 != 0;
            let mut bytes =
                if compressed { (block_header & 0x7F) + 3 } else { block_header.wrapping_add(1) };
            let mut repeated_byte = 0;
            if compressed {
                repeated_byte = hle_read_u8(bus, work.source);
                work.source = work.source.wrapping_add(1);
            }
            while bytes > 0 && work.remaining > 0 {
                bus.elapse(5 + 4 * u64::from(vram_output));
                let byte = if compressed {
                    repeated_byte
                } else {
                    let literal = hle_read_u8(bus, work.source);
                    work.source = work.source.wrapping_add(1);
                    literal
                };
                write_decompression_byte(
                    bus,
                    work.destination,
                    byte,
                    vram_output,
                    &mut work.pending_halfword,
                );
                work.destination = work.destination.wrapping_add(1);
                work.remaining -= 1;
                bytes -= 1;
            }
        }
        let mut destination = work.destination;
        let mut padding = work.padding;
        if vram_output {
            if destination & 1 != 0 {
                padding = padding.saturating_sub(1);
                destination = destination.wrapping_add(1);
            }
            while padding > 0 {
                bus.write(access(destination, AccessWidth::Half, AccessKind::Nonsequential), 0);
                destination = destination.wrapping_add(2);
                padding = padding.saturating_sub(2);
            }
        } else {
            while padding > 0 {
                bus.write(access(destination, AccessWidth::Byte, AccessKind::Nonsequential), 0);
                destination = destination.wrapping_add(1);
                padding -= 1;
            }
        }
        self.registers[0] = work.source;
        self.registers[1] = destination;
        true
    }

    pub fn begin_hle_differential_filter(
        &self,
        bus: &mut Bus,
        input_halfword: bool,
        output_halfword: bool,
    ) -> HleBulkWork {
        let source = self.registers[0] & !3;
        bus.elapse(20);
        let header = bus.read(access(source, AccessWidth::Word, AccessKind::Nonsequential)).value;
        HleBulkWork::DifferentialFilter(HleDifferentialFilterWork {
            source: source.wrapping_add(4),
            destination: self.registers[1],
            remaining: header >> 8,
            previous: 0,
            pending_halfword: 0,
            input_halfword,
            output_halfword,
        })
    }

    pub fn resume_hle_differential_filter(
        &mut self,
        bus: &mut Bus,
        work: &mut HleDifferentialFilterWork,
        stop_at: u64,
    ) -> bool {
        let input_halfword = work.input_halfword;
        let output_halfword = work.output_halfword;
        while work.remaining > 0 {
            if bus.now() >= stop_at {
                return false;
            }
            bus.elapse(8);
            let delta = if input_halfword {
                hle_read_u16(bus, work.source)
            } else {
                u16::from(hle_read_u8(bus, work.source))
            };
            let value = work.previous.wrapping_add(delta);
            if !input_halfword && output_halfword {
                work.pending_halfword >>= 8;
                work.pending_halfword |= value << 8;
                if work.source & 1 != 0 {
                    bus.write(
                        access(work.destination, AccessWidth::Half, AccessKind::Nonsequential),
                        u32::from(work.pending_halfword),
                    );
                    work.destination = work.destination.wrapping_add(2);
                    work.remaining = work.remaining.saturating_sub(2);
                }
            } else if output_halfword {
                bus.write(
                    access(work.destination, AccessWidth::Half, AccessKind::Nonsequential),
                    u32::from(value),
                );
                work.destination = work.destination.wrapping_add(2);
                work.remaining = work.remaining.saturating_sub(2);
            } else {
                bus.write(
                    access(work.destination, AccessWidth::Byte, AccessKind::Nonsequential),
                    u32::from(value as u8),
                );
                work.destination = work.destination.wrapping_add(1);
                work.remaining -= 1;
            }
            work.previous = value;
            work.source = work.source.wrapping_add(if input_halfword { 2 } else { 1 });
        }
        self.registers[0] = work.source;
        self.registers[1] = work.destination;
        true
    }
}

fn read_lz77_byte(bus: &mut Bus, address: u32, vram_output: bool) -> u8 {
    if !vram_output {
        return hle_read_u8(bus, address);
    }
    let halfword = hle_read_u16(bus, address & !1);
    (halfword >> ((address & 1) * 8)) as u8
}

fn write_decompression_byte(
    bus: &mut Bus,
    address: u32,
    byte: u8,
    vram_output: bool,
    pending_halfword: &mut u16,
) {
    if !vram_output {
        bus.write(access(address, AccessWidth::Byte, AccessKind::Nonsequential), u32::from(byte));
        return;
    }
    if address & 1 == 0 {
        *pending_halfword = u16::from(byte);
        return;
    }
    *pending_halfword |= u16::from(byte) << 8;
    bus.write(
        access(address ^ 1, AccessWidth::Half, AccessKind::Nonsequential),
        u32::from(*pending_halfword),
    );
}
