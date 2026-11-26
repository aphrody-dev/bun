//! What `$bindgen(file, symbol)` and `$cpp(file, symbol)` in src/js find on Boa: the functions of the crates on
//! `bun_js`, by the schema and export that name them. `None` is a native Boa has no implementation of yet.

use bun_js::{JSGlobalObject, JSValue, JsResult};

pub fn bindgen(_global: &JSGlobalObject, _file: &str, _symbol: &str) -> Option<JsResult<JSValue>> {
    None
}

pub fn cpp(_global: &JSGlobalObject, _file: &str, _symbol: &str) -> Option<JsResult<JSValue>> {
    None
}
