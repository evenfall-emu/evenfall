//! ZiGBA's padding-free, versioned state encoding.
#![no_std]
extern crate alloc;
extern crate self as evenfall_state;

use alloc::{boxed::Box, string::String, vec::Vec};
use core::ops::Range;
pub use evenfall_state_derive::Codec;

pub type Digest = [u8; 32];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidSaveState,
    UnsupportedSaveStateVersion,
    SaveStateSchemaMismatch,
    SaveStateBootModeMismatch,
    SaveStateRomMismatch,
    SaveStateBiosMismatch,
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
}

pub trait Codec {
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

impl Codec for () {
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

/// ZiGBA's narrow `uN`/`iN` representation applied recursively to a Rust field.
pub trait NarrowCodec: Codec {
    fn encode_bits(&self, s: &mut Sink<'_>, bits: u16) -> Result<()>;
    fn decode_bits(&mut self, s: &mut Source<'_>, bits: u16) -> Result<()>;
    fn schema_bits(s: &mut Sink<'_>, bits: u16) -> Result<()>;
}
macro_rules! integer {
    ($($ty:ty => $sign:literal),* $(,)?) => {$ (
        impl NarrowCodec for $ty {
            fn encode_bits(&self, s: &mut Sink<'_>, bits: u16) -> Result<()> {
                if bits > (core::mem::size_of::<Self>() * 8) as u16 { return Err(Error::InvalidSaveState); }
                let raw = (*self as u128) & if bits == 0 { 0 } else { u128::MAX >> (128 - bits) };
                let bytes = raw.to_le_bytes(); s.write_all(&bytes[..usize::from(bits.div_ceil(8))])
            }
            fn decode_bits(&mut self, s: &mut Source<'_>, bits: u16) -> Result<()> {
                if bits > (core::mem::size_of::<Self>() * 8) as u16 { return Err(Error::InvalidSaveState); }
                let mut bytes = [0u8; 16];
                s.read_all(&mut bytes[..usize::from(bits.div_ceil(8))])?;
                let raw = u128::from_le_bytes(bytes);
                if (bits == 0 && raw != 0) || (bits != 0 && raw >> bits != 0) { return Err(Error::InvalidSaveState); }
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
impl<T: Codec + Default> Codec for Option<T> {
    fn encode(&self, s: &mut Sink<'_>) -> Result<()> {
        self.is_some().encode(s)?;
        if let Some(v) = self {
            v.encode(s)?;
        }
        Ok(())
    }
    fn decode(&mut self, s: &mut Source<'_>) -> Result<()> {
        let mut present = false;
        present.decode(s)?;
        if present {
            self.get_or_insert_with(T::default).decode(s)
        } else {
            *self = None;
            Ok(())
        }
    }
    fn schema(s: &mut Sink<'_>) -> Result<()> {
        s.write_all(b"?")?;
        T::schema(s)
    }
}
impl<T: NarrowCodec + Default> NarrowCodec for Option<T> {
    fn encode_bits(&self, s: &mut Sink<'_>, b: u16) -> Result<()> {
        self.is_some().encode(s)?;
        if let Some(v) = self {
            v.encode_bits(s, b)?;
        }
        Ok(())
    }
    fn decode_bits(&mut self, s: &mut Source<'_>, b: u16) -> Result<()> {
        let mut present = false;
        present.decode(s)?;
        if present {
            self.get_or_insert_with(T::default).decode_bits(s, b)
        } else {
            *self = None;
            Ok(())
        }
    }
    fn schema_bits(s: &mut Sink<'_>, b: u16) -> Result<()> {
        s.write_all(b"?")?;
        T::schema_bits(s, b)
    }
}
impl<T: Codec, const N: usize> Codec for [T; N] {
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
