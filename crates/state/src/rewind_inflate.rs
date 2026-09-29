//! Bounded raw DEFLATE decoding for untrusted rewind bundles (RFC 1951).
use crate::{Error, Result};
const INVALID: Error = Error::InvalidSaveState;

struct Bits<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl Bits<'_> {
    fn take(&mut self, count: u8) -> Result<u16> {
        let mut value = 0u16;
        for i in 0..count {
            let byte = *self.bytes.get(self.position / 8).ok_or(INVALID)?;
            value |= u16::from((byte >> (self.position % 8)) & 1) << i;
            self.position += 1;
        }
        Ok(value)
    }
}

struct Tree {
    counts: [u16; 16],
    first: [u16; 16],
    offsets: [u16; 16],
    symbols: [u16; 288],
}
impl Tree {
    fn new(lengths: &[u8], singleton_or_empty: bool) -> Result<Self> {
        let mut tree =
            Self { counts: [0; 16], first: [0; 16], offsets: [0; 16], symbols: [0; 288] };
        for &len in lengths {
            if len > 15 {
                return Err(INVALID);
            }
            if len != 0 {
                tree.counts[usize::from(len)] += 1;
            }
        }
        let mut code = 0u32;
        let mut total = 0u16;
        for len in 1..16 {
            code = (code + u32::from(tree.counts[len - 1])) * 2;
            if code + u32::from(tree.counts[len]) > 1 << len {
                return Err(INVALID);
            }
            tree.first[len] = code as u16;
            tree.offsets[len] = total;
            total += tree.counts[len];
        }
        if code + u32::from(tree.counts[15]) != 32768
            && !(singleton_or_empty && (total == 0 || (total == 1 && tree.counts[1] == 1)))
        {
            return Err(INVALID);
        }
        let mut slots = tree.offsets;
        for (value, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            tree.symbols[usize::from(slots[usize::from(len)])] = value as u16;
            slots[usize::from(len)] += 1;
        }
        Ok(tree)
    }
    fn symbol(&self, bits: &mut Bits<'_>) -> Result<u16> {
        let mut code = 0u16;
        for len in 1..16 {
            code = code * 2 + bits.take(1)?;
            let first = self.first[len];
            if code >= first && code - first < self.counts[len] {
                return Ok(self.symbols[usize::from(self.offsets[len] + code - first)]);
            }
        }
        Err(INVALID)
    }
}

fn dynamic(bits: &mut Bits<'_>) -> Result<(Tree, Tree)> {
    let literals = usize::from(bits.take(5)?) + 257;
    let distances = usize::from(bits.take(5)?) + 1;
    let codes = usize::from(bits.take(4)?) + 4;
    if literals > 286 {
        return Err(INVALID);
    }
    let order = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut code_lengths = [0u8; 19];
    for &index in &order[..codes] {
        code_lengths[index] = bits.take(3)? as u8;
    }
    let code_tree = Tree::new(&code_lengths, false)?;
    let mut lengths = [0u8; 286 + 32];
    let count = literals + distances;
    let mut i = 0;
    while i < count {
        let value = code_tree.symbol(bits)?;
        if value < 16 {
            lengths[i] = value as u8;
            i += 1;
            continue;
        }
        let repeated = if value == 16 {
            if i == 0 {
                return Err(INVALID);
            }
            lengths[i - 1]
        } else {
            0
        };
        let run = match value {
            16 => usize::from(bits.take(2)?) + 3,
            17 => usize::from(bits.take(3)?) + 3,
            18 => usize::from(bits.take(7)?) + 11,
            _ => return Err(INVALID),
        };
        if run > count - i {
            return Err(INVALID);
        }
        lengths[i..i + run].fill(repeated);
        i += run;
    }
    if lengths[256] == 0 {
        return Err(INVALID);
    }
    Ok((Tree::new(&lengths[..literals], true)?, Tree::new(&lengths[literals..count], true)?))
}

/// A single final block must produce exactly `output.len` bytes and consume all input.
pub fn decode(input: &[u8], output: &mut [u8]) -> Result<()> {
    if input.len() > usize::MAX / 8 {
        return Err(INVALID);
    }
    let mut bits = Bits { bytes: input, position: 0 };
    let mut written = 0;
    let mut final_block = false;
    while !final_block {
        final_block = bits.take(1)? != 0;
        let kind = bits.take(2)?;
        if kind == 0 {
            bits.position = bits.position.div_ceil(8) * 8;
            let len = usize::from(bits.take(16)?);
            if bits.take(16)? != !(len as u16) {
                return Err(INVALID);
            }
            let start = bits.position / 8;
            if len > output.len() - written || len > input.len() - start {
                return Err(INVALID);
            }
            output[written..written + len].copy_from_slice(&input[start..start + len]);
            written += len;
            bits.position += len * 8;
            continue;
        }
        let (literal, distance) = match kind {
            1 => {
                let mut lengths = [0; 288];
                lengths[..144].fill(8);
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                lengths[280..].fill(8);
                (Tree::new(&lengths, false)?, Tree::new(&[5; 32], false)?)
            }
            2 => dynamic(&mut bits)?,
            _ => return Err(INVALID),
        };
        loop {
            let symbol = literal.symbol(&mut bits)?;
            if symbol == 256 {
                break;
            }
            if symbol < 256 {
                if written == output.len() {
                    return Err(INVALID);
                }
                output[written] = symbol as u8;
                written += 1;
                continue;
            }
            if symbol > 285 {
                return Err(INVALID);
            }
            let bases = [
                3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83,
                99, 115, 131, 163, 195, 227, 258,
            ];
            let extras = [
                0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5,
                0,
            ];
            let idx = usize::from(symbol - 257);
            let len = bases[idx] + usize::from(bits.take(extras[idx])?);
            let dist_symbol = distance.symbol(&mut bits)?;
            if dist_symbol >= 30 {
                return Err(INVALID);
            }
            let extra = if dist_symbol < 4 { 0 } else { (dist_symbol / 2 - 1) as u8 };
            let base = if dist_symbol < 4 {
                usize::from(dist_symbol) + 1
            } else {
                (2 + usize::from(dist_symbol % 2)) * (1usize << extra) + 1
            };
            let dist = base + usize::from(bits.take(extra)?);
            if dist > written || len > output.len() - written {
                return Err(INVALID);
            }
            for _ in 0..len {
                output[written] = output[written - dist];
                written += 1;
            }
        }
    }
    if written != output.len() || bits.position.div_ceil(8) != input.len() {
        return Err(INVALID);
    }
    Ok(())
}
