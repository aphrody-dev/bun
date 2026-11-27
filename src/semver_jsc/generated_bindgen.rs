//! What `src/codegen/bindgen.ts` generates from `src/semver_js`'s `.bind.ts` files.

include!(concat!(
    env!("BUN_CODEGEN_DIR"),
    "/generated_bindgen_bun_semver_js.rs"
));
