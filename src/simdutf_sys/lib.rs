//! simdutf's validation, transcoding and base64, in Rust, with the results of simdutf 9.
#![feature(core_io, core_io_borrowed_buf)]
#![warn(unused_must_use)]
mod base64;
mod kernels;
mod pem;
pub mod simdutf;
#[cfg(test)]
mod tests;
mod unicode;

// The canonical UTF-8 validation entry points for the codebase — `core::str::from_utf8` is NOT used at runtime
// (PORTING.md §Strings): the check of bun_simd runs on the vector unit.
pub use simdutf::validate::ascii as validate_ascii;
pub use simdutf::validate::utf8 as validate_utf8;
