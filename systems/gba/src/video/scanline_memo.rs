//! Transient memo of composed lines. Write marks invalidate only lines that
//! read the changed memory; check mode recomposes held lines in the renderer.

pub const PALETTE_SIZE: usize = 1024;
pub const VRAM_SIZE: usize = 96 * 1024;
pub const OAM_SIZE: usize = 1024;
pub const PAGE_SIZE: usize = 1024;
pub const ALL_VRAM_PAGES: u128 = (1 << (VRAM_SIZE / PAGE_SIZE)) - 1;
pub const CHECK: bool = cfg!(any(debug_assertions, test, feature = "scanline-memo-check"));

pub struct Memory<'a> {
    pub palette: &'a [u8; PALETTE_SIZE],
    pub vram: &'a [u8; VRAM_SIZE],
    pub oam: &'a [u8; OAM_SIZE],
}

#[derive(Clone, Copy)]
pub struct ObjectRows {
    pub top: u8,
    pub height: u16,
}

#[derive(Clone, Copy, Default)]
pub struct Reads {
    pub vram_pages: u128,
    pub oam_matrices: bool,
}

impl Reads {
    pub fn add_vram(&mut self, offset: usize) {
        self.vram_pages |= Self::page_bit(offset);
    }

    /// A 1 KiB page holds 32 complete 4bpp tiles or 16 8bpp tiles.
    pub fn add_tile_rows(
        &mut self,
        cache: &mut TilePages,
        char_base: usize,
        color_256: bool,
        entries: [u16; 32],
    ) {
        let source = ((char_base / PAGE_SIZE) | (usize::from(color_256) << 7)) as u8;
        if cache.source != source || cache.entries != entries {
            let mut pages = 0;
            for entry in entries {
                let tile = usize::from(entry & 0x3FF);
                let page = char_base / PAGE_SIZE + (tile >> if color_256 { 4 } else { 5 });
                pages |= 1u128 << page;
            }
            *cache = TilePages { entries, source, pages };
        }
        self.vram_pages |= cache.pages;
    }

    fn page_bit(offset: usize) -> u128 {
        1u128 << (offset / PAGE_SIZE).min(127)
    }
}

#[derive(Clone, Copy)]
pub struct TilePages {
    entries: [u16; 32],
    source: u8,
    pages: u128,
}

impl Default for TilePages {
    fn default() -> Self {
        Self { entries: [0; 32], source: 0xFF, pages: 0 }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Changes {
    registers: bool,
    memory: bool,
    palette: bool,
    vram_pages: u128,
    oam: bool,
}

impl Changes {
    fn unknown() -> Self {
        Self { registers: true, memory: true, oam: true, ..Self::default() }
    }

    #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
    fn add(&mut self, other: Self) {
        self.registers |= other.registers;
        self.memory |= other.memory;
        self.palette |= other.palette;
        self.vram_pages |= other.vram_pages;
        self.oam |= other.oam;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UnmarkedChange {
    Palette,
    VramPage(usize),
    Oam,
}

#[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
#[derive(Clone)]
struct Verification {
    palette: [u8; PALETTE_SIZE],
    vram: [u8; VRAM_SIZE],
    oam: [u8; OAM_SIZE],
    marks: Changes,
    snapshot_valid: bool,
    previous_line: usize,
}

#[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
impl Default for Verification {
    fn default() -> Self {
        Self {
            palette: [0; PALETTE_SIZE],
            vram: [0; VRAM_SIZE],
            oam: [0; OAM_SIZE],
            marks: Changes::default(),
            snapshot_valid: false,
            previous_line: usize::MAX,
        }
    }
}

/// `R` must have a unique byte representation (no uninitialized padding).
/// The Bus's scanline register key uses `#[repr(C)]` with fields in Zig order.
#[derive(Clone)]
pub struct Memo<R: Copy + PartialEq + Default, const L: usize> {
    pub changes_registers: bool,
    changes: Changes,
    valid: [bool; L],
    matrix_readers: [bool; L],
    vram_pages: [u128; L],
    line_registers: [R; L],
    line_versions: [u64; L],
    pub registers: R,
    version: u64,
    oam: [u8; OAM_SIZE],
    pub reads: Reads,
    pub tile_pages: [[TilePages; 2]; 4],
    pub kept: u64,
    #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
    verification: Verification,
}

impl<R: Copy + PartialEq + Default, const L: usize> Default for Memo<R, L> {
    fn default() -> Self {
        Self {
            changes_registers: true,
            changes: Changes::unknown(),
            valid: [false; L],
            matrix_readers: [false; L],
            vram_pages: [0; L],
            line_registers: [R::default(); L],
            line_versions: [0; L],
            registers: R::default(),
            version: 1,
            oam: [0; OAM_SIZE],
            reads: Reads::default(),
            tile_pages: [[TilePages::default(); 2]; 4],
            kept: 0,
            #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
            verification: Verification::default(),
        }
    }
}

impl<R: Copy + PartialEq + Default, const L: usize> Memo<R, L> {
    /// Whether both memos hold the same unrendered changes.
    #[cfg(test)]
    pub fn same_changes(&self, other: &Self) -> bool {
        self.changes == other.changes && self.changes_registers == other.changes_registers
    }

    pub fn mark_registers(&mut self) {
        self.changes_registers = true;
    }

    pub fn mark_palette(&mut self) {
        self.changes.palette = true;
        self.changes.memory = true;
    }

    pub fn mark_vram(&mut self, offset: usize) {
        self.changes.vram_pages |= Reads::page_bit(offset);
        self.changes.memory = true;
    }

    pub fn mark_all_vram(&mut self) {
        self.changes.vram_pages = ALL_VRAM_PAGES;
        self.changes.memory = true;
    }

    pub fn mark_oam(&mut self) {
        self.changes.oam = true;
        self.changes.memory = true;
    }

    pub fn set_registers(&mut self, registers: R) {
        self.changes_registers = false;
        if self.registers == registers {
            return;
        }
        self.registers = registers;
        self.version += 1;
    }

    pub fn verify_registers(&self, registers: R) {
        assert!(
            self.registers == registers,
            "scanline memo: LCD register changed without mark_registers"
        );
    }

    pub fn observe(
        &mut self,
        line: usize,
        memory: Memory<'_>,
        object_rows: fn(u16, u16) -> Option<ObjectRows>,
    ) {
        #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
        let snapshot = self.verify_marks(line, &memory);
        #[cfg(not(any(debug_assertions, test, feature = "scanline-memo-check")))]
        let _ = line;
        if self.changes.memory {
            self.apply_memory_changes(memory.oam, object_rows);
        }
        #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
        if snapshot {
            self.verification.marks = Changes::default();
        }
        self.reads = Reads::default();
    }

    fn apply_memory_changes(
        &mut self,
        oam: &[u8; OAM_SIZE],
        object_rows: fn(u16, u16) -> Option<ObjectRows>,
    ) {
        let changes = std::mem::take(&mut self.changes);
        self.changes.registers = self.changes_registers;
        #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
        self.verification.marks.add(changes);
        let mut stale = [false; L];
        if changes.oam && self.oam != *oam {
            for object in 0..OAM_SIZE / 8 {
                let first = object * 8;
                let kept = &self.oam[first..first + 8];
                let live = &oam[first..first + 8];
                if kept == live {
                    continue;
                }
                if kept[..6] != live[..6] {
                    Self::mark_rows(&mut stale, kept, object_rows);
                    Self::mark_rows(&mut stale, live, object_rows);
                }
                if kept[6..8] != live[6..8] {
                    for (stale, reader) in stale.iter_mut().zip(self.matrix_readers) {
                        *stale |= reader;
                    }
                }
            }
            self.oam.copy_from_slice(oam);
        }
        if changes.palette {
            stale.fill(true);
        } else if changes.vram_pages != 0 {
            for (stale, pages) in stale.iter_mut().zip(self.vram_pages) {
                *stale |= pages & changes.vram_pages != 0;
            }
        }
        for (valid, stale) in self.valid.iter_mut().zip(stale) {
            *valid &= !stale;
        }
    }

    fn mark_rows(
        stale: &mut [bool; L],
        entry: &[u8],
        object_rows: fn(u16, u16) -> Option<ObjectRows>,
    ) {
        let attr0 = u16::from_le_bytes([entry[0], entry[1]]);
        let attr1 = u16::from_le_bytes([entry[2], entry[3]]);
        if let Some(rows) = object_rows(attr0, attr1) {
            for row in 0..usize::from(rows.height) {
                let line = (usize::from(rows.top) + row) & 255;
                if line < L {
                    stale[line] = true;
                }
            }
        }
    }

    pub fn holds(&mut self, line: usize) -> bool {
        if !self.valid[line] {
            return false;
        }
        if self.line_versions[line] == self.version {
            return true;
        }
        if self.line_registers[line] != self.registers {
            return false;
        }
        self.line_versions[line] = self.version;
        true
    }

    /// Records the line as rendered from what it read: `reads` and the
    /// background `pages` the compositor returned. (Those are passed
    /// here rather than merged into the reads, where two halves written
    /// just before this reads them whole would stall store forwarding.)
    pub fn store(&mut self, line: usize, pages: u128) {
        self.valid[line] = true;
        self.vram_pages[line] = self.reads.vram_pages | pages;
        self.matrix_readers[line] = self.reads.oam_matrices;
        if self.line_versions[line] != self.version {
            self.line_registers[line] = self.registers;
            self.line_versions[line] = self.version;
        }
    }

    pub fn forget(&mut self, line: usize) {
        self.valid[line] = false;
    }

    pub fn invalidate(&mut self) {
        self.valid.fill(false);
        self.changes = Changes::unknown();
        self.changes_registers = true;
        #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
        {
            self.verification.snapshot_valid = false;
        }
    }

    #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
    fn verify_marks(&mut self, line: usize, memory: &Memory<'_>) -> bool {
        let previous = std::mem::replace(&mut self.verification.previous_line, line);
        if line > previous {
            return false;
        }
        match self.unmarked_change(memory) {
            Some(UnmarkedChange::Palette) => {
                panic!("scanline memo: palette changed without mark_palette")
            }
            Some(UnmarkedChange::VramPage(page)) => {
                panic!("scanline memo: VRAM page {page} changed without mark_vram")
            }
            Some(UnmarkedChange::Oam) => panic!("scanline memo: OAM changed without mark_oam"),
            None => {}
        }
        self.verification.palette.copy_from_slice(memory.palette);
        self.verification.vram.copy_from_slice(memory.vram);
        self.verification.oam.copy_from_slice(memory.oam);
        self.verification.marks = Changes::default();
        self.verification.snapshot_valid = true;
        true
    }

    #[cfg(any(debug_assertions, test, feature = "scanline-memo-check"))]
    pub fn unmarked_change(&self, memory: &Memory<'_>) -> Option<UnmarkedChange> {
        let verification = &self.verification;
        if !verification.snapshot_valid {
            return None;
        }
        let mut marks = verification.marks;
        marks.add(self.changes);
        if !marks.palette && verification.palette != *memory.palette {
            return Some(UnmarkedChange::Palette);
        }
        for page in 0..VRAM_SIZE / PAGE_SIZE {
            if marks.vram_pages >> page & 1 == 0 {
                let range = page * PAGE_SIZE..(page + 1) * PAGE_SIZE;
                if verification.vram[range.clone()] != memory.vram[range] {
                    return Some(UnmarkedChange::VramPage(page));
                }
            }
        }
        if !marks.oam && verification.oam != *memory.oam {
            return Some(UnmarkedChange::Oam);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Default, PartialEq, Eq)]
    struct Registers {
        control: u16,
        offsets: [u16; 3],
    }

    fn object_rows(attr0: u16, attr1: u16) -> Option<ObjectRows> {
        (attr1 != 0).then_some(ObjectRows { top: attr0 as u8, height: attr1 })
    }

    struct Fixture {
        memo: Box<Memo<Registers, 4>>,
        palette: Box<[u8; PALETTE_SIZE]>,
        vram: Box<[u8; VRAM_SIZE]>,
        oam: Box<[u8; OAM_SIZE]>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                memo: Box::new(Memo::default()),
                palette: Box::new([0; PALETTE_SIZE]),
                vram: Box::new([0; VRAM_SIZE]),
                oam: Box::new([0; OAM_SIZE]),
            }
        }

        fn memory(&self) -> Memory<'_> {
            Memory { palette: &self.palette, vram: &self.vram, oam: &self.oam }
        }

        fn line(&mut self, line: usize, registers: Registers, reads: Reads) -> bool {
            if self.memo.changes_registers {
                self.memo.set_registers(registers);
            } else {
                self.memo.verify_registers(registers);
            }
            self.memo.observe(
                line,
                Memory { palette: &self.palette, vram: &self.vram, oam: &self.oam },
                object_rows,
            );
            if self.memo.holds(line) {
                true
            } else {
                self.memo.reads = reads;
                self.memo.store(line, 0);
                false
            }
        }

        fn frame(&mut self, registers: Registers, reads: [Reads; 4]) -> [bool; 4] {
            std::array::from_fn(|line| self.line(line, registers, reads[line]))
        }

        fn write_vram(&mut self, offset: usize, value: u8) {
            if self.vram[offset] != value {
                self.memo.mark_vram(offset);
            }
            self.vram[offset] = value;
        }

        fn write_oam(&mut self, offset: usize, value: u16) {
            self.oam[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            self.memo.mark_oam();
        }
    }

    /// ZiGBA: "marked memory changes invalidate the lines that read them"
    #[test]
    fn marked_memory_changes_invalidate_the_lines_that_read_them() {
        let mut fixture = Fixture::new();
        let registers = Registers { control: 1, offsets: [2, 3, 4] };
        let mut reads = [Reads::default(); 4];
        reads[0].add_vram(0);
        reads[1].add_vram(VRAM_SIZE - 1);
        let mut entries = [64 | (1 << 10) | (1 << 11) | (5 << 12); 32];
        entries[7] = 1023;
        let mut cache = TilePages::default();
        reads[2].add_tile_rows(&mut cache, 0x4000, false, entries);
        reads[3].oam_matrices = true;
        for (char_base, pages) in
            [(0x4000, 1u128 << 17), (0xC000, 1u128 << 49), (0xC000, 1u128 << 49)]
        {
            let mut wide_reads = Reads::default();
            wide_reads.add_tile_rows(&mut cache, char_base, true, [17; 32]);
            assert_eq!(wide_reads.vram_pages, pages);
        }
        let mut cached = Reads::default();
        cached.add_tile_rows(&mut cache, 0xC000, false, [17; 32]);
        assert_eq!(cached.vram_pages, 1u128 << 48);
        assert_eq!(fixture.frame(registers, reads), [false; 4]);
        assert_eq!(fixture.frame(registers, reads), [true; 4]);
        for value in [0x5A, 0] {
            fixture.write_vram(18 * PAGE_SIZE + 1000, value);
            assert_eq!(fixture.frame(registers, reads), [true, true, false, true]);
            fixture.write_vram(47 * PAGE_SIZE, value);
            assert_eq!(fixture.frame(registers, reads), [true, true, false, true]);
            fixture.write_vram(VRAM_SIZE - 2, value);
            assert_eq!(fixture.frame(registers, reads), [true, false, true, true]);
            fixture.write_vram(5 * PAGE_SIZE, value);
            fixture.write_vram(0, 0);
            assert_eq!(fixture.frame(registers, reads), [true; 4]);
        }
        fixture.palette[PALETTE_SIZE - 1] = 1;
        fixture.memo.mark_palette();
        assert_eq!(fixture.frame(registers, reads), [false; 4]);
        fixture.write_oam(8 * 5 + 6, 0x100);
        assert_eq!(fixture.frame(registers, reads), [true, true, true, false]);
        fixture.write_oam(8 * 5 + 6, 0x100);
        assert_eq!(fixture.frame(registers, reads), [true; 4]);
    }

    /// ZiGBA: "check mode finds video memory changed without a mark"
    #[test]
    fn check_mode_finds_video_memory_changed_without_a_mark() {
        let mut fixture = Fixture::new();
        let registers = Registers::default();
        let reads = [Reads::default(); 4];
        fixture.frame(registers, reads);
        assert_eq!(fixture.memo.unmarked_change(&fixture.memory()), None);
        fixture.vram[40 * PAGE_SIZE + 3] = 1;
        assert_eq!(
            fixture.memo.unmarked_change(&fixture.memory()),
            Some(UnmarkedChange::VramPage(40))
        );
        fixture.memo.mark_vram(41 * PAGE_SIZE);
        assert_eq!(
            fixture.memo.unmarked_change(&fixture.memory()),
            Some(UnmarkedChange::VramPage(40))
        );
        fixture.memo.mark_vram(40 * PAGE_SIZE);
        assert_eq!(fixture.memo.unmarked_change(&fixture.memory()), None);
        fixture.line(1, registers, reads[1]);
        assert_eq!(fixture.memo.unmarked_change(&fixture.memory()), None);
        fixture.oam[8 * 9 + 7] = 1;
        assert_eq!(fixture.memo.unmarked_change(&fixture.memory()), Some(UnmarkedChange::Oam));
        fixture.palette[3] = 1;
        assert_eq!(fixture.memo.unmarked_change(&fixture.memory()), Some(UnmarkedChange::Palette));
        fixture.memo.invalidate();
        assert_eq!(fixture.memo.unmarked_change(&fixture.memory()), None);
        fixture.frame(registers, reads);
        assert_eq!(fixture.memo.unmarked_change(&fixture.memory()), None);
    }

    /// ZiGBA: "check mode does not let marks made before its snapshot cover later changes"
    #[test]
    fn check_mode_does_not_let_marks_made_before_its_snapshot_cover_later_changes() {
        let mut fixture = Fixture::new();
        let registers = Registers::default();
        let reads = [Reads::default(); 4];
        fixture.frame(registers, reads);
        fixture.write_vram(40 * PAGE_SIZE, 1);
        fixture.frame(registers, reads);
        fixture.vram[40 * PAGE_SIZE + 7] = 1;
        assert_eq!(
            fixture.memo.unmarked_change(&fixture.memory()),
            Some(UnmarkedChange::VramPage(40))
        );
    }
}
