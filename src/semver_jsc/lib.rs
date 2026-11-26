#![allow(non_snake_case)]
#![warn(unused_must_use)]
//! `Bun.semver` on JavaScriptCore, until `bun_runtime` moves to `bun_js`: the code and the schemas are
//! `src/semver_js`'s, which JavaScriptCore's bindgen assigns to `bun_semver_js`.

use bun_jsc as js;

#[path = "../semver_js/SemverObject.rs"]
pub mod SemverObject;
pub(crate) mod generated_bindgen;
