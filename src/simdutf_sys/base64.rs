//! simdutf 9's `base64_to_binary_safe` and `binary_to_base64`, after its scalar code (`scalar/base64.h` and
//! `base64_implementation.h`), whose results its vector code returns too.

use crate::kernels;
use crate::simdutf::{SIMDUTFResult, Status};
use core::mem::MaybeUninit;

/// `simdutf::base64_options`.
pub(crate) mod options {
    pub(crate) const DEFAULT: u64 = 0;
    pub(crate) const URL: u64 = 1;
    pub(crate) const REVERSE_PADDING: u64 = 2;
    pub(crate) const DEFAULT_ACCEPT_GARBAGE: u64 = 4;
    pub(crate) const URL_ACCEPT_GARBAGE: u64 = 5;
    pub(crate) const DEFAULT_OR_URL: u64 = 8;
    pub(crate) const DEFAULT_OR_URL_ACCEPT_GARBAGE: u64 = 12;
}

/// `simdutf::last_chunk_handling_options`. Bun decodes with `Loose`; the others are its tests'.
#[derive(Copy, Clone, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum LastChunk {
    Loose,
    Strict,
    StopBeforePartial,
    OnlyFullChunks,
}

impl LastChunk {
    fn is_partial(self) -> bool {
        matches!(self, Self::StopBeforePartial | Self::OnlyFullChunks)
    }
}

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// 0 to 63 for the alphabet, 64 for ASCII white space (not VT), 255 for the rest.
const fn values(alphabets: &[&[u8; 64]]) -> [u8; 256] {
    let mut table = [255; 256];
    let spaces = *b" \t\n\r\x0c";
    let mut i = 0;
    while i < spaces.len() {
        table[spaces[i] as usize] = 64;
        i += 1;
    }
    let mut a = 0;
    while a < alphabets.len() {
        let mut i = 0;
        while i < 64 {
            table[alphabets[a][i] as usize] = i as u8;
            i += 1;
        }
        a += 1;
    }
    table
}

static TO_DEFAULT: [u8; 256] = values(&[STANDARD]);
static TO_URL: [u8; 256] = values(&[URL_SAFE]);
static TO_DEFAULT_OR_URL: [u8; 256] = values(&[STANDARD, URL_SAFE]);

fn table(options: u64) -> &'static [u8; 256] {
    if options & options::DEFAULT_OR_URL != 0 {
        &TO_DEFAULT_OR_URL
    } else if options & options::URL != 0 {
        &TO_URL
    } else {
        &TO_DEFAULT
    }
}

fn ignores_garbage(options: u64) -> bool {
    matches!(
        options,
        options::URL_ACCEPT_GARBAGE
            | options::DEFAULT_ACCEPT_GARBAGE
            | options::DEFAULT_OR_URL_ACCEPT_GARBAGE
    )
}

/// `base64_ignorable`.
fn is_ignorable(c: u8, options: u64) -> bool {
    match table(options)[usize::from(c)] {
        0..=63 => false,
        64 => true,
        _ => ignores_garbage(options),
    }
}

#[derive(Copy, Clone)]
struct FullResult {
    error: Status,
    input_count: usize,
    output_count: usize,
    padding_error: bool,
}

impl FullResult {
    const fn new(error: Status, input_count: usize, output_count: usize) -> Self {
        Self {
            error,
            input_count,
            output_count,
            padding_error: false,
        }
    }
    const fn padding(error: Status, input_count: usize, output_count: usize) -> Self {
        Self {
            error,
            input_count,
            output_count,
            padding_error: true,
        }
    }
}

struct ReducedInput {
    equalsigns: usize,
    equallocation: usize,
    srclen: usize,
    full_input_length: usize,
}

fn find_end(src: &[u8], options: u64) -> ReducedInput {
    let to_base64 = table(options);
    if ignores_garbage(options) {
        let at = kernels::first_of(src, b'=');
        let (equalsigns, full_input_length) = if at == src.len() {
            (0, at)
        } else {
            (1, at + 1)
        };
        return ReducedInput {
            equalsigns,
            equallocation: at,
            srclen: at,
            full_input_length,
        };
    }
    // Trailing white space is in the full length (simdutf/simdutf#824).
    let full_input_length = src.len();
    let mut srclen = src.len();
    let mut equalsigns = 0;
    let is_space = |c: u8| to_base64[usize::from(c)] == 64;
    while srclen > 0 && is_space(src[srclen - 1]) {
        srclen -= 1;
    }
    let mut equallocation = srclen;
    if srclen > 0 && src[srclen - 1] == b'=' {
        equallocation = srclen - 1;
        srclen -= 1;
        equalsigns = 1;
        while srclen > 0 && is_space(src[srclen - 1]) {
            srclen -= 1;
        }
        if srclen > 0 && src[srclen - 1] == b'=' {
            equallocation = srclen - 1;
            srclen -= 1;
            equalsigns = 2;
        }
    }
    ReducedInput {
        equalsigns,
        equallocation,
        srclen,
        full_input_length,
    }
}

/// `base64_tail_decode_impl`: `src` is without its padding. With `CHECK_CAPACITY`, it fails instead of writing past
/// the end of `dst`; without, `dst` has room.
fn tail_decode<const CHECK_CAPACITY: bool>(
    dst: &mut [u8],
    src: &[u8],
    padding_characters: usize,
    options: u64,
    last_chunk: LastChunk,
) -> FullResult {
    let to_base64 = table(options);
    let ignore_garbage = ignores_garbage(options);
    let (mut s, mut d) = (0, 0);
    let mut buffer = [0u8; 4];
    loop {
        while let Some(&[c0, c1, c2, c3]) = src[s..].first_chunk::<4>() {
            let codes = [c0, c1, c2, c3].map(|c| to_base64[usize::from(c)]);
            if codes.iter().any(|&code| code > 63) {
                break;
            }
            if CHECK_CAPACITY && dst.len() - d < 3 {
                return FullResult::new(Status::OUTPUT_BUFFER_TOO_SMALL, s, d);
            }
            let triple = u32::from(codes[0]) << 18
                | u32::from(codes[1]) << 12
                | u32::from(codes[2]) << 6
                | u32::from(codes[3]);
            dst[d..d + 3].copy_from_slice(&triple.to_be_bytes()[1..]);
            (s, d) = (s + 4, d + 3);
        }
        let srccur = s;
        let mut idx = 0;
        while idx < 4 && s < src.len() {
            let code = to_base64[usize::from(src[s])];
            buffer[idx] = code;
            if code <= 63 {
                idx += 1;
            } else if !ignore_garbage && code > 64 {
                return FullResult::new(Status::INVALID_BASE64_CHARACTER, s, d);
            }
            s += 1;
        }
        if idx != 4 {
            if !ignore_garbage && idx + padding_characters > 4 {
                return FullResult::padding(Status::INVALID_BASE64_CHARACTER, s, d);
            }
            if !ignore_garbage
                && last_chunk == LastChunk::Loose
                && idx >= 2
                && padding_characters > 0
                && (idx + padding_characters) & 3 != 0
            {
                return FullResult::padding(Status::INVALID_BASE64_CHARACTER, s, d);
            }
            if !ignore_garbage
                && last_chunk == LastChunk::Strict
                && idx >= 2
                && (idx + padding_characters) & 3 != 0
            {
                return FullResult::padding(Status::BASE64_INPUT_REMAINDER, s, d);
            }
            if (last_chunk == LastChunk::StopBeforePartial
                && padding_characters + idx < 4
                && idx != 0
                && (idx >= 2 || padding_characters == 0))
                || (last_chunk == LastChunk::OnlyFullChunks
                    && (idx >= 2 || padding_characters == 0))
            {
                return FullResult::new(Status::SUCCESS, srccur, d);
            }
            if idx == 2 {
                let triple = u32::from(buffer[0]) << 18 | u32::from(buffer[1]) << 12;
                if !ignore_garbage && last_chunk == LastChunk::Strict && triple & 0xffff != 0 {
                    return FullResult::new(Status::BASE64_EXTRA_BITS, s, d);
                }
                if CHECK_CAPACITY && dst.len() - d < 1 {
                    return FullResult::new(Status::OUTPUT_BUFFER_TOO_SMALL, srccur, d);
                }
                dst[d] = (triple >> 16) as u8;
                d += 1;
            } else if idx == 3 {
                let triple = u32::from(buffer[0]) << 18
                    | u32::from(buffer[1]) << 12
                    | u32::from(buffer[2]) << 6;
                if !ignore_garbage && last_chunk == LastChunk::Strict && triple & 0xff != 0 {
                    return FullResult::new(Status::BASE64_EXTRA_BITS, s, d);
                }
                if CHECK_CAPACITY && dst.len() - d < 2 {
                    return FullResult::new(Status::OUTPUT_BUFFER_TOO_SMALL, srccur, d);
                }
                dst[d..d + 2].copy_from_slice(&triple.to_be_bytes()[1..3]);
                d += 2;
            } else if !ignore_garbage
                && idx == 1
                && (!last_chunk.is_partial() || padding_characters > 0)
            {
                return FullResult::new(Status::BASE64_INPUT_REMAINDER, s, d);
            } else if !ignore_garbage && idx == 0 && padding_characters > 0 {
                return FullResult::padding(Status::INVALID_BASE64_CHARACTER, s, d);
            }
            return FullResult::new(Status::SUCCESS, s, d);
        }
        if CHECK_CAPACITY && dst.len() - d < 3 {
            return FullResult::new(Status::OUTPUT_BUFFER_TOO_SMALL, srccur, d);
        }
        let triple = u32::from(buffer[0]) << 18
            | u32::from(buffer[1]) << 12
            | u32::from(buffer[2]) << 6
            | u32::from(buffer[3]);
        dst[d..d + 3].copy_from_slice(&triple.to_be_bytes()[1..]);
        d += 3;
    }
}

fn patch_tail_result(
    mut r: FullResult,
    equallocation: usize,
    full_input_length: usize,
    last_chunk: LastChunk,
) -> FullResult {
    if r.padding_error {
        r.input_count = equallocation;
    }
    if r.error == Status::SUCCESS && (!last_chunk.is_partial() || r.output_count % 3 != 0) {
        r.input_count = full_input_length;
    }
    r
}

/// `base64_to_binary_details_impl` (`CHECK_CAPACITY = false`) and `base64_to_binary_details_safe_impl`.
fn details<const CHECK_CAPACITY: bool>(
    input: &[u8],
    output: &mut [u8],
    options: u64,
    last_chunk: LastChunk,
) -> FullResult {
    let ignore_garbage = ignores_garbage(options);
    let ri = find_end(input, options);
    if ri.srclen == 0 {
        if !ignore_garbage && ri.equalsigns > 0 {
            return FullResult::new(Status::INVALID_BASE64_CHARACTER, ri.equallocation, 0);
        }
        return FullResult::new(Status::SUCCESS, ri.full_input_length, 0);
    }
    let r = tail_decode::<CHECK_CAPACITY>(
        output,
        &input[..ri.srclen],
        ri.equalsigns,
        options,
        last_chunk,
    );
    let mut r = patch_tail_result(r, ri.equallocation, ri.full_input_length, last_chunk);
    if !last_chunk.is_partial()
        && r.error == Status::SUCCESS
        && ri.equalsigns > 0
        && !ignore_garbage
    {
        if r.output_count % 3 == 0 || (r.output_count % 3) + 1 + ri.equalsigns != 4 {
            return FullResult::new(
                Status::INVALID_BASE64_CHARACTER,
                ri.equallocation,
                r.output_count,
            );
        }
    }
    if last_chunk.is_partial() && r.error == Status::SUCCESS && r.input_count < ri.full_input_length
    {
        while r.input_count < ri.full_input_length && is_ignorable(input[r.input_count], options) {
            r.input_count += 1;
        }
        if r.input_count < ri.full_input_length {
            while r.input_count > 0 && is_ignorable(input[r.input_count - 1], options) {
                r.input_count -= 1;
            }
        }
    }
    r
}

/// `base64_length_from_binary`.
pub(crate) fn length_from_binary(length: usize, options: u64) -> usize {
    let use_padding = (options & options::URL == 0)
        ^ (options & options::REVERSE_PADDING == options::REVERSE_PADDING);
    if use_padding {
        length.div_ceil(3) * 4
    } else {
        length / 3 * 4 + if length % 3 != 0 { length % 3 + 1 } else { 0 }
    }
}

/// `base64_length_from_binary_with_lines`.
pub(crate) fn length_from_binary_with_lines(
    length: usize,
    options: u64,
    line_length: usize,
) -> usize {
    if length == 0 {
        return 0;
    }
    let base64_length = length_from_binary(length, options);
    base64_length + base64_length.div_ceil(line_length.max(4)) - 1
}

/// `base64_to_binary_safe` without `decode_up_to_bad_char`. On success `count` is what was written to `output`,
/// else where the input is wrong.
pub(crate) fn to_binary_safe(
    input: &[u8],
    output: &mut [u8],
    options: u64,
    last_chunk: LastChunk,
) -> SIMDUTFResult {
    let safe_input = input
        .len()
        .min(length_from_binary(output.len() / 3 * 3, options));
    let done_with_partial = safe_input == input.len();
    let first_chunk = if done_with_partial {
        last_chunk
    } else {
        LastChunk::OnlyFullChunks
    };
    let r = details::<false>(&input[..safe_input], output, options, first_chunk);
    if r.error != Status::SUCCESS {
        return SIMDUTFResult {
            status: r.error,
            count: r.input_count,
        };
    }
    let (mut input_position, output_position) = (r.input_count, r.output_count);
    if done_with_partial {
        return SIMDUTFResult {
            status: Status::SUCCESS,
            count: output_position,
        };
    }
    let r = details::<true>(
        &input[input_position..],
        &mut output[output_position..],
        options,
        last_chunk,
    );
    input_position += r.input_count;
    let output_position = output_position + r.output_count;
    if r.error != Status::SUCCESS {
        return SIMDUTFResult {
            status: r.error,
            count: input_position,
        };
    }
    SIMDUTFResult {
        status: Status::SUCCESS,
        count: output_position,
    }
}

/// `binary_to_base64`. `output` has room for the encoded length.
pub(crate) fn from_binary(input: &[u8], output: &mut [MaybeUninit<u8>], options: u64) -> usize {
    let use_padding = (options & options::URL == 0)
        ^ (options & options::REVERSE_PADDING == options::REVERSE_PADDING);
    let alphabet = if options & options::URL != 0 {
        URL_SAFE
    } else {
        STANDARD
    };
    let length = length_from_binary(input.len(), options);
    let (chunks, tail) = input.as_chunks::<3>();
    let (encoded, rest) = output[..length].split_at_mut(chunks.len() * 4);
    for (from, to) in chunks.iter().zip(encoded.as_chunks_mut::<4>().0) {
        let bits = u32::from_be_bytes([0, from[0], from[1], from[2]]);
        *to = [18, 12, 6, 0].map(|shift| MaybeUninit::new(alphabet[(bits >> shift & 63) as usize]));
    }
    match *tail {
        [a] => {
            rest[0].write(alphabet[usize::from(a >> 2)]);
            rest[1].write(alphabet[usize::from((a & 3) << 4)]);
            if use_padding {
                rest[2..4].fill(MaybeUninit::new(b'='));
            }
        }
        [a, b] => {
            rest[0].write(alphabet[usize::from(a >> 2)]);
            rest[1].write(alphabet[usize::from((a & 3) << 4 | b >> 4)]);
            rest[2].write(alphabet[usize::from((b & 15) << 2)]);
            if use_padding {
                rest[3].write(b'=');
            }
        }
        _ => {}
    }
    length
}

/// `binary_to_base64_with_lines`: a newline after every `line_length` characters but the last.
pub(crate) fn from_binary_with_lines(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    options: u64,
    line_length: usize,
) -> usize {
    let line_length = line_length.max(4);
    let total = length_from_binary_with_lines(input.len(), options, line_length);
    // Encoded at the end, each line moves forward by the newlines before it.
    let newlines = total - length_from_binary(input.len(), options);
    from_binary(input, &mut output[newlines..total], options);
    let mut at = 0;
    for (line, start) in (newlines..total).step_by(line_length).enumerate() {
        if line > 0 {
            output[at].write(b'\n');
            at += 1;
        }
        let end = total.min(start + line_length);
        output.copy_within(start..end, at);
        at += end - start;
    }
    total
}
