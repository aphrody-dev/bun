//! The loops over whole runs. Each returns the length of the slice where it finds nothing.

use core::mem::MaybeUninit;

#[bun_simd::simd]
mod simd {
    use super::MaybeUninit;
    use bun_simd::prelude::*;

    #[kernel]
    pub(crate) fn first_non_ascii(bytes: &[u8]) -> usize {
        over(bytes)
            .position(|v| v.simd_gt(0x7f))
            .unwrap_or(bytes.len())
    }

    #[kernel]
    pub(crate) fn count_non_ascii(bytes: &[u8]) -> usize {
        over(bytes).count(|v| v.simd_gt(0x7f))
    }

    /// One unit for each byte that starts a character, and one more for each that starts four bytes.
    #[kernel]
    pub(crate) fn utf16_length_from_utf8(bytes: &[u8]) -> usize {
        over(bytes).count(|v| (v & 0xc0).simd_ne(0x80)) + over(bytes).count(|v| v.simd_ge(0xf0))
    }

    #[kernel]
    pub(crate) fn first_of(bytes: &[u8], byte: u8) -> usize {
        over(bytes)
            .position(|v| v.simd_eq(byte))
            .unwrap_or(bytes.len())
    }

    #[kernel]
    pub(crate) fn first_non_ascii16(units: &[u16]) -> usize {
        over(units)
            .position(|v| v.simd_gt(0x7f))
            .unwrap_or(units.len())
    }

    #[kernel]
    pub(crate) fn first_surrogate(units: &[u16]) -> usize {
        over(units)
            .position(|v| (v & 0xf800).simd_eq(0xd800))
            .unwrap_or(units.len())
    }

    /// Two bytes for each surrogate, as if it were half of a pair.
    #[kernel]
    pub(crate) fn utf8_length_from_utf16(units: &[u16]) -> usize {
        units.len()
            + over(units).count(|v| v.simd_gt(0x7f))
            + over(units).count(|v| v.simd_gt(0x7ff))
            - over(units).count(|v| (v & 0xf800).simd_eq(0xd800))
    }

    #[kernel]
    pub(crate) fn widen(from: &[u8], to: &mut [MaybeUninit<u16>]) {
        over(from).map_into(to, |v| v.cast::<u16>())
    }

    /// Only for units below 0x100.
    #[kernel]
    pub(crate) fn narrow(from: &[u16], to: &mut [MaybeUninit<u8>]) {
        over(from).map_into(to, |v| v.cast::<u8>())
    }
}

pub(crate) use simd::*;

/// What the scalar loops of simdutf check before they hand a run to a kernel.
#[inline(always)]
pub(crate) fn sixteen_ascii(bytes: &[u8]) -> bool {
    match bytes.first_chunk::<16>() {
        Some(chunk) => u128::from_ne_bytes(*chunk) & 0x8080_8080_8080_8080_8080_8080_8080_8080 == 0,
        None => false,
    }
}

#[inline(always)]
pub(crate) fn four_ascii16(units: &[u16]) -> bool {
    match units.first_chunk::<4>() {
        Some(&[a, b, c, d]) => (a | b | c | d) < 0x80,
        None => false,
    }
}
