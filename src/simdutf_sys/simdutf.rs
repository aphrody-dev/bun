use crate::{base64 as codec, unicode};
use core::io::BorrowedCursor;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SIMDUTFResult {
    pub status: Status,
    pub count: usize,
}

impl SIMDUTFResult {
    pub fn is_successful(&self) -> bool {
        self.status == Status::SUCCESS
    }
}

/// simdutf's `error_code`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Status(pub i32);

impl Status {
    pub const SUCCESS: Status = Status(0);
    /// A byte has 5 header bits or more.
    pub const HEADER_BITS: Status = Status(1);
    /// The leading byte must be followed by N-1 continuation bytes, where N is the UTF-8 character length.
    /// This is also the error when the input is truncated.
    pub const TOO_SHORT: Status = Status(2);
    /// Too many continuation bytes, or the text starts with one.
    pub const TOO_LONG: Status = Status(3);
    pub const OVERLONG: Status = Status(4);
    /// Above U+10FFFF, or for ASCII above U+7F.
    pub const TOO_LARGE: Status = Status(5);
    /// U+D800 to U+DFFF in UTF-8, or a surrogate that is not half of a pair in UTF-16.
    pub const SURROGATE: Status = Status(6);
    /// Found a character that cannot be part of a valid base64 string.
    pub const INVALID_BASE64_CHARACTER: Status = Status(7);
    /// The base64 input ends with a single character, padding aside.
    pub const BASE64_INPUT_REMAINDER: Status = Status(8);
    /// The base64 input ends with bits that are not zero.
    pub const BASE64_EXTRA_BITS: Status = Status(9);
    pub const OUTPUT_BUFFER_TOO_SMALL: Status = Status(10);
}

pub mod validate {
    use super::*;

    pub mod with_errors {
        use super::*;

        /// On an error, `count` is where the character that is wrong starts.
        pub fn utf8(input: &[u8]) -> SIMDUTFResult {
            unicode::validate_utf8_with_errors(input)
        }

        /// On an error, `count` is the first byte above 0x7F.
        pub fn ascii(input: &[u8]) -> SIMDUTFResult {
            unicode::validate_ascii_with_errors(input)
        }
    }

    pub fn utf8(input: &[u8]) -> bool {
        unicode::validate_utf8(input)
    }
    pub fn ascii(input: &[u8]) -> bool {
        unicode::validate_ascii(input)
    }
    pub fn utf16le(input: &[u16]) -> bool {
        unicode::validate_utf16(input)
    }
}

/// UTF-16 or Latin-1 text with an upper bound of the UTF-8 bytes that its conversion writes.
#[derive(Copy, Clone)]
pub struct WithUtf8Len<'a, T> {
    slice: &'a [T],
    utf8_len: usize,
}

impl<'a, T> WithUtf8Len<'a, T> {
    pub fn slice(self) -> &'a [T] {
        self.slice
    }

    pub fn utf8_len(self) -> usize {
        self.utf8_len
    }
}

impl<'a> WithUtf8Len<'a, u16> {
    pub fn utf16(slice: &'a [u16]) -> Self {
        let utf8_len = length::utf8::from::utf16::le(slice);
        Self { slice, utf8_len }
    }

    pub fn utf16_with_replacement(slice: &'a [u16]) -> Self {
        let utf8_len = length::utf8::from::utf16::le_with_replacement(slice);
        Self { slice, utf8_len }
    }

    /// `None` if the worst case does not fit in `capacity` bytes.
    pub fn utf16_worst_case(slice: &'a [u16], capacity: usize) -> Option<Self> {
        let utf8_len = slice.len() * 3;
        (utf8_len <= capacity).then_some(Self { slice, utf8_len })
    }
}

impl<'a> WithUtf8Len<'a, u8> {
    pub fn latin1(slice: &'a [u8]) -> Self {
        let utf8_len = length::utf8::from::latin1(slice);
        Self { slice, utf8_len }
    }
}

/// UTF-8 text with an upper bound of the UTF-16 units that its conversion writes.
#[derive(Copy, Clone)]
pub struct WithUtf16Len<'a> {
    slice: &'a [u8],
    utf16_len: usize,
}

impl<'a> WithUtf16Len<'a> {
    pub fn utf8(slice: &'a [u8]) -> Self {
        let utf16_len = length::utf16::from::utf8(slice);
        Self { slice, utf16_len }
    }

    /// `None` if the worst case does not fit in `capacity` units.
    pub fn utf8_worst_case(slice: &'a [u8], capacity: usize) -> Option<Self> {
        let utf16_len = slice.len();
        (utf16_len <= capacity).then_some(Self { slice, utf16_len })
    }

    pub fn utf16_len(self) -> usize {
        self.utf16_len
    }
}

pub mod convert {
    use super::*;

    pub mod utf8 {
        use super::*;
        pub mod to {
            use super::*;
            pub mod utf16 {
                use super::*;
                pub mod with_errors {
                    use super::*;

                    /// Appends nothing to `output` if the result is an error.
                    pub fn le(
                        input: WithUtf16Len<'_>,
                        mut output: BorrowedCursor<'_, u16>,
                    ) -> SIMDUTFResult {
                        assert!(input.utf16_len <= output.capacity());
                        // SAFETY: the conversion only writes values.
                        let room = unsafe { output.as_mut() };
                        let room = &mut room[..input.utf16_len];
                        let result = unicode::utf8_to_utf16(input.slice, room);
                        if result.is_successful() {
                            // SAFETY: on success `count` is the number of units it wrote.
                            unsafe { output.advance(result.count) };
                        }
                        result
                    }
                }
            }
        }
    }

    pub mod utf16 {
        use super::*;
        pub mod to {
            use super::*;
            pub mod utf8 {
                use super::*;
                pub mod with_errors {
                    use super::*;

                    /// Appends nothing to `output` if the result is an error.
                    pub fn le(
                        input: WithUtf8Len<'_, u16>,
                        mut output: BorrowedCursor<'_, u8>,
                    ) -> SIMDUTFResult {
                        assert!(input.utf8_len <= output.capacity());
                        // SAFETY: the conversion only writes values.
                        let room = unsafe { output.as_mut() };
                        let result =
                            unicode::utf16_to_utf8(input.slice, &mut room[..input.utf8_len]);
                        if result.is_successful() {
                            // SAFETY: on success `count` is the number of bytes it wrote.
                            unsafe { output.advance(result.count) };
                        }
                        result
                    }
                }
            }
        }
    }

    pub mod latin1 {
        use super::*;
        pub mod to {
            use super::*;

            pub fn utf8(input: WithUtf8Len<'_, u8>, mut output: BorrowedCursor<'_, u8>) {
                assert!(input.utf8_len <= output.capacity());
                // SAFETY: the conversion only writes values.
                let room = unsafe { output.as_mut() };
                let written = unicode::latin1_to_utf8(input.slice, &mut room[..input.utf8_len]);
                // SAFETY: it wrote `written` bytes.
                unsafe { output.advance(written) };
            }
        }
    }
}

pub mod length {
    use super::*;

    pub mod utf8 {
        use super::*;
        pub mod from {
            use super::*;
            pub mod utf16 {
                use super::*;
                /// Two bytes for each surrogate: exact for valid UTF-16.
                pub(crate) fn le(input: &[u16]) -> usize {
                    unicode::utf8_length_from_utf16(input)
                }
                /// Like [`le`], but charges 3 bytes (U+FFFD) per unpaired
                /// surrogate instead of assuming valid UTF-16. This is the
                /// exact byte count produced by the replacement encoder.
                pub fn le_with_replacement(input: &[u16]) -> usize {
                    unicode::utf8_length_from_utf16_with_replacement(input)
                }
            }

            pub fn latin1(input: &[u8]) -> usize {
                unicode::utf8_length_from_latin1(input)
            }
        }
    }

    pub mod utf16 {
        use super::*;
        pub mod from {
            use super::*;
            /// Exact for valid UTF-8.
            pub fn utf8(input: &[u8]) -> usize {
                unicode::utf16_length_from_utf8(input)
            }
        }
    }
}

pub mod base64 {
    use super::codec::{self, LastChunk, options};
    use super::{BorrowedCursor, SIMDUTFResult};

    fn alphabet(is_urlsafe: bool) -> u64 {
        if is_urlsafe {
            options::URL
        } else {
            options::DEFAULT
        }
    }

    /// Panics if `output` is shorter than [`encode_len`].
    pub fn encode(input: &[u8], mut output: BorrowedCursor<'_, u8>, is_urlsafe: bool) {
        let len = encode_len(input.len(), is_urlsafe);
        assert!(len <= output.capacity());
        // SAFETY: the encoder only writes values.
        let room = unsafe { output.as_mut() };
        let written = codec::from_binary(input, &mut room[..len], alphabet(is_urlsafe));
        // SAFETY: it wrote `written` bytes.
        unsafe { output.advance(written) };
    }

    /// With padding unless `is_urlsafe`.
    pub fn encode_len(input: usize, is_urlsafe: bool) -> usize {
        codec::length_from_binary(input, alphabet(is_urlsafe))
    }

    /// ASCII white space is skipped, and padding is optional. On success `count` is the number of bytes written to
    /// `output`, else where the input is wrong.
    pub fn decode(input: &[u8], output: &mut [u8], is_urlsafe: bool) -> SIMDUTFResult {
        codec::to_binary_safe(input, output, alphabet(is_urlsafe), LastChunk::Loose)
    }

    /// Lenient decode matching Node.js `Buffer` semantics
    /// (`simdutf::base64_default_or_url_accept_garbage` + loose last chunk):
    /// accepts both the standard and URL-safe alphabets, skips whitespace and
    /// any other non-alphabet characters, and stops at the first `'='`.
    /// On success, `count` is the number of bytes written to `output`.
    pub fn decode_lenient(input: &[u8], output: &mut [u8]) -> SIMDUTFResult {
        codec::to_binary_safe(
            input,
            output,
            options::DEFAULT_OR_URL_ACCEPT_GARBAGE,
            LastChunk::Loose,
        )
    }
}
