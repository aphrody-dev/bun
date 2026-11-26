//! Validation and transcoding with the results of simdutf 9: the error is that of its scalar code, which its vector
//! code returns too, and the count is where the character that is wrong starts.

use crate::kernels;
use crate::simdutf::{SIMDUTFResult, Status};
use core::mem::MaybeUninit;

const fn ok(count: usize) -> SIMDUTFResult {
    SIMDUTFResult {
        status: Status::SUCCESS,
        count,
    }
}

const fn error(status: Status, count: usize) -> SIMDUTFResult {
    SIMDUTFResult { status, count }
}

#[inline(always)]
fn is_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

pub(crate) fn validate_utf8(bytes: &[u8]) -> bool {
    bun_simd::text::from_utf8(bytes).is_ok()
}

pub(crate) fn validate_utf8_with_errors(bytes: &[u8]) -> SIMDUTFResult {
    match bun_simd::text::from_utf8(bytes) {
        Ok(_) => ok(bytes.len()),
        Err(at) => validate_utf8_from(bytes, at.valid_up_to()),
    }
}

/// simdutf's `scalar::utf8::validate_with_errors`, from a character that starts at `pos`.
fn validate_utf8_from(data: &[u8], mut pos: usize) -> SIMDUTFResult {
    let len = data.len();
    while pos < len {
        if kernels::sixteen_ascii(&data[pos..]) {
            pos += 16;
            continue;
        }
        let mut byte = data[pos];
        while byte < 0x80 {
            pos += 1;
            if pos == len {
                return ok(len);
            }
            byte = data[pos];
        }
        let lead = u32::from(byte);
        let at = |i: usize| u32::from(data[pos + i] & 0x3f);
        let next = if byte & 0xe0 == 0xc0 {
            if pos + 2 > len || !is_continuation(data[pos + 1]) {
                return error(Status::TOO_SHORT, pos);
            }
            let code_point = (lead & 0x1f) << 6 | at(1);
            if !(0x80..=0x7ff).contains(&code_point) {
                return error(Status::OVERLONG, pos);
            }
            pos + 2
        } else if byte & 0xf0 == 0xe0 {
            if pos + 3 > len || !is_continuation(data[pos + 1]) || !is_continuation(data[pos + 2]) {
                return error(Status::TOO_SHORT, pos);
            }
            let code_point = (lead & 0x0f) << 12 | at(1) << 6 | at(2);
            if !(0x800..=0xffff).contains(&code_point) {
                return error(Status::OVERLONG, pos);
            }
            if (0xd800..0xe000).contains(&code_point) {
                return error(Status::SURROGATE, pos);
            }
            pos + 3
        } else if byte & 0xf8 == 0xf0 {
            if pos + 4 > len
                || !is_continuation(data[pos + 1])
                || !is_continuation(data[pos + 2])
                || !is_continuation(data[pos + 3])
            {
                return error(Status::TOO_SHORT, pos);
            }
            let code_point = (lead & 0x07) << 18 | at(1) << 12 | at(2) << 6 | at(3);
            if code_point <= 0xffff {
                return error(Status::OVERLONG, pos);
            }
            if code_point > 0x10ffff {
                return error(Status::TOO_LARGE, pos);
            }
            pos + 4
        } else if is_continuation(byte) {
            return error(Status::TOO_LONG, pos);
        } else {
            return error(Status::HEADER_BITS, pos);
        };
        pos = next;
    }
    ok(len)
}

pub(crate) fn validate_ascii(bytes: &[u8]) -> bool {
    kernels::first_non_ascii(bytes) == bytes.len()
}

pub(crate) fn validate_ascii_with_errors(bytes: &[u8]) -> SIMDUTFResult {
    match kernels::first_non_ascii(bytes) {
        at if at == bytes.len() => ok(at),
        at => error(Status::TOO_LARGE, at),
    }
}

#[inline(always)]
fn is_surrogate(unit: u16) -> bool {
    unit & 0xf800 == 0xd800
}

/// A high surrogate at `pos` that a low one follows.
#[inline(always)]
fn is_pair(units: &[u16], pos: usize) -> bool {
    units[pos].wrapping_sub(0xd800) <= 0x3ff
        && units
            .get(pos + 1)
            .is_some_and(|low| low.wrapping_sub(0xdc00) <= 0x3ff)
}

pub(crate) fn validate_utf16(units: &[u16]) -> bool {
    let mut pos = 0;
    loop {
        pos += kernels::first_surrogate(&units[pos..]);
        while pos < units.len() && is_surrogate(units[pos]) {
            if !is_pair(units, pos) {
                return false;
            }
            pos += 2;
        }
        if pos >= units.len() {
            return true;
        }
    }
}

pub(crate) fn utf8_length_from_utf16(units: &[u16]) -> usize {
    kernels::utf8_length_from_utf16(units)
}

/// Three bytes for each surrogate that is not half of a pair, which becomes U+FFFD.
pub(crate) fn utf8_length_from_utf16_with_replacement(units: &[u16]) -> usize {
    let mut length = kernels::utf8_length_from_utf16(units);
    let mut pos = kernels::first_surrogate(units);
    while pos < units.len() {
        if is_pair(units, pos) {
            pos += 2;
        } else if is_surrogate(units[pos]) {
            length += 1;
            pos += 1;
        } else {
            pos += 1 + kernels::first_surrogate(&units[pos + 1..]);
        }
    }
    length
}

pub(crate) fn utf16_length_from_utf8(bytes: &[u8]) -> usize {
    kernels::utf16_length_from_utf8(bytes)
}

pub(crate) fn utf8_length_from_latin1(bytes: &[u8]) -> usize {
    bytes.len() + kernels::count_non_ascii(bytes)
}

/// simdutf's `scalar::utf8_to_utf16::convert_with_errors`. `out` has room for what the text has in UTF-16.
pub(crate) fn utf8_to_utf16(data: &[u8], out: &mut [MaybeUninit<u16>]) -> SIMDUTFResult {
    let len = data.len();
    let (mut pos, mut written) = (0, 0);
    while pos < len {
        if kernels::sixteen_ascii(&data[pos..]) {
            let run = kernels::first_non_ascii(&data[pos..]);
            kernels::widen(&data[pos..pos + run], &mut out[written..written + run]);
            (pos, written) = (pos + run, written + run);
            continue;
        }
        let byte = data[pos];
        let lead = u32::from(byte);
        let at = |i: usize| u32::from(data[pos + i] & 0x3f);
        if byte < 0x80 {
            out[written].write(u16::from(byte));
            (pos, written) = (pos + 1, written + 1);
        } else if byte & 0xe0 == 0xc0 {
            if pos + 1 >= len || !is_continuation(data[pos + 1]) {
                return error(Status::TOO_SHORT, pos);
            }
            let code_point = (lead & 0x1f) << 6 | at(1);
            if !(0x80..=0x7ff).contains(&code_point) {
                return error(Status::OVERLONG, pos);
            }
            out[written].write(code_point as u16);
            (pos, written) = (pos + 2, written + 1);
        } else if byte & 0xf0 == 0xe0 {
            if pos + 2 >= len || !is_continuation(data[pos + 1]) || !is_continuation(data[pos + 2])
            {
                return error(Status::TOO_SHORT, pos);
            }
            let code_point = (lead & 0x0f) << 12 | at(1) << 6 | at(2);
            if !(0x800..=0xffff).contains(&code_point) {
                return error(Status::OVERLONG, pos);
            }
            if (0xd800..0xe000).contains(&code_point) {
                return error(Status::SURROGATE, pos);
            }
            out[written].write(code_point as u16);
            (pos, written) = (pos + 3, written + 1);
        } else if byte & 0xf8 == 0xf0 {
            if pos + 3 >= len
                || !is_continuation(data[pos + 1])
                || !is_continuation(data[pos + 2])
                || !is_continuation(data[pos + 3])
            {
                return error(Status::TOO_SHORT, pos);
            }
            let code_point = (lead & 0x07) << 18 | at(1) << 12 | at(2) << 6 | at(3);
            if code_point <= 0xffff {
                return error(Status::OVERLONG, pos);
            }
            if code_point > 0x10ffff {
                return error(Status::TOO_LARGE, pos);
            }
            let code_point = code_point - 0x10000;
            out[written].write(0xd800 + (code_point >> 10) as u16);
            out[written + 1].write(0xdc00 + (code_point & 0x3ff) as u16);
            (pos, written) = (pos + 4, written + 2);
        } else if is_continuation(byte) {
            return error(Status::TOO_LONG, pos);
        } else {
            return error(Status::HEADER_BITS, pos);
        }
    }
    ok(written)
}

/// simdutf's `scalar::utf16_to_utf8::convert_with_errors`. `out` has room for what the text has in UTF-8.
pub(crate) fn utf16_to_utf8(data: &[u16], out: &mut [MaybeUninit<u8>]) -> SIMDUTFResult {
    let len = data.len();
    let (mut pos, mut written) = (0, 0);
    while pos < len {
        if kernels::four_ascii16(&data[pos..]) {
            let run = kernels::first_non_ascii16(&data[pos..]);
            kernels::narrow(&data[pos..pos + run], &mut out[written..written + run]);
            (pos, written) = (pos + run, written + run);
            continue;
        }
        let word = data[pos];
        if word & 0xff80 == 0 {
            out[written].write(word as u8);
            (pos, written) = (pos + 1, written + 1);
        } else if word & 0xf800 == 0 {
            out[written].write((word >> 6) as u8 | 0xc0);
            out[written + 1].write((word & 0x3f) as u8 | 0x80);
            (pos, written) = (pos + 1, written + 2);
        } else if !is_surrogate(word) {
            out[written].write((word >> 12) as u8 | 0xe0);
            out[written + 1].write((word >> 6 & 0x3f) as u8 | 0x80);
            out[written + 2].write((word & 0x3f) as u8 | 0x80);
            (pos, written) = (pos + 1, written + 3);
        } else {
            if !is_pair(data, pos) {
                return error(Status::SURROGATE, pos);
            }
            let value =
                (u32::from(word - 0xd800) << 10) + u32::from(data[pos + 1] - 0xdc00) + 0x10000;
            out[written].write((value >> 18) as u8 | 0xf0);
            out[written + 1].write((value >> 12 & 0x3f) as u8 | 0x80);
            out[written + 2].write((value >> 6 & 0x3f) as u8 | 0x80);
            out[written + 3].write((value & 0x3f) as u8 | 0x80);
            (pos, written) = (pos + 2, written + 4);
        }
    }
    ok(written)
}

/// `out` has room for what the text has in UTF-8.
pub(crate) fn latin1_to_utf8(data: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
    let (mut pos, mut written) = (0, 0);
    while pos < data.len() {
        if kernels::sixteen_ascii(&data[pos..]) {
            let run = kernels::first_non_ascii(&data[pos..]);
            out[written..written + run].write_copy_of_slice(&data[pos..pos + run]);
            (pos, written) = (pos + run, written + run);
            continue;
        }
        let byte = data[pos];
        if byte < 0x80 {
            out[written].write(byte);
            written += 1;
        } else {
            out[written].write(byte >> 6 | 0xc0);
            out[written + 1].write(byte & 0x3f | 0x80);
            written += 2;
        }
        pos += 1;
    }
    written
}
