//! Bun on Boa: Bun's own transpiler (`bun_js_parser` + `bun_js_printer`) in front of a pure-Rust JavaScript
//! engine, with no JavaScriptCore and no C++.

// Weak symbols in `yarr`.
#![feature(linkage)]

#[cfg(feature = "build")]
pub mod build;
#[cfg(feature = "build")]
mod bundler_shims;
pub mod cli;
pub mod fmt;
pub mod lint;
pub mod loader;
pub mod runtime;
pub mod scripts;
pub mod transpile;
pub mod wtf;
pub mod yarr;
