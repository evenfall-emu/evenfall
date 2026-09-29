//! Rewind word-run deltas and framed raw DEFLATE storage.
use crate::{Codec, Error, Result, Sink, Source, rewind_inflate};
use alloc::vec::Vec;
const INVALID: Error = Error::InvalidSaveState;
pub const MAX_DECODED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BUNDLE_BYTES: usize = 68 * 1024 * 1024;

/// ZiGBA's `u64le(raw_len)` followed by a raw DEFLATE stream.
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
fn same(base: &[u8], target: &[u8], i: usize) -> bool {
    let lo = i * 4;
    let mut padded = [0; 4];
    if let Some(bytes) = base.get(lo..) {
        let count = bytes.len().min(4);
        padded[..count].copy_from_slice(&bytes[..count]);
    }
    padded == target[lo..lo + 4]
}

/// Headers (`skip_words`, `copy_words`) precede changed words, then trailing bytes.
pub fn diff(base: &[u8], target: &[u8]) -> Vec<u8> {
    let words = target.len() / 4;
    let mut headers = Vec::new();
    let mut payload = Vec::new();
    let mut i = 0;
    while i < words {
        let run_start = i;
        while i < words && same(base, target, i) {
            i += 1;
        }
        if i == words {
            break;
        }
        let copy_start = i;
        while i < words {
            if !same(base, target, i) {
                i += 1;
            } else if i + 1 < words && !same(base, target, i + 1) {
                i += 2;
            } else {
                break;
            }
        }
        varint(&mut headers, copy_start - run_start);
        varint(&mut headers, i - copy_start);
        payload.extend_from_slice(&target[copy_start * 4..i * 4]);
    }
    let mut out = Vec::new();
    varint(&mut out, headers.len());
    out.extend_from_slice(&headers);
    out.extend_from_slice(&payload);
    out.extend_from_slice(&target[words * 4..]);
    out
}
pub fn patch(base: &[u8], delta: &[u8], target_len: usize) -> Result<Vec<u8>> {
    if target_len > MAX_DECODED_BYTES {
        return Err(INVALID);
    }
    let mut out = alloc::vec![0; target_len];
    let shared = base.len().min(target_len);
    out[..shared].copy_from_slice(&base[..shared]);
    let words = target_len / 4;
    let mut pos = 0;
    let header_len = get_varint(delta, &mut pos)?;
    if header_len > delta.len() - pos {
        return Err(INVALID);
    }
    let headers = &delta[pos..pos + header_len];
    let mut body = pos + header_len;
    let mut header_pos = 0;
    let mut word = 0;
    while header_pos < headers.len() {
        let skip = get_varint(headers, &mut header_pos)?;
        let copy = get_varint(headers, &mut header_pos)?;
        if skip > words - word || copy > words - word - skip || copy * 4 > delta.len() - body {
            return Err(INVALID);
        }
        word += skip;
        out[word * 4..(word + copy) * 4].copy_from_slice(&delta[body..body + copy * 4]);
        body += copy * 4;
        word += copy;
    }
    if delta.len() - body != target_len - words * 4 {
        return Err(INVALID);
    }
    out[words * 4..].copy_from_slice(&delta[body..]);
    Ok(out)
}

pub fn encode_delta(base: &[u8], target: &[u8]) -> Vec<u8> {
    let delta = diff(base, target);
    let mut out = Vec::new();
    out.extend_from_slice(&(target.len() as u32).to_le_bytes());
    out.extend_from_slice(&(delta.len() as u32).to_le_bytes());
    out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(&delta, 1));
    out
}
pub fn decode_delta(base: &[u8], stored: &[u8]) -> Result<Vec<u8>> {
    let header = stored.get(..8).ok_or(INVALID)?;
    let target_len = u32::from_le_bytes(header[..4].try_into().map_err(|_| INVALID)?) as usize;
    let delta_len = u32::from_le_bytes(header[4..].try_into().map_err(|_| INVALID)?) as usize;
    if target_len > MAX_DECODED_BYTES || delta_len > MAX_DECODED_BYTES * 2 {
        return Err(INVALID);
    }
    let mut delta = alloc::vec![0; delta_len];
    rewind_inflate::decode(&stored[8..], &mut delta)?;
    patch(base, &delta, target_len)
}

pub const DEFAULT_BUDGET_BYTES: u32 = 15 * 1024 * 1024;
pub const MAX_BUDGET_BYTES: u32 = 64 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 4096;
pub const ENTRY_OVERHEAD_BYTES: usize = 64;
pub const SPEED_SCALE_ONE: u16 = 256;

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
    Delta = 2,
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
pub struct Entry {
    pub frame: u64,
    pub cycle: u64,
    pub sequence: u64,
    pub state: Vec<u8>,
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
    stored: Vec<u8>,
    refs: u32,
}

/// Storage policy only. The session decides when it is safe to snapshot or restore.
#[derive(Debug)]
pub struct RewindBuffer {
    pub config: RewindConfig,
    entries: Vec<Entry>,
    anchors: Vec<Anchor>,
    current_anchor_raw: Option<Vec<u8>>,
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
            current_anchor_raw: None,
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
        self.current_anchor_raw = None;
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
            let stored = encode_delta(base, raw);
            if self
                .anchor_index(self.current_anchor)
                .is_some_and(|i| stored.len() * 2 <= self.anchors[i].stored.len())
            {
                return stored;
            }
        }
        let anchor_stored = deflate(raw, 6);
        let stored = encode_delta(raw, raw);
        self.current_anchor_raw = Some(raw.to_vec());
        while self.anchor_index(self.next_anchor).is_some() {
            self.next_anchor = self.next_anchor.wrapping_add(1);
        }
        self.current_anchor = self.next_anchor;
        self.next_anchor = self.next_anchor.wrapping_add(1);
        self.stored_bytes += anchor_stored.len();
        self.anchors.push(Anchor { id: self.current_anchor, stored: anchor_stored, refs: 0 });
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
                (state, Encoding::Delta, id)
            }
        };
        self.stored_bytes += state.len() + ENTRY_OVERHEAD_BYTES;
        self.entries.push(Entry {
            frame,
            cycle,
            sequence: self.next_sequence,
            state,
            encoding,
            anchor,
        });
        self.next_sequence += 1;
        self.last_capture_frame = Some(frame);
        self.enforce_budget();
        Ok(())
    }
    pub fn decode(&self, index: usize) -> Result<Vec<u8>> {
        let entry = self.entries.get(index).ok_or(Error::InvalidRewindSteps)?;
        match entry.encoding {
            Encoding::Raw => Ok(entry.state.clone()),
            Encoding::Deflate => inflate(&entry.state),
            Encoding::Delta => {
                if entry.anchor == self.current_anchor
                    && let Some(raw) = &self.current_anchor_raw
                {
                    return decode_delta(raw, &entry.state);
                }
                let i = self.anchor_index(entry.anchor).ok_or(INVALID)?;
                decode_delta(&inflate(&self.anchors[i].stored)?, &entry.state)
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
    }
    fn bytes_freed_by(&self, entry: &Entry) -> usize {
        let mut bytes = entry.state.len() + ENTRY_OVERHEAD_BYTES;
        if entry.encoding == Encoding::Delta
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
        if entry.encoding == Encoding::Delta {
            self.release_anchor(entry.anchor);
        }
    }
    fn free_anchor(&mut self, index: usize) {
        let anchor = self.anchors.swap_remove(index);
        self.stored_bytes -= anchor.stored.len();
        if anchor.id == self.current_anchor {
            self.current_anchor_raw = None;
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
                self.entries.iter().position(|e| e.encoding == Encoding::Delta && e.anchor == id)
            && let Ok(raw) = self.decode(i)
        {
            let stored = deflate(&raw, 1);
            self.stored_bytes -= self.entries[i].state.len();
            self.entries[i].state = stored;
            self.entries[i].encoding = Encoding::Deflate;
            self.stored_bytes += self.entries[i].state.len();
            self.free_anchor(index);
        }
    }

    /// Session-owned live state precedes the policy, anchors and entries.
    pub fn write_bundle(&self, live: &[u8]) -> Result<Vec<u8>> {
        if live.len() > MAX_DECODED_BYTES {
            return Err(INVALID);
        }
        let mut out = Vec::new();
        {
            let mut sink = Sink::new(Some(&mut out), None);
            sink.write_all(b"ZGBRWD\0\0")?;
            1u32.encode(&mut sink)?;
            (live.len() as u32).encode(&mut sink)?;
            sink.write_all(live)?;
            self.write_history(&mut sink)?;
        }
        let hash = blake3::hash(&out);
        out.extend_from_slice(hash.as_bytes());
        Ok(out)
    }
    fn write_history(&self, sink: &mut Sink<'_>) -> Result<()> {
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
            sink.write_all(&anchor.stored)?;
        }
        for entry in &self.entries {
            entry.frame.encode(sink)?;
            entry.cycle.encode(sink)?;
            entry.sequence.encode(sink)?;
            (entry.encoding as u8).encode(sink)?;
            entry.anchor.encode(sink)?;
            (entry.state.len() as u32).encode(sink)?;
            sink.write_all(&entry.state)?;
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
        if &magic != b"ZGBRWD\0\0" {
            return Err(Error::InvalidRewindBundle);
        }
        if read_value::<u32>(&mut source)? != 1 {
            return Err(Error::UnsupportedRewindBundleVersion);
        }
        let live = read_bytes(&mut source, MAX_DECODED_BYTES)?;
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
            let stored = read_bytes(&mut source, MAX_DECODED_BYTES * 2 + 64 * 1024)?;
            history.stored_bytes += stored.len();
            history.anchors.push(Anchor { id, stored, refs: 0 });
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
            let stored = read_bytes(
                &mut source,
                if encoding == Encoding::Raw {
                    MAX_DECODED_BYTES
                } else {
                    MAX_DECODED_BYTES * 2 + 64 * 1024
                },
            )?;
            if encoding == Encoding::Delta {
                let i = history.anchor_index(anchor).ok_or(Error::InvalidRewindBundle)?;
                history.anchors[i].refs += 1;
            }
            history.stored_bytes += stored.len() + ENTRY_OVERHEAD_BYTES;
            history.entries.push(Entry { frame, cycle, sequence, encoding, anchor, state: stored });
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
            history.current_anchor_raw = Some(inflate(&history.anchors[i].stored)?);
        } else if history.anchor_index(history.current_anchor).is_some() {
            return Err(Error::InvalidRewindBundle);
        }
        for anchor in &history.anchors {
            validate(&inflate(&anchor.stored)?)?;
        }
        for (i, entry) in history.entries.iter().enumerate() {
            let labels = validate(&history.decode(i)?)?;
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
fn read_bytes(source: &mut Source<'_>, limit: usize) -> Result<Vec<u8>> {
    let len = read_value::<u32>(source)? as usize;
    if len > limit || len > source.input.len() {
        return Err(Error::InvalidRewindBundle);
    }
    let mut bytes = alloc::vec![0; len];
    source.read_all(&mut bytes)?;
    Ok(bytes)
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
        2 => Ok(Encoding::Delta),
        _ => Err(Error::InvalidRewindBundle),
    }
}
