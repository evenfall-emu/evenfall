//! Register and latched-address state for one GBA DMA channel.

use evenfall_state::Codec;

#[derive(Clone, Codec, Default)]
pub struct Channel {
    pub source: u32,
    pub destination: u32,
    pub word_count: u16,
    pub control: u16,
    pub latched_source: u32,
    pub latched_destination: u32,
    /// The word count the next transfer copies: `word_count` as it was when
    /// the channel was enabled, then as it was when the last repeat ended
    /// (GBATEK, DMA Transfers).
    pub latched_word_count: u16,
}

impl Channel {
    pub fn enabled(&self) -> bool {
        self.control & (1 << 15) != 0
    }
    pub fn start_timing(&self) -> u8 {
        ((self.control >> 12) & 3) as u8
    }
    /// Units the next transfer copies, from the latched count.
    pub fn transfer_words(&self, index: usize) -> u32 {
        let count = self.latched_word_count;
        let masked = if index == 3 { count } else { count & 0x3fff };
        if masked != 0 {
            u32::from(masked)
        } else if index == 3 {
            0x1_0000
        } else {
            0x4000
        }
    }
    pub fn write_source_low(&mut self, value: u16) {
        self.source = (self.source & 0xffff_0000) | u32::from(value);
    }
    pub fn write_source_high(&mut self, index: usize, value: u16) {
        let mask = if index == 0 { 0x07ff } else { 0x0fff };
        self.source = (self.source & 0xffff) | (u32::from(value & mask) << 16);
    }
    pub fn write_destination_low(&mut self, value: u16) {
        self.destination = (self.destination & 0xffff_0000) | u32::from(value);
    }
    pub fn write_destination_high(&mut self, index: usize, value: u16) {
        let mask = if index == 3 { 0x0fff } else { 0x07ff };
        self.destination = (self.destination & 0xffff) | (u32::from(value & mask) << 16);
    }
    /// `DMAxCNT_H` as a read returns it: bit 11, the Game Pak DRQ flag,
    /// exists only on DMA3 and reads as zero on the other channels (the mGBA
    /// suite's I/O read table).
    pub fn read_control(&self, index: usize) -> u16 {
        if index == 3 { self.control } else { self.control & !(1 << 11) }
    }
    pub fn write_control(&mut self, value: u16) -> bool {
        let was_enabled = self.enabled();
        self.control = value & 0xffe0;
        let newly_enabled = !was_enabled && self.enabled();
        if newly_enabled {
            self.latched_source = self.source;
            self.latched_destination = self.destination;
            self.latched_word_count = self.word_count;
        }
        newly_enabled
    }
    fn unit_offset_mask(&self) -> u32 {
        if self.control & (1 << 10) != 0 { 3 } else { 1 }
    }
    pub fn align_latched_addresses(&mut self) {
        let alignment_mask = self.unit_offset_mask();
        self.latched_source = if self.latched_source < 0x0200_0000 {
            0
        } else {
            self.latched_source & !alignment_mask
        };
        self.latched_destination &= !alignment_mask;
    }
    pub fn reload_latched_destination(&mut self) {
        self.latched_destination = self.destination & !self.unit_offset_mask();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dma_register_high_halves_apply_channel_specific_address_widths() {
        let mut channels =
            [Channel::default(), Channel::default(), Channel::default(), Channel::default()];
        channels[0].write_source_low(0xffff);
        channels[0].write_source_high(0, 0xffff);
        channels[1].write_source_high(1, 0xffff);
        channels[2].write_destination_low(0xffff);
        channels[2].write_destination_high(2, 0xffff);
        channels[3].write_destination_low(0xffff);
        channels[3].write_destination_high(3, 0xffff);
        assert_eq!(channels[0].source, 0x07ff_ffff);
        assert_eq!(channels[1].source, 0x0fff_0000);
        assert_eq!(channels[2].destination, 0x07ff_ffff);
        assert_eq!(channels[3].destination, 0x0fff_ffff);
    }
}
