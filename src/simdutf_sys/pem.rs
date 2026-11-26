//! BoringSSL is built with BORINGSSL_PEM_FAST_PUBLIC_BASE64 (scripts/build/deps/boringssl.ts), which routes the base64
//! of PEM CERTIFICATE / CRL / PUBLIC KEY blocks here instead of through its constant-time codec. Private-key PEM never
//! reaches these. Contract: <openssl/pem.h>.

use crate::base64::{self, LastChunk, options};
use core::ffi::c_int;
use core::mem::MaybeUninit;

/// # Safety
/// `input` holds `in_len` bytes, `out` has room for `max_out` and `out_len` is writable.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn OPENSSL_pem_public_base64_decode(
    out: *mut u8,
    out_len: *mut usize,
    max_out: usize,
    input: *const u8,
    in_len: usize,
) -> c_int {
    // SAFETY: the caller passes two slices, as pointers and lengths.
    let (input, output) = unsafe { (slice(input, in_len), slice_mut(out, max_out)) };
    let result = base64::to_binary_safe(input, output, options::DEFAULT, LastChunk::Loose);
    if !result.is_successful() {
        return 0;
    }
    let written = result.count;
    // The decoder tolerates missing '=' padding; PEM (like EVP_DecodeUpdate, OpenSSL and Go) does not.
    let padding = input
        .iter()
        .rev()
        .take_while(|&&c| c == b'=' || matches!(c, b'\n' | b'\r' | b' ' | b'\t' | 0x0c | 0x0b))
        .filter(|&&c| c == b'=')
        .count();
    if padding != (3 - written % 3) % 3 {
        return 0;
    }
    // SAFETY: the caller passes a writable `out_len`.
    unsafe { out_len.write(written) };
    1
}

/// # Safety
/// `input` holds `in_len` bytes and `out` has room for `max_out`.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn OPENSSL_pem_public_base64_encode(
    out: *mut u8,
    max_out: usize,
    input: *const u8,
    in_len: usize,
) -> usize {
    const LINE_LENGTH: usize = 64;
    let needed = base64::length_from_binary_with_lines(in_len, options::DEFAULT, LINE_LENGTH) + 1;
    if in_len == 0 || needed > max_out {
        return 0;
    }
    // SAFETY: the caller passes two slices, as pointers and lengths.
    let (input, output) = unsafe { (slice(input, in_len), uninit(out, max_out)) };
    let written = base64::from_binary_with_lines(input, output, options::DEFAULT, LINE_LENGTH);
    // EVP_EncodeFinal terminates the last line too.
    output[written].write(b'\n');
    written + 1
}

/// # Safety
/// `len` bytes at `ptr`, or a length of 0.
unsafe fn slice<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        return &[];
    }
    // SAFETY: the caller's contract.
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

/// Zeroed first: BoringSSL hands over memory that it has not initialized.
///
/// # Safety
/// `len` writable bytes at `ptr`, or a length of 0.
unsafe fn slice_mut<'a>(ptr: *mut u8, len: usize) -> &'a mut [u8] {
    if len == 0 {
        return &mut [];
    }
    // SAFETY: the caller's contract.
    unsafe {
        ptr.write_bytes(0, len);
        core::slice::from_raw_parts_mut(ptr, len)
    }
}

/// # Safety
/// `len` writable bytes at `ptr`, and a length above 0.
unsafe fn uninit<'a>(ptr: *mut u8, len: usize) -> &'a mut [MaybeUninit<u8>] {
    // SAFETY: the caller's contract.
    unsafe { core::slice::from_raw_parts_mut(ptr.cast(), len) }
}
