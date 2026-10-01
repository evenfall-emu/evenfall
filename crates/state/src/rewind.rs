//! Rewind history: states as copy/literal deltas against anchors, in framed
//! raw DEFLATE storage.
use crate::{Codec, Error, Result, Sink, Source, rewind_inflate};
use alloc::vec::Vec;
use core::ops::Range;
const INVALID: Error = Error::InvalidSaveState;
pub const MAX_DECODED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BUNDLE_BYTES: usize = 68 * 1024 * 1024;

/// `u64le(raw_len)` followed by a raw DEFLATE stream.
pub fn deflate(raw: &[u8], level: u8) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    result.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(raw, level));
    result
}
pub fn inflate(stored: &[u8]) -> Result<Vec<u8>> {
    let prefix = stored.get(..8).ok_or(INVALID)?;
    let length = u64::from_le_bytes(prefix.try_into().map_err(|_| INVALID)?);
    if length > MAX_DECODED_BYTES as u64 {
        return Err(INVALID);
    }
    let mut output = alloc::vec![0; length as usize];
    rewind_inflate::decode(&stored[8..], &mut output)?;
    Ok(output)
}

fn varint(out: &mut Vec<u8>, mut value: usize) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}
fn get_varint(bytes: &[u8], pos: &mut usize) -> Result<usize> {
    let mut value = 0usize;
    let mut shift = 0;
    loop {
        if shift > 28 {
            return Err(INVALID);
        }
        let byte = *bytes.get(*pos).ok_or(INVALID)?;
        *pos += 1;
        if shift == 28 && byte > 0x0f {
            return Err(INVALID);
        }
        value |= usize::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(value);
        }
        shift += 7;
    }
}
const INDEX_BITS: u32 = 17;
const MIN_COPY: usize = 12;
/// A copy this long at the previous displacement is taken without searching.
const LONG_COPY: usize = 64;

/// Positions of 8-byte keys at every fourth byte of an anchor, built once
/// per anchor so `copy_diff` finds data that moved within the state.
#[derive(Debug)]
pub struct AnchorIndex(Vec<u32>);
fn index_slot(bytes: &[u8]) -> usize {
    let key = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    (key.wrapping_mul(0x9E37_79B1_85EB_CA87) >> (64 - INDEX_BITS)) as usize
}
impl AnchorIndex {
    pub fn new(base: &[u8]) -> Self {
        let mut slots = alloc::vec![u32::MAX; 1 << INDEX_BITS];
        for at in (0..base.len().saturating_sub(7)).step_by(4) {
            slots[index_slot(&base[at..])] = at as u32;
        }
        Self(slots)
    }
    fn lookup(&self, bytes: &[u8]) -> Option<usize> {
        let at = self.0[index_slot(bytes)];
        (at != u32::MAX).then_some(at as usize)
    }
}
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let len = a.len().min(b.len());
    let mut n = 0;
    while n + 16 <= len && a[n..n + 16] == b[n..n + 16] {
        n += 16;
    }
    while n < len && a[n] == b[n] {
        n += 1;
    }
    n
}
fn longer(a: (usize, usize), b: (usize, usize)) -> (usize, usize) {
    if a.1 > b.1 { a } else { b }
}
fn zigzag(value: i64) -> usize {
    ((value << 1) ^ (value >> 63)) as usize
}
fn unzigzag(value: usize) -> i64 {
    let value = value as u64;
    (value >> 1) as i64 ^ -((value & 1) as i64)
}

/// Copy/literal difference against `base`, which must be the base `index`
/// was built from. Ops (literal length, copy length, zigzag change of the
/// copy's displacement from the previous copy) precede the literals; bytes
/// after the last copy are trailing literals. At each position the previous
/// displacement is tried first, then the same offset, then the index, so a
/// field that changes length shifts only one displacement.
pub fn copy_diff(base: &[u8], index: &AnchorIndex, target: &[u8]) -> Vec<u8> {
    let mut ops = Vec::new();
    let mut literals = Vec::new();
    let (mut at, mut literal_start) = (0usize, 0usize);
    // The last two distinct displacements and that of the last long copy:
    // short index matches inside changed data do not lose the displacement
    // of a region shifted by a field that changed length.
    let mut recent = [0i64; 2];
    let mut settled = 0i64;
    while at + MIN_COPY <= target.len() {
        let copy_from = |displacement: i64| {
            at.checked_add_signed(displacement as isize)
                .filter(|&s| s < base.len())
                .map_or((0, 0), |s| (s, common_prefix(&base[s..], &target[at..])))
        };
        // The longest candidate wins; ties keep the cheaper, earlier one.
        let mut best = copy_from(recent[0]);
        if best.1 < LONG_COPY {
            for displacement in [recent[1], settled, 0] {
                best = longer(copy_from(displacement), best);
            }
            if let Some(source) = index.lookup(&target[at..]) {
                best = longer(copy_from(source as i64 - at as i64), best);
            }
        }
        if best.1 < MIN_COPY {
            at += 1;
            continue;
        }
        varint(&mut ops, at - literal_start);
        literals.extend_from_slice(&target[literal_start..at]);
        varint(&mut ops, best.1);
        let next = best.0 as i64 - at as i64;
        varint(&mut ops, zigzag(next - recent[0]));
        if next != recent[0] {
            recent = [next, recent[0]];
        }
        if best.1 >= LONG_COPY {
            settled = next;
        }
        at += best.1;
        literal_start = at;
    }
    let mut out = Vec::with_capacity(ops.len() + literals.len() + target.len() - literal_start + 5);
    varint(&mut out, ops.len());
    out.extend_from_slice(&ops);
    out.extend_from_slice(&literals);
    out.extend_from_slice(&target[literal_start..]);
    out
}
pub fn copy_patch(base: &[u8], delta: &[u8], target_len: usize) -> Result<Vec<u8>> {
    if target_len > MAX_DECODED_BYTES {
        return Err(INVALID);
    }
    let mut pos = 0;
    let ops_len = get_varint(delta, &mut pos)?;
    if ops_len > delta.len() - pos {
        return Err(INVALID);
    }
    let ops = &delta[pos..pos + ops_len];
    let mut literals = &delta[pos + ops_len..];
    let mut out = Vec::with_capacity(target_len);
    let (mut op, mut displacement) = (0, 0i64);
    while op < ops.len() {
        let literal = get_varint(ops, &mut op)?;
        let copy = get_varint(ops, &mut op)?;
        let change = unzigzag(get_varint(ops, &mut op)?);
        if literal > literals.len() || literal > target_len - out.len() {
            return Err(INVALID);
        }
        out.extend_from_slice(&literals[..literal]);
        literals = &literals[literal..];
        displacement = displacement.checked_add(change).ok_or(INVALID)?;
        let source = (out.len() as i64)
            .checked_add(displacement)
            .and_then(|source| usize::try_from(source).ok())
            .ok_or(INVALID)?;
        if copy > target_len - out.len() || source > base.len() || copy > base.len() - source {
            return Err(INVALID);
        }
        out.extend_from_slice(&base[source..source + copy]);
    }
    if literals.len() != target_len - out.len() {
        return Err(INVALID);
    }
    out.extend_from_slice(literals);
    Ok(out)
}

/// `u32` target length, `u32` raw copy-delta length, then level-3 raw DEFLATE.
pub fn encode_copy_delta(base: &[u8], index: &AnchorIndex, target: &[u8]) -> Vec<u8> {
    let delta = copy_diff(base, index, target);
    let mut out = Vec::new();
    out.extend_from_slice(&(target.len() as u32).to_le_bytes());
    out.extend_from_slice(&(delta.len() as u32).to_le_bytes());
    out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(&delta, 3));
    out
}
pub fn decode_copy_delta(base: &[u8], stored: &[u8]) -> Result<Vec<u8>> {
    let header = stored.get(..8).ok_or(INVALID)?;
    let target_len = u32::from_le_bytes(header[..4].try_into().map_err(|_| INVALID)?) as usize;
    let delta_len = u32::from_le_bytes(header[4..].try_into().map_err(|_| INVALID)?) as usize;
    if target_len > MAX_DECODED_BYTES || delta_len > MAX_DECODED_BYTES * 2 {
        return Err(INVALID);
    }
    let mut delta = alloc::vec![0; delta_len];
    rewind_inflate::decode(&stored[8..], &mut delta)?;
    copy_patch(base, &delta, target_len)
}

pub const DEFAULT_BUDGET_BYTES: u32 = 15 * 1024 * 1024;
pub const MAX_BUDGET_BYTES: u32 = 64 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 4096;
pub const ENTRY_OVERHEAD_BYTES: usize = 64;
pub const SPEED_SCALE_ONE: u16 = 256;
const BUNDLE_MAGIC: &[u8; 8] = b"EFREWIND";
/// Bundles embed save states, so a new state version needs a new bundle version.
const BUNDLE_VERSION: u32 = 3;
const INDEX_MAGIC: &[u8; 8] = b"EFRWINDX";
/// Policy fields between a bundle's live state and its anchor count.
const POLICY_BYTES: usize = 4 + 2 + 1 + 8 + 2 + 8 + 1 + 8 + 1 + 4 + 4;

/// Names a stored payload by its content: the first 16 bytes of its BLAKE3.
pub type PayloadKey = [u8; 16];
pub fn payload_key(bytes: &[u8]) -> PayloadKey {
    let mut key = [0; 16];
    key.copy_from_slice(&blake3::hash(bytes).as_bytes()[..16]);
    key
}

// Exact 192-bit product for p=0 and p=1 eviction comparisons.
fn product3(a: u64, b: u64, c: u64) -> (u64, u64, u64) {
    let first = u128::from(a) * u128::from(b);
    let low = u128::from(first as u64) * u128::from(c);
    let upper = (first >> 64) * u128::from(c) + (low >> 64);
    ((upper >> 64) as u64, upper as u64, low as u64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Encoding {
    Raw = 0,
    Deflate = 1,
    /// Copy/literal difference against an anchor; written by `Compression::Delta`.
    CopyDelta = 3,
}
impl Encoding {
    fn uses_anchor(self) -> bool {
        self == Self::CopyDelta
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    Off = 0,
    Fast = 1,
    Delta = 2,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RewindConfig {
    pub budget_bytes: u32,
    pub interval_frames: u16,
    pub compression: Compression,
    pub age_exponent: f64,
}
impl Default for RewindConfig {
    fn default() -> Self {
        Self {
            budget_bytes: DEFAULT_BUDGET_BYTES,
            interval_frames: 60,
            compression: Compression::Delta,
            age_exponent: 0.5,
        }
    }
}
impl RewindConfig {
    pub fn validate(self) -> Result<()> {
        if self.budget_bytes > MAX_BUDGET_BYTES
            || self.interval_frames == 0
            || !self.age_exponent.is_finite()
            || self.age_exponent < 0.0
        {
            return Err(Error::InvalidRewindConfig);
        }
        Ok(())
    }
    pub fn disabled(self) -> bool {
        self.budget_bytes == 0
    }
}
#[derive(Clone, Debug)]
enum Stored {
    Owned(Vec<u8>),
    Imported(Range<usize>),
}
impl Stored {
    fn len(&self) -> usize {
        match self {
            Self::Owned(bytes) => bytes.len(),
            Self::Imported(range) => range.len(),
        }
    }

    fn bytes<'a>(&'a self, bundle: &'a [u8]) -> &'a [u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Imported(range) => &bundle[range.clone()],
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    pub frame: u64,
    pub cycle: u64,
    pub sequence: u64,
    state: Stored,
    key: PayloadKey,
    pub encoding: Encoding,
    pub anchor: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub frame: u64,
    pub cycle: u64,
    pub bytes: usize,
    pub encoding: Encoding,
}
#[derive(Clone, Debug)]
struct Anchor {
    id: u32,
    stored: Stored,
    key: PayloadKey,
    refs: u32,
}

/// Storage policy only. The session decides when it is safe to snapshot or restore.
#[derive(Debug)]
pub struct RewindBuffer {
    pub config: RewindConfig,
    entries: Vec<Entry>,
    anchors: Vec<Anchor>,
    bundle: Option<Vec<u8>>,
    bundle_end: usize,
    current_anchor_raw: Option<Vec<u8>>,
    current_anchor_index: Option<AnchorIndex>,
    current_anchor: u32,
    next_anchor: u32,
    next_sequence: u64,
    stored_bytes: usize,
    last_capture_frame: Option<u64>,
    speed_scale_q8: u16,
}
impl RewindBuffer {
    pub fn new(config: RewindConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            entries: Vec::new(),
            anchors: Vec::new(),
            bundle: None,
            bundle_end: 0,
            current_anchor_raw: None,
            current_anchor_index: None,
            current_anchor: 0,
            next_anchor: 0,
            next_sequence: 0,
            stored_bytes: 0,
            last_capture_frame: None,
            speed_scale_q8: SPEED_SCALE_ONE,
        })
    }
    pub fn set_config(&mut self, config: RewindConfig) -> Result<()> {
        config.validate()?;
        self.clear();
        self.config = config;
        Ok(())
    }
    pub fn set_speed_scale(&mut self, scale: u16) -> Result<()> {
        if !(SPEED_SCALE_ONE / 2..=SPEED_SCALE_ONE * 16).contains(&scale) {
            return Err(Error::InvalidRewindSpeed);
        }
        self.speed_scale_q8 = scale;
        Ok(())
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.anchors.clear();
        self.bundle = None;
        self.bundle_end = 0;
        self.current_anchor_raw = None;
        self.current_anchor_index = None;
        self.stored_bytes = 0;
        self.last_capture_frame = None;
    }
    pub fn depth(&self) -> usize {
        self.entries.len()
    }
    pub fn memory_bytes(&self) -> usize {
        self.stored_bytes
    }
    pub fn info(&self, index: usize) -> Option<Info> {
        self.entries.get(index).map(|entry| Info {
            frame: entry.frame,
            cycle: entry.cycle,
            bytes: entry.state.len(),
            encoding: entry.encoding,
        })
    }
    pub fn newest(&self) -> Option<Info> {
        self.entries.len().checked_sub(1).and_then(|i| self.info(i))
    }
    pub fn due_for_capture(&self, frame: u64) -> bool {
        if self.config.disabled() || frame == 0 {
            return false;
        }
        let interval = (u64::from(self.config.interval_frames) * u64::from(self.speed_scale_q8))
            .div_ceil(u64::from(SPEED_SCALE_ONE));
        let Some(last) = self.last_capture_frame else {
            return frame >= interval;
        };
        frame >= last && frame - last >= interval
    }
    pub fn note_skipped(&mut self, frame: u64) {
        self.last_capture_frame = Some(frame);
    }
    fn anchor_index(&self, id: u32) -> Option<usize> {
        self.anchors.iter().position(|a| a.id == id)
    }
    fn encode_against_anchor(&mut self, raw: &[u8]) -> Vec<u8> {
        if let Some(base) = &self.current_anchor_raw {
            let index = self.current_anchor_index.get_or_insert_with(|| AnchorIndex::new(base));
            let stored = encode_copy_delta(base, index, raw);
            if self
                .anchor_index(self.current_anchor)
                .is_some_and(|i| stored.len() * 2 <= self.anchors[i].stored.len())
            {
                return stored;
            }
        }
        let anchor_stored = deflate(raw, 6);
        let index = AnchorIndex::new(raw);
        let stored = encode_copy_delta(raw, &index, raw);
        self.current_anchor_raw = Some(raw.to_vec());
        self.current_anchor_index = Some(index);
        while self.anchor_index(self.next_anchor).is_some() {
            self.next_anchor = self.next_anchor.wrapping_add(1);
        }
        self.current_anchor = self.next_anchor;
        self.next_anchor = self.next_anchor.wrapping_add(1);
        self.stored_bytes += anchor_stored.len();
        self.anchors.push(Anchor {
            id: self.current_anchor,
            key: payload_key(&anchor_stored),
            stored: Stored::Owned(anchor_stored),
            refs: 0,
        });
        stored
    }
    pub fn push(&mut self, frame: u64, cycle: u64, raw: &[u8]) -> Result<()> {
        if self.config.disabled() {
            return Ok(());
        }
        if raw.len() > MAX_DECODED_BYTES {
            return Err(INVALID);
        }
        if self.next_sequence == u64::MAX {
            return Err(Error::RewindSequenceExhausted);
        }
        let (state, encoding, anchor) = match self.config.compression {
            Compression::Off => (raw.to_vec(), Encoding::Raw, 0),
            Compression::Fast => (deflate(raw, 1), Encoding::Deflate, 0),
            Compression::Delta => {
                let state = self.encode_against_anchor(raw);
                let id = self.current_anchor;
                let i = self.anchor_index(id).expect("newly created anchor exists");
                self.anchors[i].refs += 1;
                (state, Encoding::CopyDelta, id)
            }
        };
        self.stored_bytes += state.len() + ENTRY_OVERHEAD_BYTES;
        self.entries.push(Entry {
            frame,
            cycle,
            sequence: self.next_sequence,
            key: payload_key(&state),
            state: Stored::Owned(state),
            encoding,
            anchor,
        });
        self.next_sequence += 1;
        self.last_capture_frame = Some(frame);
        self.enforce_budget();
        self.reuse_bundle_space();
        Ok(())
    }
    pub fn decode(&self, index: usize) -> Result<Vec<u8>> {
        self.decode_with_bundle(index, self.bundle.as_deref().unwrap_or(&[]))
    }
    fn decode_with_bundle(&self, index: usize, bundle: &[u8]) -> Result<Vec<u8>> {
        let entry = self.entries.get(index).ok_or(Error::InvalidRewindSteps)?;
        let stored = entry.state.bytes(bundle);
        match entry.encoding {
            Encoding::Raw => Ok(stored.to_vec()),
            Encoding::Deflate => inflate(stored),
            Encoding::CopyDelta => {
                if entry.anchor == self.current_anchor
                    && let Some(raw) = &self.current_anchor_raw
                {
                    return decode_copy_delta(raw, stored);
                }
                let i = self.anchor_index(entry.anchor).ok_or(INVALID)?;
                decode_copy_delta(&inflate(self.anchors[i].stored.bytes(bundle))?, stored)
            }
        }
    }
    pub fn target_older_than(&self, current_frame: u64, steps: usize) -> Option<usize> {
        if steps == 0 {
            return None;
        }
        self.entries
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, e)| e.frame < current_frame)
            .nth(steps - 1)
            .map(|(i, _)| i)
    }
    pub fn truncate_newer(&mut self, index: usize) {
        assert!(index < self.entries.len());
        while self.entries.len() > index + 1 {
            self.remove_at(self.entries.len() - 1);
        }
        self.last_capture_frame = Some(self.entries[index].frame);
        if !self.entries.iter().any(|entry| matches!(entry.state, Stored::Imported(_)))
            && !self.anchors.iter().any(|anchor| matches!(anchor.stored, Stored::Imported(_)))
        {
            self.bundle = None;
            self.bundle_end = 0;
        }
    }
    fn bytes_freed_by(&self, entry: &Entry) -> usize {
        let mut bytes = entry.state.len() + ENTRY_OVERHEAD_BYTES;
        if entry.encoding.uses_anchor()
            && entry.anchor != self.current_anchor
            && let Some(a) = self.anchors.iter().find(|a| a.id == entry.anchor && a.refs == 1)
        {
            bytes += a.stored.len();
        }
        bytes
    }
    fn enforce_budget(&mut self) {
        while self.entries.len() > 1
            && (self.stored_bytes > self.config.budget_bytes as usize
                || self.entries.len() > MAX_ENTRIES)
        {
            if self.entries.len() == 2 {
                self.remove_at(0);
                continue;
            }
            let items = &self.entries;
            let now = items.last().unwrap().sequence;
            let p = self.config.age_exponent;
            let mut best = 1;
            let mut best_gap = items[2].sequence - items[0].sequence;
            let mut best_age = now - items[1].sequence + 1;
            let mut best_bytes = self.bytes_freed_by(&items[1]);
            for i in 2..items.len() - 1 {
                let gap = items[i + 1].sequence - items[i - 1].sequence;
                let age = now - items[i].sequence + 1;
                let freed = self.bytes_freed_by(&items[i]);
                let better = if p == 0.0 || p == 1.0 {
                    product3(gap, best_bytes as u64, if p == 1.0 { best_age } else { 1 })
                        < product3(best_gap, freed as u64, if p == 1.0 { age } else { 1 })
                } else {
                    let ratio = age as f64 / best_age as f64;
                    let weight = if p == 0.5 { libm::sqrt(ratio) } else { libm::pow(ratio, p) };
                    (gap as f64 * best_bytes as f64) < best_gap as f64 * freed as f64 * weight
                };
                if better {
                    best = i;
                    best_gap = gap;
                    best_age = age;
                    best_bytes = freed;
                }
            }
            self.remove_at(best);
        }
    }
    fn remove_at(&mut self, index: usize) {
        let entry = self.entries.remove(index);
        self.stored_bytes -= entry.state.len() + ENTRY_OVERHEAD_BYTES;
        if entry.encoding.uses_anchor() {
            self.release_anchor(entry.anchor);
        }
    }
    fn free_anchor(&mut self, index: usize) {
        let anchor = self.anchors.swap_remove(index);
        self.stored_bytes -= anchor.stored.len();
        if anchor.id == self.current_anchor {
            self.current_anchor_raw = None;
            self.current_anchor_index = None;
        }
    }
    fn release_anchor(&mut self, id: u32) {
        let Some(index) = self.anchor_index(id) else {
            return;
        };
        self.anchors[index].refs -= 1;
        if self.anchors[index].refs == 0 {
            self.free_anchor(index);
        } else if self.anchors[index].refs == 1
            && id != self.current_anchor
            && let Some(i) =
                self.entries.iter().position(|e| e.encoding.uses_anchor() && e.anchor == id)
            && let Ok(raw) = self.decode(i)
        {
            let stored = deflate(&raw, 1);
            self.stored_bytes -= self.entries[i].state.len();
            self.entries[i].key = payload_key(&stored);
            self.entries[i].state = Stored::Owned(stored);
            self.entries[i].encoding = Encoding::Deflate;
            self.stored_bytes += self.entries[i].state.len();
            self.free_anchor(index);
        }
    }

    /// Compact live imported spans only when the tail cannot hold new
    /// payloads, then move owned captures into the freed upload space.
    fn reuse_bundle_space(&mut self) {
        let Some(bundle) = &mut self.bundle else { return };
        let mut spans: Vec<&mut Stored> = self
            .anchors
            .iter_mut()
            .map(|anchor| &mut anchor.stored)
            .chain(self.entries.iter_mut().map(|entry| &mut entry.state))
            .collect();
        let mut used = 0;
        let mut incoming = 0;
        let mut smallest = usize::MAX;
        for span in &spans {
            match span {
                Stored::Imported(range) => used += range.len(),
                Stored::Owned(bytes) => {
                    incoming += bytes.len();
                    smallest = smallest.min(bytes.len());
                }
            }
        }
        if used == 0 && smallest > bundle.len() {
            self.bundle = None;
            self.bundle_end = 0;
            return;
        }
        if incoming == 0 || smallest > bundle.len() - used {
            return;
        }
        if incoming > bundle.len() - self.bundle_end && used < self.bundle_end {
            spans.sort_unstable_by_key(|span| match span {
                Stored::Imported(range) => range.start,
                Stored::Owned(_) => usize::MAX,
            });
            self.bundle_end = 0;
            for span in &mut spans {
                if let Stored::Imported(range) = &mut **span {
                    let len = range.len();
                    bundle.copy_within(range.clone(), self.bundle_end);
                    *range = self.bundle_end..self.bundle_end + len;
                    self.bundle_end += len;
                }
            }
        }
        for span in &mut spans {
            if let Stored::Owned(bytes) = &mut **span {
                let end = self.bundle_end + bytes.len();
                if end <= bundle.len() {
                    bundle[self.bundle_end..end].copy_from_slice(bytes);
                    **span = Stored::Imported(self.bundle_end..end);
                    self.bundle_end = end;
                }
            }
        }
    }

    /// Session-owned live state precedes the policy, anchors and entries.
    pub fn write_bundle(&self, live: &[u8]) -> Result<Vec<u8>> {
        if live.len() > MAX_DECODED_BYTES {
            return Err(INVALID);
        }
        let mut count = Sink::new(None, None);
        self.write_history(&mut count, false)?;
        let history_len = usize::try_from(count.count).map_err(|_| INVALID)?;
        let len = 16usize
            .checked_add(live.len())
            .and_then(|n| n.checked_add(history_len))
            .and_then(|n| n.checked_add(32))
            .ok_or(INVALID)?;
        let mut out = Vec::new();
        out.try_reserve_exact(len).map_err(|_| Error::OutOfMemory)?;
        {
            let mut sink = Sink::new(Some(&mut out), None);
            sink.write_all(BUNDLE_MAGIC)?;
            BUNDLE_VERSION.encode(&mut sink)?;
            (live.len() as u32).encode(&mut sink)?;
            sink.write_all(live)?;
            self.write_history(&mut sink, false)?;
        }
        let hash = blake3::hash(&out);
        out.extend_from_slice(hash.as_bytes());
        debug_assert_eq!(out.len(), len);
        Ok(out)
    }

    /// A bundle without its live state, with each payload replaced by its key.
    /// Together with the live state and `payloads()`, it rebuilds the bundle
    /// through `assemble_bundle`, so a host can store payloads once each.
    pub fn write_index(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut sink = Sink::new(Some(&mut out), None);
            sink.write_all(INDEX_MAGIC)?;
            BUNDLE_VERSION.encode(&mut sink)?;
            self.write_history(&mut sink, true)?;
        }
        let hash = blake3::hash(&out);
        out.extend_from_slice(hash.as_bytes());
        Ok(out)
    }

    /// Every stored payload in index order (anchors, then entries oldest
    /// first), with the key the index names it by.
    pub fn payloads(&self) -> impl Iterator<Item = (&PayloadKey, &[u8])> {
        let bundle = self.bundle.as_deref().unwrap_or(&[]);
        self.anchors
            .iter()
            .map(move |anchor| (&anchor.key, anchor.stored.bytes(bundle)))
            .chain(self.entries.iter().map(move |entry| (&entry.key, entry.state.bytes(bundle))))
    }

    /// Rebuilds the bundle that `write_index` describes. Every payload must
    /// match its key; `read_bundle` then validates the result as any upload.
    pub fn assemble_bundle<'a>(
        live: &[u8],
        index: &[u8],
        payload: impl Fn(&PayloadKey) -> Option<&'a [u8]>,
    ) -> Result<Vec<u8>> {
        const BAD: Error = Error::InvalidRewindBundle;
        fn take<'b>(input: &mut &'b [u8], len: usize) -> Result<&'b [u8]> {
            if input.len() < len {
                return Err(BAD);
            }
            let (head, tail) = input.split_at(len);
            *input = tail;
            Ok(head)
        }
        fn u32_at(bytes: &[u8]) -> u32 {
            u32::from_le_bytes(bytes.try_into().unwrap())
        }
        if live.len() > MAX_DECODED_BYTES {
            return Err(INVALID);
        }
        if index.len() < 8 + 4 + POLICY_BYTES + 8 + 32 {
            return Err(BAD);
        }
        let (mut body, digest) = index.split_at(index.len() - 32);
        if digest != blake3::hash(body).as_bytes() {
            return Err(BAD);
        }
        if take(&mut body, 8)? != INDEX_MAGIC {
            return Err(BAD);
        }
        if u32_at(take(&mut body, 4)?) != BUNDLE_VERSION {
            return Err(Error::UnsupportedRewindBundleVersion);
        }
        let mut out = Vec::new();
        out.extend_from_slice(BUNDLE_MAGIC);
        out.extend_from_slice(&BUNDLE_VERSION.to_le_bytes());
        out.extend_from_slice(&(live.len() as u32).to_le_bytes());
        out.extend_from_slice(live);
        out.extend_from_slice(take(&mut body, POLICY_BYTES)?);
        let counts = take(&mut body, 8)?;
        let (anchors, entries) = (u32_at(&counts[..4]), u32_at(&counts[4..]));
        if entries as usize > MAX_ENTRIES || anchors > entries {
            return Err(BAD);
        }
        out.extend_from_slice(counts);
        for i in 0..anchors + entries {
            // Anchor id, or entry frame, cycle, sequence, encoding and anchor id.
            out.extend_from_slice(take(&mut body, if i < anchors { 4 } else { 29 })?);
            let len = take(&mut body, 4)?;
            let key: &PayloadKey = take(&mut body, 16)?.try_into().unwrap();
            let bytes = payload(key).ok_or(BAD)?;
            if bytes.len() != u32_at(len) as usize
                || out.len() + bytes.len() > MAX_BUNDLE_BYTES
                || payload_key(bytes) != *key
            {
                return Err(BAD);
            }
            out.extend_from_slice(len);
            out.extend_from_slice(bytes);
        }
        if !body.is_empty() {
            return Err(BAD);
        }
        let hash = blake3::hash(&out);
        out.extend_from_slice(hash.as_bytes());
        Ok(out)
    }

    /// With `keys`, each payload is written as its key instead of its bytes.
    fn write_history(&self, sink: &mut Sink<'_>, keys: bool) -> Result<()> {
        self.config.budget_bytes.encode(sink)?;
        self.config.interval_frames.encode(sink)?;
        (self.config.compression as u8).encode(sink)?;
        self.config.age_exponent.to_bits().encode(sink)?;
        self.speed_scale_q8.encode(sink)?;
        self.next_sequence.encode(sink)?;
        self.last_capture_frame.is_some().encode(sink)?;
        self.last_capture_frame.unwrap_or(0).encode(sink)?;
        self.current_anchor_raw.is_some().encode(sink)?;
        self.current_anchor.encode(sink)?;
        self.next_anchor.encode(sink)?;
        (self.anchors.len() as u32).encode(sink)?;
        (self.entries.len() as u32).encode(sink)?;
        for anchor in &self.anchors {
            anchor.id.encode(sink)?;
            (anchor.stored.len() as u32).encode(sink)?;
            if keys {
                sink.write_all(&anchor.key)?;
            } else {
                sink.write_all(anchor.stored.bytes(self.bundle.as_deref().unwrap_or(&[])))?;
            }
        }
        for entry in &self.entries {
            entry.frame.encode(sink)?;
            entry.cycle.encode(sink)?;
            entry.sequence.encode(sink)?;
            (entry.encoding as u8).encode(sink)?;
            entry.anchor.encode(sink)?;
            (entry.state.len() as u32).encode(sink)?;
            if keys {
                sink.write_all(&entry.key)?;
            } else {
                sink.write_all(entry.state.bytes(self.bundle.as_deref().unwrap_or(&[])))?;
            }
        }
        Ok(())
    }

    /// Validates every stored state through the caller's transactional state
    /// loader. The returned history is owned; the caller commits it alongside
    /// the separately returned live state only after both are accepted.
    pub fn read_bundle(
        bytes: &[u8],
        validate: impl Fn(&[u8]) -> Result<(u64, u64)>,
    ) -> Result<(Vec<u8>, Self)> {
        Self::read_bundle_impl(bytes, true, validate)
    }

    /// Validates without copying stored payloads and adopts the upload only
    /// after every state passes. On failure the caller gets the unchanged Vec.
    pub fn read_bundle_owned(
        bytes: Vec<u8>,
        validate: impl Fn(&[u8]) -> Result<(u64, u64)>,
    ) -> core::result::Result<(Vec<u8>, Self), (Error, Vec<u8>)> {
        match Self::read_bundle_impl(&bytes, false, validate) {
            Ok((live, mut history)) => {
                if !history.entries.is_empty() {
                    history.bundle_end = bytes.len();
                    history.bundle = Some(bytes);
                }
                Ok((live, history))
            }
            Err(error) => Err((error, bytes)),
        }
    }

    fn read_bundle_impl(
        bytes: &[u8],
        copy_payloads: bool,
        validate: impl Fn(&[u8]) -> Result<(u64, u64)>,
    ) -> Result<(Vec<u8>, Self)> {
        if bytes.len() > MAX_BUNDLE_BYTES {
            return Err(Error::RewindBundleTooLarge);
        }
        if bytes.len() < 48 {
            return Err(Error::InvalidRewindBundle);
        }
        let (body, digest) = bytes.split_at(bytes.len() - 32);
        if digest != blake3::hash(body).as_bytes() {
            return Err(Error::InvalidRewindBundle);
        }
        let mut source = Source::new(body);
        let mut magic = [0; 8];
        source.read_all(&mut magic)?;
        if &magic != BUNDLE_MAGIC {
            return Err(Error::InvalidRewindBundle);
        }
        let version: u32 = read_value(&mut source)?;
        if version != BUNDLE_VERSION {
            return Err(Error::UnsupportedRewindBundleVersion);
        }
        let live = read_bytes(&mut source, MAX_DECODED_BYTES)?.to_vec();
        let (live_frame, live_cycle) = validate(&live)?;
        let config = RewindConfig {
            budget_bytes: read_value(&mut source)?,
            interval_frames: read_value(&mut source)?,
            compression: compression(read_value(&mut source)?)?,
            age_exponent: f64::from_bits(read_value(&mut source)?),
        };
        config.validate()?;
        let mut history = Self::new(config)?;
        history.set_speed_scale(read_value(&mut source)?)?;
        history.next_sequence = read_value(&mut source)?;
        let has_capture: bool = read_value(&mut source)?;
        let frame: u64 = read_value(&mut source)?;
        if frame > live_frame || (!has_capture && frame != 0) {
            return Err(Error::InvalidRewindBundle);
        }
        history.last_capture_frame = has_capture.then_some(frame);
        let has_current: bool = read_value(&mut source)?;
        history.current_anchor = read_value(&mut source)?;
        history.next_anchor = read_value(&mut source)?;
        let anchor_count: u32 = read_value(&mut source)?;
        let entry_count: u32 = read_value(&mut source)?;
        if entry_count as usize > MAX_ENTRIES
            || anchor_count > entry_count
            || (config.disabled() && entry_count != 0)
        {
            return Err(Error::InvalidRewindBundle);
        }
        for _ in 0..anchor_count {
            let id = read_value(&mut source)?;
            if history.anchor_index(id).is_some() {
                return Err(Error::InvalidRewindBundle);
            }
            let stored = read_stored(
                &mut source,
                body.len(),
                MAX_DECODED_BYTES * 2 + 64 * 1024,
                copy_payloads,
            )?;
            history.stored_bytes += stored.len();
            let key = payload_key(stored.bytes(body));
            history.anchors.push(Anchor { id, stored, key, refs: 0 });
        }
        for _ in 0..entry_count {
            let frame: u64 = read_value(&mut source)?;
            let cycle: u64 = read_value(&mut source)?;
            let sequence: u64 = read_value(&mut source)?;
            let encoding = encoding(read_value(&mut source)?)?;
            let anchor = read_value(&mut source)?;
            if !has_capture
                || frame == 0
                || frame > history.last_capture_frame.unwrap_or(0)
                || cycle > live_cycle
                || sequence >= history.next_sequence
            {
                return Err(Error::InvalidRewindBundle);
            }
            if let Some(previous) = history.entries.last()
                && (frame <= previous.frame
                    || cycle <= previous.cycle
                    || sequence <= previous.sequence)
            {
                return Err(Error::InvalidRewindBundle);
            }
            let stored = read_stored(
                &mut source,
                body.len(),
                if encoding == Encoding::Raw {
                    MAX_DECODED_BYTES
                } else {
                    MAX_DECODED_BYTES * 2 + 64 * 1024
                },
                copy_payloads,
            )?;
            if encoding.uses_anchor() {
                let i = history.anchor_index(anchor).ok_or(Error::InvalidRewindBundle)?;
                history.anchors[i].refs += 1;
            }
            history.stored_bytes += stored.len() + ENTRY_OVERHEAD_BYTES;
            let key = payload_key(stored.bytes(body));
            history.entries.push(Entry {
                frame,
                cycle,
                sequence,
                key,
                encoding,
                anchor,
                state: stored,
            });
        }
        if !source.input.is_empty()
            || (entry_count > 1 && history.stored_bytes > config.budget_bytes as usize)
            || history.anchors.iter().any(|anchor| anchor.refs == 0)
        {
            return Err(Error::InvalidRewindBundle);
        }
        if has_current {
            let i =
                history.anchor_index(history.current_anchor).ok_or(Error::InvalidRewindBundle)?;
            history.current_anchor_raw = Some(inflate(history.anchors[i].stored.bytes(body))?);
        } else if history.anchor_index(history.current_anchor).is_some() {
            return Err(Error::InvalidRewindBundle);
        }
        for anchor in &history.anchors {
            validate(&inflate(anchor.stored.bytes(body))?)?;
        }
        for (i, entry) in history.entries.iter().enumerate() {
            let labels = validate(&history.decode_with_bundle(i, body)?)?;
            if labels != (entry.frame, entry.cycle) {
                return Err(Error::InvalidRewindBundle);
            }
        }
        Ok((live, history))
    }
}

fn read_value<T: Codec + Default>(source: &mut Source<'_>) -> Result<T> {
    let mut value = T::default();
    value.decode(source).map_err(|_| Error::InvalidRewindBundle)?;
    Ok(value)
}
fn read_bytes<'a>(source: &mut Source<'a>, limit: usize) -> Result<&'a [u8]> {
    let len = read_value::<u32>(source)? as usize;
    if len > limit || len > source.input.len() {
        return Err(Error::InvalidRewindBundle);
    }
    let (bytes, tail) = source.input.split_at(len);
    source.input = tail;
    source.count += len as u64;
    Ok(bytes)
}
fn read_stored(
    source: &mut Source<'_>,
    body_len: usize,
    limit: usize,
    copy: bool,
) -> Result<Stored> {
    let bytes = read_bytes(source, limit)?;
    let start = body_len - source.input.len() - bytes.len();
    Ok(if copy {
        Stored::Owned(bytes.to_vec())
    } else {
        Stored::Imported(start..start + bytes.len())
    })
}
fn compression(raw: u8) -> Result<Compression> {
    match raw {
        0 => Ok(Compression::Off),
        1 => Ok(Compression::Fast),
        2 => Ok(Compression::Delta),
        _ => Err(Error::InvalidRewindBundle),
    }
}
fn encoding(raw: u8) -> Result<Encoding> {
    match raw {
        0 => Ok(Encoding::Raw),
        1 => Ok(Encoding::Deflate),
        3 => Ok(Encoding::CopyDelta),
        _ => Err(Error::InvalidRewindBundle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Scene and region changes like a game's; the PRNG is
    // local to this fixture, so expected states do not come from decode().
    fn synthetic_state(len: usize, step: u64) -> Vec<u8> {
        let mut state = alloc::vec![0; len];
        let mut scene = 0x5eed_u32.wrapping_add((step / 1000) as u32);
        for (i, byte) in state.iter_mut().enumerate() {
            if i % 7 == 0 {
                scene ^= scene << 13;
                scene ^= scene >> 17;
                scene ^= scene << 5;
                *byte = scene as u8;
            }
        }
        let mut noise = step as u32 | 1;
        for region in 0..8 {
            let at = (region * len / 8 + step as usize * 97) % (len - 256);
            for byte in &mut state[at..at + 64] {
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                *byte = noise as u8;
            }
        }
        state[..8].copy_from_slice(&step.to_le_bytes());
        state
    }

    fn push_synthetic(buffer: &mut RewindBuffer, len: usize, step: u64) {
        buffer.push((step + 1) * 60, (step + 1) * 1000, &synthetic_state(len, step)).unwrap();
    }

    fn expect_decodes(buffer: &RewindBuffer, index: usize, len: usize) {
        let step = buffer.info(index).unwrap().frame / 60 - 1;
        assert_eq!(buffer.decode(index).unwrap(), synthetic_state(len, step));
    }

    #[test]
    fn adopted_histories_survive_compaction_eviction_anchor_folding_and_branching() {
        let len = 8192;
        let mut compacted = false;
        for compression in [Compression::Off, Compression::Fast, Compression::Delta] {
            for initial in [2, 36] {
                let mut original = RewindBuffer::new(RewindConfig {
                    compression,
                    budget_bytes: 32 * 1024,
                    ..RewindConfig::default()
                })
                .unwrap();
                for step in 0..initial {
                    push_synthetic(&mut original, len, 980 + step);
                }
                let live = synthetic_state(len, 980 + initial);
                let upload = original.write_bundle(&live).unwrap();
                let address = upload.as_ptr();
                drop(original);
                let validate = |state: &[u8]| {
                    let step = u64::from_le_bytes(state[..8].try_into().unwrap());
                    Ok(((step + 1) * 60, (step + 1) * 1000))
                };
                let (loaded_live, mut loaded) =
                    RewindBuffer::read_bundle_owned(upload, validate).unwrap();
                assert_eq!(loaded_live, live);
                assert_eq!(loaded.bundle.as_ref().unwrap().as_ptr(), address);
                assert!(
                    loaded.entries.iter().all(|entry| matches!(entry.state, Stored::Imported(_)))
                );
                let mut reused = false;
                for step in initial..160 {
                    if step == 90 {
                        loaded.truncate_newer(loaded.depth() / 2);
                    }
                    let previous_end = loaded.bundle_end;
                    push_synthetic(&mut loaded, len, 980 + step);
                    assert!(loaded.memory_bytes() <= 32 * 1024);
                    compacted |= loaded.bundle.is_some() && loaded.bundle_end < previous_end;
                    reused |= loaded.entries.iter().any(|entry| {
                        entry.frame >= (981 + initial) * 60
                            && matches!(entry.state, Stored::Imported(_))
                    });
                    for index in 0..loaded.depth() {
                        expect_decodes(&loaded, index, len);
                    }
                }
                assert!(reused, "new captures must recycle the adopted allocation");
                loaded.clear();
                assert!(loaded.bundle.is_none());
                push_synthetic(&mut loaded, len, 2001);
                expect_decodes(&loaded, 0, len);
            }
        }
        assert!(compacted, "imported spans must survive in-place compaction");

        // Two imported dependents share an old anchor; a scene change opens a
        // new one. Evicting the middle dependent folds the oldest in place.
        let mut original = RewindBuffer::new(RewindConfig::default()).unwrap();
        for step in [0, 1, 1000] {
            push_synthetic(&mut original, len, step);
        }
        let upload = original.write_bundle(&synthetic_state(len, 1001)).unwrap();
        let (_, mut loaded) = RewindBuffer::read_bundle_owned(upload, |state| {
            let step = u64::from_le_bytes(state[..8].try_into().unwrap());
            Ok(((step + 1) * 60, (step + 1) * 1000))
        })
        .unwrap();
        let old_anchor = loaded.entries[0].anchor;
        assert_eq!(old_anchor, loaded.entries[1].anchor);
        assert_ne!(old_anchor, loaded.entries[2].anchor);
        loaded.config.budget_bytes = (loaded.memory_bytes() - 1) as u32;
        loaded.enforce_budget();
        assert_eq!(loaded.depth(), 2);
        assert_eq!(loaded.info(0).unwrap().encoding, Encoding::Deflate);
        assert!(loaded.anchors.iter().all(|anchor| anchor.id != old_anchor));
        expect_decodes(&loaded, 0, len);
        expect_decodes(&loaded, 1, len);
    }

    #[test]
    fn delta_entries_are_small_share_one_anchor_and_decode_exactly() {
        let len = 64 * 1024;
        let mut buffer = RewindBuffer::new(RewindConfig::default()).unwrap();
        for step in 0..20 {
            push_synthetic(&mut buffer, len, step);
        }
        assert_eq!(buffer.depth(), 20);
        assert_eq!(buffer.anchors.len(), 1);
        let anchor_bytes = buffer.anchors[0].stored.len();
        for index in 1..20 {
            let info = buffer.info(index).unwrap();
            assert_eq!(info.encoding, Encoding::CopyDelta);
            assert!(info.bytes * 2 <= anchor_bytes);
        }
        for index in 0..20 {
            expect_decodes(&buffer, index, len);
        }
    }

    #[test]
    fn budget_eviction_keeps_newest_and_oldest_and_coarsens_with_age() {
        let len = 16 * 1024;
        let mut buffer =
            RewindBuffer::new(RewindConfig { budget_bytes: 64 * 1024, ..RewindConfig::default() })
                .unwrap();
        for step in 0..400 {
            push_synthetic(&mut buffer, len, step);
            assert!(buffer.memory_bytes() <= 64 * 1024 || buffer.depth() == 1);
        }
        assert!(buffer.depth() < 400);
        assert_eq!(buffer.info(0).unwrap().frame, 60);
        assert_eq!(buffer.newest().unwrap().frame, 400 * 60);
        // Recent history is denser than old history. Copy deltas make the
        // capture right after an anchor much cheaper than the rest, so byte
        // weighting keeps it; compare the oldest and newest quarters' spans.
        let items = &buffer.entries;
        let quarter = items.len() / 4;
        let recent_span =
            items[items.len() - 1].sequence - items[items.len() - 1 - quarter].sequence;
        let old_span = items[quarter].sequence - items[0].sequence;
        assert!(recent_span < old_span);
        for index in 0..buffer.depth() {
            expect_decodes(&buffer, index, len);
        }
    }

    #[test]
    fn entry_cap_bounds_history_that_never_reaches_the_byte_budget() {
        let mut buffer = RewindBuffer::new(RewindConfig {
            budget_bytes: MAX_BUDGET_BYTES,
            compression: Compression::Off,
            ..RewindConfig::default()
        })
        .unwrap();
        for i in 0..MAX_ENTRIES + 50 {
            buffer.push((i + 1) as u64, (i + 1) as u64, &[0; 256]).unwrap();
        }
        assert!(buffer.memory_bytes() < MAX_BUDGET_BYTES as usize);
        assert_eq!(buffer.depth(), MAX_ENTRIES);
        assert_eq!(buffer.info(0).unwrap().frame, 1);
        assert_eq!(buffer.newest().unwrap().frame, (MAX_ENTRIES + 50) as u64);
    }

    #[test]
    fn truncate_newer_keeps_the_target_as_newest_and_releases_anchors() {
        let len = 8 * 1024;
        let mut buffer = RewindBuffer::new(RewindConfig::default()).unwrap();
        for step in 0..5 {
            push_synthetic(&mut buffer, len, step);
        }
        push_synthetic(&mut buffer, len, 1000);
        let discarded_anchor = buffer.entries.last().unwrap().anchor;
        assert_ne!(discarded_anchor, buffer.entries[2].anchor);
        buffer.truncate_newer(2);
        assert_eq!(buffer.depth(), 3);
        assert_eq!(buffer.newest().unwrap().frame, 180);
        assert!(buffer.anchors.iter().all(|anchor| anchor.id != discarded_anchor));
        push_synthetic(&mut buffer, len, 9);
        expect_decodes(&buffer, 3, len);
        assert!(buffer.anchors.iter().all(|anchor| {
            anchor.refs as usize
                == buffer.entries.iter().filter(|entry| entry.anchor == anchor.id).count()
        }));
    }

    #[test]
    fn invalid_owned_bundle_returns_the_original_upload() {
        let mut bundle =
            RewindBuffer::new(RewindConfig::default()).unwrap().write_bundle(b"live").unwrap();
        bundle[8] = 4;
        let digest = blake3::hash(&bundle[..bundle.len() - 32]);
        let end = bundle.len() - 32;
        bundle[end..].copy_from_slice(digest.as_bytes());
        let original = bundle.clone();
        let (error, returned) =
            RewindBuffer::read_bundle_owned(bundle, |_| Ok((0, 0))).unwrap_err();
        assert_eq!(error, Error::UnsupportedRewindBundleVersion);
        assert_eq!(returned, original);

        let good =
            RewindBuffer::new(RewindConfig::default()).unwrap().write_bundle(b"live").unwrap();
        let address = good.as_ptr();
        let (error, returned) =
            RewindBuffer::read_bundle_owned(good, |_| Err(Error::InvalidSaveState)).unwrap_err();
        assert_eq!(error, Error::InvalidSaveState);
        assert_eq!(returned.as_ptr(), address);
    }
}
