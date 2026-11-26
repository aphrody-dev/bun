//! What simdutf 9.0.0 returned for the inputs of `tests/fixtures.txt` (`bun scripts/simdutf-fixtures.ts`).
#![allow(
    clippy::disallowed_methods,
    reason = "bun_core::strings is above this crate"
)]

use crate::base64::{self, LastChunk, options};
use crate::simdutf::{SIMDUTFResult, Status};
use crate::{pem, unicode};
use core::mem::MaybeUninit;

static FIXTURES: &str = include_str!("tests/fixtures.txt");

fn unhex(text: &str) -> Vec<u8> {
    if text == "-" {
        return Vec::new();
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// FNV-1a, as the driver prints it.
fn hash(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

fn written<T: Copy>(out: &[MaybeUninit<T>], n: usize) -> Vec<T> {
    // SAFETY: the conversion initialized the first `n` elements.
    out[..n]
        .iter()
        .map(|it| unsafe { it.assume_init() })
        .collect()
}

fn units_bytes(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}

/// `status count hash`, with `-` for the hash of an error, as the driver prints a result.
fn result(r: SIMDUTFResult, output: &[u8]) -> String {
    let h = if r.is_successful() {
        hash(output)
    } else {
        "-".into()
    };
    format!("{}\t{}\t{h}", r.status.0, r.count)
}

fn run(op: &str, input: &[u8], arg: &str) -> String {
    let mut fields: Vec<String> = Vec::new();
    match op {
        "utf8" => {
            let r = unicode::validate_utf8_with_errors(input);
            fields.push(format!(
                "{}\t{}\t{}",
                u8::from(unicode::validate_utf8(input)),
                r.status.0,
                r.count
            ));
            let len16 = unicode::utf16_length_from_utf8(input);
            let mut out = vec![MaybeUninit::uninit(); len16 + 1];
            let c = unicode::utf8_to_utf16(input, &mut out[..len16]);
            let output = if c.is_successful() {
                units_bytes(&written(&out, c.count))
            } else {
                Vec::new()
            };
            fields.push(format!("{len16}\t{}", result(c, &output)));
            let a = unicode::validate_ascii_with_errors(input);
            fields.push(format!(
                "{}\t{}\t{}",
                u8::from(unicode::validate_ascii(input)),
                a.status.0,
                a.count
            ));
            let len8 = unicode::utf8_length_from_latin1(input);
            let mut out = vec![MaybeUninit::uninit(); len8];
            let w = unicode::latin1_to_utf8(input, &mut out);
            fields.push(format!("{len8}\t{}", hash(&written(&out, w))));
        }
        "utf16" => {
            let units: Vec<u16> = input
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&p| u16::from_le_bytes(p))
                .collect();
            let len8 = unicode::utf8_length_from_utf16(&units);
            let len8r = unicode::utf8_length_from_utf16_with_replacement(&units);
            let mut out = vec![MaybeUninit::uninit(); len8];
            let c = unicode::utf16_to_utf8(&units, &mut out);
            let output = if c.is_successful() {
                written(&out, c.count)
            } else {
                Vec::new()
            };
            fields.push(format!(
                "{}\t{len8}\t{len8r}\t{}",
                u8::from(unicode::validate_utf16(&units)),
                result(c, &output)
            ));
        }
        "b64enc" => {
            let options = if arg == "1" {
                options::URL
            } else {
                options::DEFAULT
            };
            let len = base64::length_from_binary(input.len(), options);
            let mut out = vec![MaybeUninit::uninit(); len];
            let w = base64::from_binary(input, &mut out, options);
            fields.push(format!("{len}\t{}", hash(&written(&out, w))));
        }
        "b64dec" => {
            for room in arg.split(',') {
                let room: usize = room.parse().unwrap();
                for options in [
                    options::DEFAULT,
                    options::URL,
                    options::DEFAULT_OR_URL_ACCEPT_GARBAGE,
                ] {
                    let mut out = vec![0; room];
                    let r = base64::to_binary_safe(input, &mut out, options, LastChunk::Loose);
                    let n = if r.is_successful() { r.count } else { 0 };
                    fields.push(result(r, &out[..n]));
                }
            }
        }
        "pemdec" => {
            let max_out: usize = arg.parse().unwrap();
            let mut out = vec![0xaa; max_out + 1];
            let mut out_len = 0;
            // SAFETY: both are slices of those lengths.
            let ok = unsafe {
                pem::OPENSSL_pem_public_base64_decode(
                    out.as_mut_ptr(),
                    &raw mut out_len,
                    max_out,
                    input.as_ptr(),
                    input.len(),
                )
            };
            fields.push(format!(
                "{ok}\t{}",
                if ok == 1 {
                    hash(&out[..out_len])
                } else {
                    "-".into()
                }
            ));
        }
        "pemenc" => {
            let max_out: usize = arg.parse().unwrap();
            let mut out = vec![0; max_out + 1];
            // SAFETY: both are slices of those lengths.
            let w = unsafe {
                pem::OPENSSL_pem_public_base64_encode(
                    out.as_mut_ptr(),
                    max_out,
                    input.as_ptr(),
                    input.len(),
                )
            };
            fields.push(hash(&out[..w]));
        }
        _ => panic!("unknown operation {op}"),
    }
    fields.join("\t")
}

#[test]
fn matches_simdutf_9() {
    let (mut cases, mut failures) = (0, Vec::new());
    for line in FIXTURES.lines().filter(|line| !line.starts_with('#')) {
        let mut parts = line.splitn(4, '\t');
        let (op, input, arg) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let expected = parts.next().unwrap_or("");
        let actual = run(op, &unhex(input), arg);
        if actual != expected {
            failures.push(format!(
                "{op} {input} {arg}\n  simdutf: {expected}\n  rust:    {actual}"
            ));
        }
        cases += 1;
    }
    assert!(cases > 3000, "{cases} cases");
    assert!(
        failures.is_empty(),
        "{} of {cases} differ:\n{}",
        failures.len(),
        failures[..failures.len().min(20)].join("\n")
    );
}

#[test]
fn strict_and_partial_chunks() {
    let decode = |text: &[u8], last| {
        let mut out = [0; 16];
        let r = base64::to_binary_safe(text, &mut out, options::DEFAULT, last);
        (r.status, r.count)
    };
    assert_eq!(
        decode(b"QQ", LastChunk::Strict),
        (Status::BASE64_INPUT_REMAINDER, 2)
    );
    assert_eq!(
        decode(b"QR==", LastChunk::Strict),
        (Status::BASE64_EXTRA_BITS, 2)
    );
    assert_eq!(decode(b"QQ==", LastChunk::Strict), (Status::SUCCESS, 1));
    assert_eq!(
        decode(b"QUJDQQ", LastChunk::StopBeforePartial),
        (Status::SUCCESS, 3)
    );
    assert_eq!(
        decode(b"QUJDQQ==", LastChunk::OnlyFullChunks),
        (Status::SUCCESS, 3)
    );
}

#[test]
fn utf8_errors_at_each_offset() {
    // Across the widths of all vectors: where the check of bun_simd ends and the scalar code goes on.
    for at in 0..200 {
        for (bad, status) in [
            (&b"\x80"[..], Status::TOO_LONG),
            (b"\xc2", Status::TOO_SHORT),
            (b"\xc0\x80", Status::OVERLONG),
            (b"\xed\xa0\x80", Status::SURROGATE),
            (b"\xf4\x90\x80\x80", Status::TOO_LARGE),
            (b"\xf8", Status::HEADER_BITS),
        ] {
            let mut text = "é".repeat(at / 2).into_bytes();
            text.resize(at, b'a');
            let start = text.len();
            text.extend_from_slice(bad);
            text.extend_from_slice(b"tail");
            let r = unicode::validate_utf8_with_errors(&text);
            assert_eq!((r.status, r.count), (status, start), "{at} {bad:?}");
            let mut out = vec![MaybeUninit::uninit(); text.len()];
            let c = unicode::utf8_to_utf16(&text, &mut out);
            assert_eq!((c.status, c.count), (status, start), "{at} {bad:?}");
        }
    }
}
