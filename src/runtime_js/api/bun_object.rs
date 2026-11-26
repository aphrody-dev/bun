//! The functions of `Bun` that src/jsc/bindings/BunObject.cpp defines in C++, on `bun_js`.

use bun_js::{CallFrame, DeepEqualsMode, JSFunction, JSGlobalObject, JSValue, JsResult};

/// `Bun.deepEquals(a, b, strict?, skipPrototype?)`, as C++'s `functionBunDeepEquals`.
pub fn deep_equals(global: &JSGlobalObject, frame: &CallFrame<'_>) -> JsResult<JSValue> {
    if frame.arguments_count() < 2 {
        return Err(global.throw_type_error(format_args!("Expected 2 values to compare")));
    }
    let is_true = |value: JSValue| value.is_boolean() && value.as_boolean();
    let strict = is_true(frame.argument(2));
    let mode = DeepEqualsMode {
        strict,
        skip_prototype_identity: strict && is_true(frame.argument(3)),
    };
    let equal = frame
        .argument(0)
        .deep_equals_with(&frame.argument(1), global, mode)?;
    Ok(JSValue::js_boolean(equal))
}

/// Each function by the name `Bun` has it under.
pub fn functions(global: &JSGlobalObject) -> Vec<(&'static str, JSValue)> {
    vec![(
        "deepEquals",
        JSFunction::create(global, "deepEquals", deep_equals, 2),
    )]
}
