#![warn(unused_must_use)]
//! `bun_runtime`'s JS-visible APIs on `bun_js`, the engine boundary on Boa: the bodies of the `.bind.ts` schemas that
//! the JavaScriptCore build implements in `src/runtime`, over the same engine-free crates.
//!
//! [`generated_bindgen::native`] is what `$bindgen(file, symbol)` answers.

pub mod generated_bindgen;
pub mod node;
pub mod test_runner;
pub mod web;
