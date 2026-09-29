//! Lanes of 16-bit pixel data for the layer-major compositor. One kernel is
//! written against [`Lanes`] and instantiated for eight-lane vectors
//! ([`Sse2`] on x86-64, `Simd128` on WebAssembly built with `simd128`) or
//! [`Scalar`], one lane, on every other target. LLVM's auto-vectorization of
//! the same loops proved fragile: small source changes turned selects into
//! per-lane branches or address selects. The scalar lane is the reference
//! for the vector lanes.
//!
//! Masks are all-ones (true) or all-zeros (false) per lane.

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
#[cfg(any(
    test,
    not(any(target_arch = "x86_64", all(target_arch = "wasm32", target_feature = "simd128")))
))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Scalar(pub u16);

#[cfg(any(
    test,
    not(any(target_arch = "x86_64", all(target_arch = "wasm32", target_feature = "simd128")))
))]
impl Scalar {
    fn mask(value: bool) -> Self {
        Self(if value { 0xFFFF } else { 0 })
    }
}

#[cfg(any(
    test,
    not(any(target_arch = "x86_64", all(target_arch = "wasm32", target_feature = "simd128")))
))]
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

#[cfg(target_arch = "x86_64")]
pub(super) use sse2::Sse2;

#[cfg(target_arch = "x86_64")]
mod sse2 {
    use super::Lanes;
    use core::arch::x86_64::*;

    /// Eight 16-bit lanes in one SSE2 register.
    #[derive(Clone, Copy)]
    pub(in crate::video) struct Sse2(__m128i);

    /// SSE2 intrinsics are `unsafe` only because they require the `sse2`
    /// target feature, which every x86-64 target enables.
    macro_rules! sse2 {
        ($intrinsic:expr) => {
            // SAFETY: SSE2 is part of the x86-64 baseline; memory operands
            // are fixed-size array references checked by the callers.
            unsafe { $intrinsic }
        };
    }

    impl Lanes for Sse2 {
        const WIDTH: usize = 8;
        #[inline(always)]
        fn load(source: &[u16], at: usize) -> Self {
            let lanes: &[u16; 8] = source[at..at + 8].try_into().unwrap();
            Self(sse2!(_mm_loadu_si128(lanes.as_ptr().cast())))
        }
        #[inline(always)]
        fn load_bytes(source: &[u8], at: usize) -> Self {
            let bytes: &[u8; 8] = source[at..at + 8].try_into().unwrap();
            let low = sse2!(_mm_loadl_epi64(bytes.as_ptr().cast()));
            Self(sse2!(_mm_unpacklo_epi8(low, _mm_setzero_si128())))
        }
        #[inline(always)]
        fn store(self, target: &mut [u16], at: usize) {
            let lanes: &mut [u16; 8] = (&mut target[at..at + 8]).try_into().unwrap();
            sse2!(_mm_storeu_si128(lanes.as_mut_ptr().cast(), self.0));
        }
        #[inline(always)]
        fn splat(value: u16) -> Self {
            Self(sse2!(_mm_set1_epi16(value as i16)))
        }
        #[inline(always)]
        fn and(self, other: Self) -> Self {
            Self(sse2!(_mm_and_si128(self.0, other.0)))
        }
        #[inline(always)]
        fn or(self, other: Self) -> Self {
            Self(sse2!(_mm_or_si128(self.0, other.0)))
        }
        #[inline(always)]
        fn and_not(self, other: Self) -> Self {
            Self(sse2!(_mm_andnot_si128(self.0, other.0)))
        }
        #[inline(always)]
        fn add(self, other: Self) -> Self {
            Self(sse2!(_mm_add_epi16(self.0, other.0)))
        }
        #[inline(always)]
        fn sub(self, other: Self) -> Self {
            Self(sse2!(_mm_sub_epi16(self.0, other.0)))
        }
        #[inline(always)]
        fn mul(self, other: Self) -> Self {
            Self(sse2!(_mm_mullo_epi16(self.0, other.0)))
        }
        #[inline(always)]
        fn min(self, other: Self) -> Self {
            Self(sse2!(_mm_min_epi16(self.0, other.0)))
        }
        #[inline(always)]
        fn shr<const BITS: i32>(self) -> Self {
            Self(sse2!(_mm_srli_epi16::<BITS>(self.0)))
        }
        #[inline(always)]
        fn shl<const BITS: i32>(self) -> Self {
            Self(sse2!(_mm_slli_epi16::<BITS>(self.0)))
        }
        #[inline(always)]
        fn eq(self, other: Self) -> Self {
            Self(sse2!(_mm_cmpeq_epi16(self.0, other.0)))
        }
        #[inline(always)]
        fn gt(self, other: Self) -> Self {
            Self(sse2!(_mm_cmpgt_epi16(self.0, other.0)))
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub(super) use simd128::Simd128;

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod simd128 {
    use super::Lanes;
    use core::arch::wasm32::*;

    /// Eight 16-bit lanes in one WebAssembly `v128`.
    #[derive(Clone, Copy)]
    pub(in crate::video) struct Simd128(v128);

    impl Lanes for Simd128 {
        const WIDTH: usize = 8;
        #[inline(always)]
        fn load(source: &[u16], at: usize) -> Self {
            let lanes: &[u16; 8] = source[at..at + 8].try_into().unwrap();
            // SAFETY: `lanes` is a valid 16-byte array; v128 loads need no
            // alignment.
            Self(unsafe { v128_load(lanes.as_ptr().cast()) })
        }
        #[inline(always)]
        fn load_bytes(source: &[u8], at: usize) -> Self {
            let bytes: &[u8; 8] = source[at..at + 8].try_into().unwrap();
            // SAFETY: `bytes` is a valid 8-byte array.
            Self(unsafe { u16x8_load_extend_u8x8(bytes.as_ptr()) })
        }
        #[inline(always)]
        fn store(self, target: &mut [u16], at: usize) {
            let lanes: &mut [u16; 8] = (&mut target[at..at + 8]).try_into().unwrap();
            // SAFETY: `lanes` is a valid, exclusively borrowed 16-byte array.
            unsafe { v128_store(lanes.as_mut_ptr().cast(), self.0) }
        }
        #[inline(always)]
        fn splat(value: u16) -> Self {
            Self(u16x8_splat(value))
        }
        #[inline(always)]
        fn and(self, other: Self) -> Self {
            Self(v128_and(self.0, other.0))
        }
        #[inline(always)]
        fn or(self, other: Self) -> Self {
            Self(v128_or(self.0, other.0))
        }
        #[inline(always)]
        fn and_not(self, other: Self) -> Self {
            // WebAssembly's andnot clears the bits of its second operand.
            Self(v128_andnot(other.0, self.0))
        }
        #[inline(always)]
        fn add(self, other: Self) -> Self {
            Self(i16x8_add(self.0, other.0))
        }
        #[inline(always)]
        fn sub(self, other: Self) -> Self {
            Self(i16x8_sub(self.0, other.0))
        }
        #[inline(always)]
        fn mul(self, other: Self) -> Self {
            Self(i16x8_mul(self.0, other.0))
        }
        #[inline(always)]
        fn min(self, other: Self) -> Self {
            Self(i16x8_min(self.0, other.0))
        }
        #[inline(always)]
        fn shr<const BITS: i32>(self) -> Self {
            Self(u16x8_shr(self.0, BITS as u32))
        }
        #[inline(always)]
        fn shl<const BITS: i32>(self) -> Self {
            Self(i16x8_shl(self.0, BITS as u32))
        }
        #[inline(always)]
        fn eq(self, other: Self) -> Self {
            Self(i16x8_eq(self.0, other.0))
        }
        #[inline(always)]
        fn gt(self, other: Self) -> Self {
            Self(i16x8_gt(self.0, other.0))
        }
        #[inline(always)]
        fn select(mask: Self, on: Self, off: Self) -> Self {
            Self(v128_bitselect(on.0, off.0, mask.0))
        }
        #[inline(always)]
        fn any(self, bits: u16) -> Self {
            Self(i16x8_ne(v128_and(self.0, u16x8_splat(bits)), u16x8_splat(0)))
        }
    }
}
