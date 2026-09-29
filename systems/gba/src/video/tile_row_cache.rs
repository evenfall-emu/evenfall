//! Transient decoded palette for tile rows: RGB555 words, 256-color BG
//! words with a transparent index zero, and per-bank tables that decode one
//! 4bpp byte into two colors. The palette copy is compared on every use, so
//! neither palette writes nor state restoration need invalidation hooks.

/// Bit 15 of a decoded row color marks a transparent pixel.
pub const TRANSPARENT: u16 = 0x8000;

#[derive(Clone)]
pub struct Cache {
    palette: [u8; 1024],
    /// RGB555 words of the BG (0..256) and OBJ (256..512) palettes.
    pub words: [u16; 512],
    /// BG words for 256-color pixels: index zero is transparent.
    pub bg256: [u16; 256],
    /// Per 16-color bank, the two colors of each 4bpp byte, built on first
    /// use after a BG palette change.
    pairs: [[u32; 256]; 16],
    pair_banks: u16,
}

impl Default for Cache {
    fn default() -> Self {
        let mut bg256 = [0; 256];
        bg256[0] = TRANSPARENT;
        Self { palette: [0; 1024], words: [0; 512], bg256, pairs: [[0; 256]; 16], pair_banks: 0 }
    }
}

impl Cache {
    /// Validate once per scanline that reads the live palette; historical
    /// palette draws do not use decoded colors.
    pub fn prepare_palette(&mut self, palette: &[u8; 1024]) {
        if self.palette == *palette {
            return;
        }
        if self.palette[..512] != palette[..512] {
            self.pair_banks = 0;
        }
        self.palette = *palette;
        for (index, color) in self.words.iter_mut().enumerate() {
            *color = u16::from_le_bytes([palette[index * 2], palette[index * 2 + 1]]) & 0x7FFF;
        }
        self.bg256.copy_from_slice(&self.words[..256]);
        self.bg256[0] = TRANSPARENT;
    }

    /// Two-pixel colors of a 16-color bank: entry `b` holds the colors of
    /// byte `b`'s low nibble (bits 0–15, the left pixel) and high nibble
    /// (bits 16–31); index zero is [`TRANSPARENT`].
    #[inline]
    pub fn pairs(&mut self, palette_bank: u8) -> &[u32; 256] {
        let bank = usize::from(palette_bank & 15);
        if self.pair_banks >> bank & 1 == 0 {
            let words = &self.words[bank * 16..][..16];
            let color =
                |index: usize| u32::from(if index == 0 { TRANSPARENT } else { words[index] });
            for (byte, pair) in self.pairs[bank].iter_mut().enumerate() {
                *pair = color(byte & 15) | color(byte >> 4) << 16;
            }
            self.pair_banks |= 1 << bank;
        }
        &self.pairs[bank]
    }

    /// Colors of one 4bpp row in display order: pixel `i` is nibble `i` of
    /// `source`.
    #[inline]
    pub fn row(&mut self, source: u32, palette_bank: u8, pixels: &mut [u16; 8]) {
        let pairs = self.pairs(palette_bank);
        for (pair, byte) in pixels.as_chunks_mut::<2>().0.iter_mut().zip(source.to_le_bytes()) {
            let colors = pairs[usize::from(byte)];
            pair[0] = colors as u16;
            pair[1] = (colors >> 16) as u16;
        }
    }
}

/// Reverse the eight 4-bit pixels of a 4bpp row (horizontal flip).
#[inline]
pub fn reverse_nibbles(source: u32) -> u32 {
    let bytes = source.swap_bytes();
    ((bytes >> 4) & 0x0F0F_0F0F) | ((bytes & 0x0F0F_0F0F) << 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cache: &mut Cache, source: u32, bank: u8) -> [u16; 8] {
        let mut pixels = [0; 8];
        cache.row(source, bank, &mut pixels);
        pixels
    }

    /// ZiGBA: "expanded rows validate palette bytes banks and source replacement"
    #[test]
    fn expanded_rows_validate_palette_bytes_banks_and_source_replacement() {
        let mut cache = Cache::default();
        let mut palette = [0; 1024];
        for index in 0..256 {
            palette[index * 2..][..2].copy_from_slice(&(0x8000 | index as u16).to_le_bytes());
        }
        cache.prepare_palette(&palette);
        assert_eq!(row(&mut cache, 0x8765_4321, 0), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(row(&mut cache, 0x8765_4321, 2), [33, 34, 35, 36, 37, 38, 39, 40]);
        palette[2..4].copy_from_slice(&0x9234u16.to_le_bytes());
        cache.prepare_palette(&palette);
        assert_eq!(row(&mut cache, 0x8765_4321, 0)[0], 0x1234);
        palette[2..4].copy_from_slice(&0x8001u16.to_le_bytes());
        cache.prepare_palette(&palette);
        assert_eq!(row(&mut cache, 0x8765_4321, 0)[0], 1);
        // An OBJ palette change keeps BG rows and updates the OBJ words.
        palette[512 + 6..512 + 8].copy_from_slice(&0x7C1Fu16.to_le_bytes());
        cache.prepare_palette(&palette);
        assert_eq!(cache.words[256 + 3], 0x7C1F);
        assert_eq!(row(&mut cache, 0x8765_4321, 0)[0], 1);
        for value in 0..4096 {
            let source = value | 0xA234_0000;
            let decoded = row(&mut cache, source, 3);
            for (pixel, color) in decoded.iter().enumerate() {
                let index = ((source >> (pixel * 4)) & 15) as u16;
                assert_eq!(*color, if index == 0 { TRANSPARENT } else { 48 + index });
            }
        }
        assert_eq!(row(&mut cache, 0x8765_4321, 0), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(cache.bg256[0], TRANSPARENT);
        assert_eq!(cache.bg256[17], 17);
    }

    #[test]
    fn reversed_nibbles_mirror_the_row() {
        assert_eq!(reverse_nibbles(0x8765_4321), 0x1234_5678);
        assert_eq!(reverse_nibbles(0xF000_000A), 0xA000_000F);
        assert_eq!(reverse_nibbles(reverse_nibbles(0x0BAD_F00D)), 0x0BAD_F00D);
    }
}
