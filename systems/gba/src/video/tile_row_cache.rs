//! Transient content-addressed cache of decoded 4bpp text tile rows.
//! Neither VRAM writes nor state restoration require invalidation hooks.

#[derive(Clone, Copy, Default)]
pub struct Entry {
    source: u32,
    pub indices: [u8; 8],
    pub colors: [u16; 8],
    palette_bank_tag: u8,
    pub fully_opaque: bool,
}

#[derive(Clone)]
pub struct Cache {
    entries: [Entry; 1024],
    palette: [u8; 512],
    palette_words: [u16; 256],
}

impl Default for Cache {
    fn default() -> Self {
        Self { entries: [Entry::default(); 1024], palette: [0; 512], palette_words: [0; 256] }
    }
}

impl Cache {
    /// Validate once per eligible scanline; historical palette draws do not
    /// use expanded colors.
    pub fn prepare_palette(&mut self, palette: &[u8; 512]) {
        if self.palette == *palette {
            return;
        }
        self.palette = *palette;
        for (index, color) in self.palette_words.iter_mut().enumerate() {
            *color = u16::from_le_bytes([palette[index * 2], palette[index * 2 + 1]]) & 0x7FFF;
        }
        for entry in &mut self.entries {
            entry.palette_bank_tag = 0;
        }
    }

    /// The entry stays valid until the next lookup. Zero-filled entries
    /// already decode source zero correctly.
    pub fn row(&mut self, source: u32, palette_bank: u8) -> &Entry {
        let hash =
            source.wrapping_mul(0x9E37_79B1) ^ u32::from(palette_bank).wrapping_mul(0x85EB_CA6B);
        let index = (hash >> 22) as usize;
        let entry = &mut self.entries[index];
        if entry.source != source {
            for (pixel, index) in entry.indices.iter_mut().enumerate() {
                *index = ((source >> (pixel * 4)) & 15) as u8;
            }
            entry.source = source;
            entry.fully_opaque = entry.indices.iter().all(|&index| index != 0);
            entry.palette_bank_tag = 0;
        }
        let bank_tag = palette_bank + 1;
        if entry.palette_bank_tag != bank_tag {
            let bank_start = usize::from(palette_bank) * 16;
            for (pixel, color) in entry.colors.iter_mut().enumerate() {
                *color = self.palette_words[bank_start + usize::from(entry.indices[pixel])];
            }
            entry.palette_bank_tag = bank_tag;
        }
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ZiGBA: "expanded rows validate palette bytes banks and source replacement"
    #[test]
    fn expanded_rows_validate_palette_bytes_banks_and_source_replacement() {
        let mut cache = Cache::default();
        let mut palette = [0; 512];
        for index in 0..256 {
            palette[index * 2..][..2].copy_from_slice(&(0x8000 | index as u16).to_le_bytes());
        }
        cache.prepare_palette(&palette);
        let row = *cache.row(0x8765_4321, 0);
        assert!(row.fully_opaque);
        assert_eq!(cache.row(0x8765_4321, 0).colors, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(cache.row(0x8765_4321, 2).colors, [33, 34, 35, 36, 37, 38, 39, 40]);
        palette[2..4].copy_from_slice(&0x9234u16.to_le_bytes());
        cache.prepare_palette(&palette);
        assert_eq!(cache.row(0x8765_4321, 0).colors[0], 0x1234);
        palette[2..4].copy_from_slice(&0x8001u16.to_le_bytes());
        cache.prepare_palette(&palette);
        assert_eq!(cache.row(0x8765_4321, 0).colors[0], 1);
        for value in 0..4096 {
            let source = value | 0xA234_0000;
            let decoded = *cache.row(source, 3);
            for (pixel, color) in decoded.colors.iter().enumerate() {
                let index = ((source >> (pixel * 4)) & 15) as u16;
                assert_eq!(*color, 48 + index);
            }
            assert_eq!(decoded.fully_opaque, (0..8).all(|pixel| source >> (pixel * 4) & 15 != 0));
        }
        assert_eq!(cache.row(0x8765_4321, 0).colors, [1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
