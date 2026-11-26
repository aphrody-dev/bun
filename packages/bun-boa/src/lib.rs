//! Bun on Boa: Bun's own transpiler (`bun_js_parser` + `bun_js_printer`) in front of a pure-Rust JavaScript
//! engine, with no JavaScriptCore and no C++.

// Weak symbols in `yarr`.
#![feature(linkage)]

#[cfg(feature = "build")]
pub mod build;
pub mod builtins;
pub mod cli;
pub mod fmt;
pub mod lint;
pub mod loader;
pub mod natives;
pub mod runtime;
pub mod scripts;
pub mod test;
pub mod transpile;
pub mod wtf;
pub mod yarr;
