#![warn(unused_must_use)]
//! `bun_runtime`'s JS-visible APIs on `bun_js`, the engine boundary on Boa: the bodies of the `.bind.ts` schemas that
//! the JavaScriptCore build implements in `src/runtime`, over the same engine-free crates.
//!
//! [`generated_bindgen::native`] is what `$bindgen(file, symbol)` answers, [`cpp`] what `$cpp(file, symbol)` and
//! `$newCppFunction(file, symbol, length)` do, [`bun_object`] what else of `Bun` this crate implements.

pub mod api;
mod engine;
pub mod generated_bindgen;
pub mod node;
pub mod sys_js;
pub mod test_runner;
pub mod web;
pub mod webcore;

use bun_js::{JSGlobalObject, JSValue, JsResult};

/// The natives that the JavaScriptCore build has in C++ (`src/jsc/bindings/<file>`), ported: `None` for one that
/// is not.
pub fn cpp(global: &bun_js::JSGlobalObject, file: &str, symbol: &str) -> Option<bun_js::JsResult<bun_js::JSValue>> {
    // `$newCppFunction` names the wrapper that src/codegen/generate-js2native.ts writes around the C++ function.
    let symbol = symbol.strip_prefix("js2native_wrap_").unwrap_or(symbol);
    match file {
        "NodeAsyncHooks.cpp" => node::async_hooks::native(global, symbol),
        "NodeVM.cpp" => node::vm::native(global, symbol),
        _ => None,
    }
}

/// The properties of `Bun` this crate implements besides Blob's, by name: `Bun.Glob`, `Bun.deepEquals`.
pub fn bun_object(global: &JSGlobalObject) -> JsResult<Vec<(&'static str, JSValue)>> {
    let mut properties = generated_bindgen::glob::exports(global)?;
    properties.extend(api::bun_object::functions(global));
    Ok(properties)
}
