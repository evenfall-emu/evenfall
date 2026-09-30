//! Lanes of 16-bit pixel data for the layer-major compositor. One kernel is
//! written against [`Lanes`] and instantiated for [`Native`]: eight lanes of
//! [`wide::i16x8`] where the `wide` crate has a vector backend (SSE2 on
//! x86-64, NEON on AArch64, `simd128` on WebAssembly built with it), and
//! [`Scalar`], one lane, elsewhere. LLVM's auto-vectorization of the same
//! loops proved fragile: small source changes turned selects into per-lane
//! branches or address selects. The scalar lane is the reference for the
//! vector lanes.
//!
//! Masks are all-ones (true) or all-zeros (false) per lane.

use wide::{i16x8, u8x16, u16x8};

cfg_select! {
    any(
        target_feature = "sse2",
        target_feature = "simd128",
        all(target_arch = "aarch64", target_feature = "neon"),
    ) => {
        /// The lanes the compositor runs on: `wide`'s vector backend.
        pub(super) type Native = i16x8;
    }
    _ => {
        /// The lanes the compositor runs on. Without a vector backend `wide`
        /// uses eight-element arrays, which measured slower than one lane.
        pub(super) type Native = Scalar;
    }
}

pub(super) trait Lanes: Copy {
    /// Pixels per lane group.
    const WIDTH: usize;
    fn load(source: &[u16], at: usize) -> Self;
    /// Widen `WIDTH` bytes to 16-bit lanes.
    fn load_bytes(source: &[u8], at: usize) -> Self;
    fn store(self, target: &mut [u16], at: usize);
    fn splat(value: u16) -> Self;
    fn and(self, other: Self) -> Self;
    fn or(self, other: Self) -> Self;
    /// `!self & other`.
    fn and_not(self, other: Self) -> Self;
    fn add(self, other: Self) -> Self;
    fn sub(self, other: Self) -> Self;
    /// Low 16 bits of the product.
    fn mul(self, other: Self) -> Self;
    /// Minimum of lanes read as `i16`.
    fn min(self, other: Self) -> Self;
    /// Logical shift right.
    fn shr<const BITS: i32>(self) -> Self;
    fn shl<const BITS: i32>(self) -> Self;
    fn eq(self, other: Self) -> Self;
    /// Lanes read as `i16`.
    fn gt(self, other: Self) -> Self;
    /// `mask ? on : off` per lane.
    fn select(mask: Self, on: Self, off: Self) -> Self {
        mask.and(on).or(mask.and_not(off))
    }
    /// Mask of lanes with any bit of `bits` set.
    fn any(self, bits: u16) -> Self {
        self.and(Self::splat(bits)).eq(Self::splat(0)).and_not(Self::splat(0xFFFF))
    }
}

/// One lane: the portable form and the reference for vector lanes.
#[cfg_attr(not(test), allow(dead_code, reason = "unused where `wide` has a vector backend"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Scalar(pub u16);

#[cfg_attr(not(test), allow(dead_code, reason = "unused where `wide` has a vector backend"))]
impl Scalar {
    fn mask(value: bool) -> Self {
        Self(if value { 0xFFFF } else { 0 })
    }
}

impl Lanes for Scalar {
    const WIDTH: usize = 1;
    #[inline(always)]
    fn load(source: &[u16], at: usize) -> Self {
        Self(source[at])
    }
    #[inline(always)]
    fn load_bytes(source: &[u8], at: usize) -> Self {
        Self(u16::from(source[at]))
    }
    #[inline(always)]
    fn store(self, target: &mut [u16], at: usize) {
        target[at] = self.0;
    }
    #[inline(always)]
    fn splat(value: u16) -> Self {
        Self(value)
    }
    #[inline(always)]
    fn and(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
    #[inline(always)]
    fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    #[inline(always)]
    fn and_not(self, other: Self) -> Self {
        Self(!self.0 & other.0)
    }
    #[inline(always)]
    fn add(self, other: Self) -> Self {
        Self(self.0.wrapping_add(other.0))
    }
    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        Self(self.0.wrapping_sub(other.0))
    }
    #[inline(always)]
    fn mul(self, other: Self) -> Self {
        Self(self.0.wrapping_mul(other.0))
    }
    #[inline(always)]
    fn min(self, other: Self) -> Self {
        Self((self.0 as i16).min(other.0 as i16) as u16)
    }
    #[inline(always)]
    fn shr<const BITS: i32>(self) -> Self {
        Self(self.0 >> BITS)
    }
    #[inline(always)]
    fn shl<const BITS: i32>(self) -> Self {
        Self(self.0 << BITS)
    }
    #[inline(always)]
    fn eq(self, other: Self) -> Self {
        Self::mask(self.0 == other.0)
    }
    #[inline(always)]
    fn gt(self, other: Self) -> Self {
        Self::mask(self.0 as i16 > other.0 as i16)
    }
    #[inline(always)]
    fn select(mask: Self, on: Self, off: Self) -> Self {
        if mask.0 != 0 { on } else { off }
    }
    #[inline(always)]
    fn any(self, bits: u16) -> Self {
        Self::mask(self.0 & bits != 0)
    }
}

/// Eight lanes. `i16x8` gives the signed comparisons and minimum the kernel
/// needs; logical shifts go through the unsigned view of the same bits.
impl Lanes for i16x8 {
    const WIDTH: usize = 8;
    #[inline(always)]
    fn load(source: &[u16], at: usize) -> Self {
        let lanes: &[u16; 8] = source[at..at + 8].try_into().unwrap();
        u16x8::new(*lanes).cast_signed()
    }
    #[inline(always)]
    fn load_bytes(source: &[u8], at: usize) -> Self {
        // A zero-padded 16-byte vector widened from its low half: one load
        // and one unpack where `wide` has a vector backend.
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&source[at..at + 8]);
        u16x8::from_u8x16_low(u8x16::new(bytes)).cast_signed()
    }
    #[inline(always)]
    fn store(self, target: &mut [u16], at: usize) {
        let lanes: &mut [u16; 8] = (&mut target[at..at + 8]).try_into().unwrap();
        *lanes = self.cast_unsigned().to_array();
    }
    #[inline(always)]
    fn splat(value: u16) -> Self {
        i16x8::splat(value as i16)
    }
    #[inline(always)]
    fn and(self, other: Self) -> Self {
        self & other
    }
    #[inline(always)]
    fn or(self, other: Self) -> Self {
        self | other
    }
    #[inline(always)]
    fn and_not(self, other: Self) -> Self {
        !self & other
    }
    #[inline(always)]
    fn add(self, other: Self) -> Self {
        self + other
    }
    #[inline(always)]
    fn sub(self, other: Self) -> Self {
        self - other
    }
    #[inline(always)]
    fn mul(self, other: Self) -> Self {
        self * other
    }
    #[inline(always)]
    fn min(self, other: Self) -> Self {
        i16x8::min(self, other)
    }
    #[inline(always)]
    fn shr<const BITS: i32>(self) -> Self {
        (self.cast_unsigned() >> BITS as u32).cast_signed()
    }
    #[inline(always)]
    fn shl<const BITS: i32>(self) -> Self {
        self << BITS as u32
    }
    #[inline(always)]
    fn eq(self, other: Self) -> Self {
        self.simd_eq(other)
    }
    #[inline(always)]
    fn gt(self, other: Self) -> Self {
        self.simd_gt(other)
    }
    #[inline(always)]
    fn select(mask: Self, on: Self, off: Self) -> Self {
        mask.select(on, off)
    }
}
