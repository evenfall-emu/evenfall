//! Evenfall's versioned state encoding: little-endian, minimal-width values in
//! declaration order, with a fixed-size slot for every option and union.
#![no_std]
extern crate alloc;
extern crate self as evenfall_state;

pub mod envelope;
pub mod rewind;
pub mod rewind_inflate;

use alloc::{boxed::Box, string::String, vec::Vec};
use core::ops::Range;
pub use evenfall_state_derive::Codec;

pub type Digest = [u8; 32];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidSaveState,
    OutOfMemory,
    UnsupportedSaveStateVersion,
    SaveStateSchemaMismatch,
    SaveStateBootModeMismatch,
    SaveStateRomMismatch,
    SaveStateBiosMismatch,
    InvalidRewindConfig,
    InvalidRewindSpeed,
    InvalidRewindBundle,
    UnsupportedRewindBundleVersion,
    RewindBundleTooLarge,
    InvalidRewindSteps,
    RewindSequenceExhausted,
}
pub type Result<T> = core::result::Result<T, Error>;

/// A writer that may count only, write, hash, and optionally record field offsets.
pub struct Sink<'a> {
    pub output: Option<&'a mut Vec<u8>>,
    pub hasher: Option<&'a mut blake3::Hasher>,
    pub count: u64,
    pub trace: Option<&'a mut Vec<(String, Range<u64>)>>,
    path: String,
}
impl<'a> Sink<'a> {
    pub fn new(output: Option<&'a mut Vec<u8>>, hasher: Option<&'a mut blake3::Hasher>) -> Self {
        Self { output, hasher, count: 0, trace: None, path: String::new() }
    }
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        let count = self.count.checked_add(bytes.len() as u64).ok_or(Error::InvalidSaveState)?;
        if let Some(out) = &mut self.output {
            out.extend_from_slice(bytes);
        }
        if let Some(hash) = &mut self.hasher {
            hash.update(bytes);
        }
        self.count = count;
        Ok(())
    }
    /// Zero-fills a slot of `len` bytes that started at count `start`.
    pub fn pad_slot(&mut self, start: u64, len: usize) -> Result<()> {
        let written = self.count - start;
        let mut rest = (len as u64).checked_sub(written).ok_or(Error::InvalidSaveState)? as usize;
        while rest > 0 {
            let chunk = rest.min(ZEROS.len());
            self.write_all(&ZEROS[..chunk])?;
            rest -= chunk;
        }
        Ok(())
    }
    pub fn field(&mut self, name: &str, f: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        if self.trace.is_none() {
            return f(self);
        }
        let old = self.path.len();
        if old != 0 {
            self.path.push('.');
        }
        self.path.push_str(name);
        let start = self.count;
        let result = f(self);
        if result.is_ok() {
            self.trace.as_mut().unwrap().push((self.path.clone(), start..self.count));
        }
        self.path.truncate(old);
        result
    }
}

pub struct Source<'a> {
    pub input: &'a [u8],
    pub hasher: Option<&'a mut blake3::Hasher>,
    pub limit: Option<u64>,
    pub count: u64,
}
impl<'a> Source<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self { input, hasher: None, limit: None, count: 0 }
    }
    pub fn read_all(&mut self, bytes: &mut [u8]) -> Result<()> {
        let count = self.count.checked_add(bytes.len() as u64).ok_or(Error::InvalidSaveState)?;
        if count > self.limit.unwrap_or(u64::MAX) || bytes.len() > self.input.len() {
            return Err(Error::InvalidSaveState);
        }
        let (head, tail) = self.input.split_at(bytes.len());
        bytes.copy_from_slice(head);
        if let Some(hash) = &mut self.hasher {
            hash.update(head);
        }
        self.input = tail;
        self.count = count;
        Ok(())
    }
    /// Reads the rest of a slot of `len` bytes that started at count
    /// `start`, which must be zero: an encoding has exactly one form.
    pub fn skip_slot(&mut self, start: u64, len: usize) -> Result<()> {
        let read = self.count - start;
        let mut rest = (len as u64).checked_sub(read).ok_or(Error::InvalidSaveState)? as usize;
        let mut buffer = [0; ZEROS.len()];
        while rest > 0 {
            let chunk = &mut buffer[..rest.min(ZEROS.len())];
            self.read_all(chunk)?;
            if chunk.iter().any(|&byte| byte != 0) {
                return Err(Error::InvalidSaveState);
            }
            rest -= chunk.len();
        }
        Ok(())
    }
}

const ZEROS: [u8; 64] = [0; 64];

pub trait Codec {
    /// The length of every value's encoding, or `None` when it depends on the
    /// value. Options and unions pad their payloads to a fixed slot, so only
    /// types with a fixed length may appear inside them.
    const ENCODED_LEN: Option<usize>;
    fn encode(&self, sink: &mut Sink<'_>) -> Result<()>;
    fn decode(&mut self, source: &mut Source<'_>) -> Result<()>;
    fn schema(sink: &mut Sink<'_>) -> Result<()>;
    const FIELD_COUNT: u32 = 0;
    fn schema_fields(_sink: &mut Sink<'_>) -> Result<()> {
        Err(Error::InvalidSaveState)
    }
}

pub fn schema_name(sink: &mut Sink<'_>, name: &str) -> Result<()> {
    (name.len() as u32).encode(sink)?;
    sink.write_all(name.as_bytes())
}
pub fn schema_field<T: Codec>(sink: &mut Sink<'_>, name: &str) -> Result<()> {
    schema_name(sink, name)?;
    T::schema(sink)
}

/// Const arithmetic on encoded lengths, for derived impls.
#[doc(hidden)]
pub mod len {
    pub const fn sum(a: Option<usize>, b: Option<usize>) -> Option<usize> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a + b),
            _ => None,
        }
    }
    /// A payload's length, which must be fixed.
    pub const fn fixed(len: Option<usize>) -> usize {
        match len {
            Some(len) => len,
            None => panic!("an option or union payload needs a fixed encoded length"),
        }
    }
    pub const fn max(a: usize, b: usize) -> usize {
        if a > b { a } else { b }
    }
    pub const fn bytes(bits: u16) -> usize {
        (bits as usize).div_ceil(8)
    }
}

impl Codec for () {
    const ENCODED_LEN: Option<usize> = Some(0);
    fn encode(&self, _: &mut Sink<'_>) -> Result<()> {
        Ok(())
    }
    fn decode(&mut self, _: &mut Source<'_>) -> Result<()> {
        Ok(())
    }
    fn schema(s: &mut Sink<'_>) -> Result<()> {
        s.write_all(b"0")
    }
}
impl Codec for bool {
    const ENCODED_LEN: Option<usize> = Some(1);
    fn encode(&self, s: &mut Sink<'_>) -> Result<()> {
        s.write_all(&[u8::from(*self)])
    }
    fn decode(&mut self, s: &mut Source<'_>) -> Result<()> {
        let mut b = [0];
        s.read_all(&mut b)?;
        *self = match b[0] {
            0 => false,
            1 => true,
            _ => return Err(Error::InvalidSaveState),
        };
        Ok(())
    }
    fn schema(s: &mut Sink<'_>) -> Result<()> {
        s.write_all(b"b")
    }
}

/// Integers of `bits` bits in their smallest little-endian byte width,
/// applied recursively to a Rust field.
pub trait NarrowCodec: Codec {
    /// Integers in a value, each encoded in `len::bytes(bits)` bytes.
    const INTEGERS: usize;
    /// Other bytes in a value: the presence flags of options.
    const FLAGS: usize;
    fn encode_bits(&self, s: &mut Sink<'_>, bits: u16) -> Result<()>;
    fn decode_bits(&mut self, s: &mut Source<'_>, bits: u16) -> Result<()>;
    fn schema_bits(s: &mut Sink<'_>, bits: u16) -> Result<()>;
}
/// The encoded length of a `T` narrowed to `bits`.
pub const fn narrow_len<T: NarrowCodec>(bits: u16) -> usize {
    T::INTEGERS * len::bytes(bits) + T::FLAGS
}
macro_rules! integer {
    ($($ty:ty => $sign:literal),* $(,)?) => {$ (
        impl NarrowCodec for $ty {
            const INTEGERS: usize = 1;
            const FLAGS: usize = 0;
            fn encode_bits(&self, s: &mut Sink<'_>, bits: u16) -> Result<()> {
                if bits > (core::mem::size_of::<Self>() * 8) as u16 { return Err(Error::InvalidSaveState); }
                if bits < (core::mem::size_of::<Self>() * 8) as u16 {
                    let valid = if bits == 0 { *self == 0 } else if $sign == b'i' {
                        let shift = bits - 1;
                        (*self as i128) >= -(1i128 << shift) && (*self as i128) < (1i128 << shift)
                    } else { (*self as u128) >> bits == 0 };
                    if !valid { return Err(Error::InvalidSaveState); }
                }
                let raw = (*self as u128) & if bits == 0 { 0 } else { u128::MAX >> (128 - bits) };
                let bytes = raw.to_le_bytes(); s.write_all(&bytes[..usize::from(bits.div_ceil(8))])
            }
            fn decode_bits(&mut self, s: &mut Source<'_>, bits: u16) -> Result<()> {
                if bits > (core::mem::size_of::<Self>() * 8) as u16 { return Err(Error::InvalidSaveState); }
                let mut bytes = [0u8; 16];
                s.read_all(&mut bytes[..usize::from(bits.div_ceil(8))])?;
                let raw = u128::from_le_bytes(bytes);
                if bits < 128 && raw >> bits != 0 { return Err(Error::InvalidSaveState); }
                // Sign-extend the narrow two's-complement representation.
                *self = if $sign == b'i' && bits > 0 && bits < 128 && raw & (1u128 << (bits - 1)) != 0 {
                    (raw | (u128::MAX << bits)) as Self
                } else { raw as Self };
                Ok(())
            }
            fn schema_bits(s: &mut Sink<'_>, bits: u16) -> Result<()> {
                s.write_all(&[$sign])?; bits.encode(s)
            }
        }
        impl Codec for $ty {
            const ENCODED_LEN: Option<usize> = Some(core::mem::size_of::<Self>());
            fn encode(&self, s: &mut Sink<'_>) -> Result<()> { self.encode_bits(s, (core::mem::size_of::<Self>() * 8) as u16) }
            fn decode(&mut self, s: &mut Source<'_>) -> Result<()> { self.decode_bits(s, (core::mem::size_of::<Self>() * 8) as u16) }
            fn schema(s: &mut Sink<'_>) -> Result<()> { Self::schema_bits(s, (core::mem::size_of::<Self>() * 8) as u16) }
        }
    )* };
}
integer!(u8 => b'u', u16 => b'u', u32 => b'u', u64 => b'u', u128 => b'u', i8 => b'i', i16 => b'i', i32 => b'i', i64 => b'i', i128 => b'i');
macro_rules! size_int {
    ($ty:ty, $wide:ty, $marker:literal) => {
        impl Codec for $ty {
            const ENCODED_LEN: Option<usize> = Some(core::mem::size_of::<$wide>());
            fn encode(&self, s: &mut Sink<'_>) -> Result<()> {
                (*self as $wide).encode(s)
            }
            fn decode(&mut self, s: &mut Source<'_>) -> Result<()> {
                let mut v: $wide = 0;
                v.decode(s)?;
                *self = v.try_into().map_err(|_| Error::InvalidSaveState)?;
                Ok(())
            }
            fn schema(s: &mut Sink<'_>) -> Result<()> {
                s.write_all($marker)
            }
        }
    };
}
size_int!(usize, u64, b"z");
size_int!(isize, i64, b"Z");
macro_rules! float {
    ($ty:ty,$bits:ty,$width:expr) => {
        impl Codec for $ty {
            const ENCODED_LEN: Option<usize> = Some(core::mem::size_of::<$bits>());
            fn encode(&self, s: &mut Sink<'_>) -> Result<()> {
                self.to_bits().encode(s)
            }
            fn decode(&mut self, s: &mut Source<'_>) -> Result<()> {
                let mut b: $bits = 0;
                b.decode(s)?;
                *self = Self::from_bits(b);
                Ok(())
            }
            fn schema(s: &mut Sink<'_>) -> Result<()> {
                s.write_all(b"f")?;
                $width.encode(s)
            }
        }
    };
}
float!(f32, u32, 32u16);
float!(f64, u64, 64u16);
/// A presence byte, then the value or zeros in a slot of the value's length.
impl<T: Codec + Default> Codec for Option<T> {
    const ENCODED_LEN: Option<usize> = Some(1 + len::fixed(T::ENCODED_LEN));
    fn encode(&self, s: &mut Sink<'_>) -> Result<()> {
        let start = s.count;
        self.is_some().encode(s)?;
        if let Some(v) = self {
            v.encode(s)?;
        }
        s.pad_slot(start, len::fixed(Self::ENCODED_LEN))
    }
    fn decode(&mut self, s: &mut Source<'_>) -> Result<()> {
        let start = s.count;
        let mut present = false;
        present.decode(s)?;
        if present {
            self.get_or_insert_with(T::default).decode(s)?;
        } else {
            *self = None;
        }
        s.skip_slot(start, len::fixed(Self::ENCODED_LEN))
    }
    fn schema(s: &mut Sink<'_>) -> Result<()> {
        s.write_all(b"?")?;
        T::schema(s)
    }
}
impl<T: NarrowCodec + Default> NarrowCodec for Option<T> {
    const INTEGERS: usize = T::INTEGERS;
    const FLAGS: usize = 1 + T::FLAGS;
    fn encode_bits(&self, s: &mut Sink<'_>, b: u16) -> Result<()> {
        let start = s.count;
        self.is_some().encode(s)?;
        if let Some(v) = self {
            v.encode_bits(s, b)?;
        }
        s.pad_slot(start, narrow_len::<Self>(b))
    }
    fn decode_bits(&mut self, s: &mut Source<'_>, b: u16) -> Result<()> {
        let start = s.count;
        let mut present = false;
        present.decode(s)?;
        if present {
            self.get_or_insert_with(T::default).decode_bits(s, b)?;
        } else {
            *self = None;
        }
        s.skip_slot(start, narrow_len::<Self>(b))
    }
    fn schema_bits(s: &mut Sink<'_>, b: u16) -> Result<()> {
        s.write_all(b"?")?;
        T::schema_bits(s, b)
    }
}
impl<T: Codec, const N: usize> Codec for [T; N] {
    const ENCODED_LEN: Option<usize> = match T::ENCODED_LEN {
        Some(len) => Some(len * N),
        None => None,
    };
    fn encode(&self, s: &mut Sink<'_>) -> Result<()> {
        for v in self {
            v.encode(s)?;
        }
        Ok(())
    }
    fn decode(&mut self, s: &mut Source<'_>) -> Result<()> {
        for v in self {
            v.decode(s)?;
        }
        Ok(())
    }
    fn schema(s: &mut Sink<'_>) -> Result<()> {
        s.write_all(b"a")?;
        (N as u64).encode(s)?;
        T::schema(s)
    }
}
impl<T: NarrowCodec, const N: usize> NarrowCodec for [T; N] {
    const INTEGERS: usize = T::INTEGERS * N;
    const FLAGS: usize = T::FLAGS * N;
    fn encode_bits(&self, s: &mut Sink<'_>, b: u16) -> Result<()> {
        for v in self {
            v.encode_bits(s, b)?;
        }
        Ok(())
    }
    fn decode_bits(&mut self, s: &mut Source<'_>, b: u16) -> Result<()> {
        for v in self {
            v.decode_bits(s, b)?;
        }
        Ok(())
    }
    fn schema_bits(s: &mut Sink<'_>, b: u16) -> Result<()> {
        s.write_all(b"a")?;
        (N as u64).encode(s)?;
        T::schema_bits(s, b)
    }
}
impl<T: Codec, const N: usize> Codec for Box<[T; N]> {
    const ENCODED_LEN: Option<usize> = <[T; N]>::ENCODED_LEN;
    fn encode(&self, s: &mut Sink<'_>) -> Result<()> {
        (**self).encode(s)
    }
    fn decode(&mut self, s: &mut Source<'_>) -> Result<()> {
        (**self).decode(s)
    }
    fn schema(s: &mut Sink<'_>) -> Result<()> {
        <[T; N]>::schema(s)
    }
}
impl<T: NarrowCodec, const N: usize> NarrowCodec for Box<[T; N]> {
    const INTEGERS: usize = <[T; N]>::INTEGERS;
    const FLAGS: usize = <[T; N]>::FLAGS;
    fn encode_bits(&self, s: &mut Sink<'_>, b: u16) -> Result<()> {
        (**self).encode_bits(s, b)
    }
    fn decode_bits(&mut self, s: &mut Source<'_>, b: u16) -> Result<()> {
        (**self).decode_bits(s, b)
    }
    fn schema_bits(s: &mut Sink<'_>, b: u16) -> Result<()> {
        <[T; N]>::schema_bits(s, b)
    }
}
