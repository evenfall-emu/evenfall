//! Transient content-addressed cache of decoded 4bpp text tile rows and the
//! decoded palette words. Neither VRAM writes nor state restoration require
//! invalidation hooks: rows are keyed by their source bytes and palette bank,
//! and the palette copy is compared on every use.

/// Bit 15 of a decoded row color marks a transparent pixel.
pub const TRANSPARENT: u16 = 0x8000;

#[derive(Clone, Copy, Default)]
pub struct Entry {
    source: u32,
    palette_bank_tag: u8,
    pub colors: [u16; 8],
}

#[derive(Clone)]
pub struct Cache {
    entries: [Entry; 1024],
    palette: [u8; 1024],
    /// RGB555 words of the BG (0..256) and OBJ (256..512) palettes.
    pub words: [u16; 512],
    /// BG words for 256-color pixels: index zero is transparent.
    pub bg256: [u16; 256],
}

impl Default for Cache {
    fn default() -> Self {
        let mut bg256 = [0; 256];
        bg256[0] = TRANSPARENT;
        Self { entries: [Entry::default(); 1024], palette: [0; 1024], words: [0; 512], bg256 }
    }
}

impl Cache {
    /// Validate once per scanline that reads the live palette; historical
    /// palette draws do not use decoded colors.
    pub fn prepare_palette(&mut self, palette: &[u8; 1024]) {
        if self.palette == *palette {
            return;
        }
        let banks_changed = self.palette[..512] != palette[..512];
        self.palette = *palette;
        for (index, color) in self.words.iter_mut().enumerate() {
            *color = u16::from_le_bytes([palette[index * 2], palette[index * 2 + 1]]) & 0x7FFF;
        }
        self.bg256.copy_from_slice(&self.words[..256]);
        self.bg256[0] = TRANSPARENT;
        if banks_changed {
            for entry in &mut self.entries {
                entry.palette_bank_tag = 0;
            }
        }
    }

    /// Colors of one 4bpp row in display order: pixel `i` is nibble `i` of
    /// `source`, index zero is [`TRANSPARENT`]. The entry stays valid until
    /// the next lookup.
    #[inline]
    pub fn row(&mut self, source: u32, palette_bank: u8) -> &[u16; 8] {
        let hash =
            source.wrapping_mul(0x9E37_79B1) ^ u32::from(palette_bank).wrapping_mul(0x85EB_CA6B);
        let words = &self.words;
        let entry = &mut self.entries[(hash >> 22) as usize];
        let bank_tag = palette_bank + 1;
        if entry.source != source || entry.palette_bank_tag != bank_tag {
            let bank = &words[usize::from(palette_bank) * 16..][..16];
            for (pixel, color) in entry.colors.iter_mut().enumerate() {
                let index = ((source >> (pixel * 4)) & 15) as usize;
                *color = if index == 0 { TRANSPARENT } else { bank[index] };
            }
            entry.source = source;
            entry.palette_bank_tag = bank_tag;
        }
        &entry.colors
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

    /// ZiGBA: "expanded rows validate palette bytes banks and source replacement"
    #[test]
    fn expanded_rows_validate_palette_bytes_banks_and_source_replacement() {
        let mut cache = Cache::default();
        let mut palette = [0; 1024];
        for index in 0..256 {
            palette[index * 2..][..2].copy_from_slice(&(0x8000 | index as u16).to_le_bytes());
        }
        cache.prepare_palette(&palette);
        assert_eq!(*cache.row(0x8765_4321, 0), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(*cache.row(0x8765_4321, 2), [33, 34, 35, 36, 37, 38, 39, 40]);
        palette[2..4].copy_from_slice(&0x9234u16.to_le_bytes());
        cache.prepare_palette(&palette);
        assert_eq!(cache.row(0x8765_4321, 0)[0], 0x1234);
        palette[2..4].copy_from_slice(&0x8001u16.to_le_bytes());
        cache.prepare_palette(&palette);
        assert_eq!(cache.row(0x8765_4321, 0)[0], 1);
        // An OBJ palette change keeps BG rows and updates the OBJ words.
        palette[512 + 6..512 + 8].copy_from_slice(&0x7C1Fu16.to_le_bytes());
        cache.prepare_palette(&palette);
        assert_eq!(cache.words[256 + 3], 0x7C1F);
        assert_eq!(cache.row(0x8765_4321, 0)[0], 1);
        for value in 0..4096 {
            let source = value | 0xA234_0000;
            let decoded = *cache.row(source, 3);
            for (pixel, color) in decoded.iter().enumerate() {
                let index = ((source >> (pixel * 4)) & 15) as u16;
                assert_eq!(*color, if index == 0 { TRANSPARENT } else { 48 + index });
            }
        }
        assert_eq!(*cache.row(0x8765_4321, 0), [1, 2, 3, 4, 5, 6, 7, 8]);
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
