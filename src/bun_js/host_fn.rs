//! Host functions: Rust that JavaScript calls.

use boa_engine::object::FunctionObjectBuilder;
use boa_engine::{Context, JsString, JsValue, NativeFunction};

use crate::{CallFrame, JSGlobalObject, JSValue, JsResult};

/// A function JavaScript calls, as `bun_jsc::JSHostFn`: an exception it throws is pending on `global` when it
/// returns `Err`.
pub type JSHostFn = fn(&JSGlobalObject, &CallFrame<'_>) -> JsResult<JSValue>;

/// Calls `f` as Boa's native function `(this, args, context)`.
pub(crate) fn call_host_fn(
    f: impl FnOnce(&JSGlobalObject, &CallFrame<'_>) -> JsResult<JSValue>,
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> boa_engine::JsResult<JsValue> {
    let global = JSGlobalObject::from_context(context);
    let frame = CallFrame::new(this, args);
    match f(global, &frame) {
        Ok(value) => {
            debug_assert!(
                !global.has_exception(),
                "a host function returned a value with an exception pending"
            );
            Ok(value.0)
        }
        Err(error) => Err(global.into_engine_error(error)),
    }
}

/// Boa's native function for a host function.
pub fn native_function(f: JSHostFn) -> NativeFunction {
    NativeFunction::from_copy_closure(move |this, args, context| {
        call_host_fn(f, this, args, context)
    })
}

/// A function object.
pub struct JSFunction;

impl JSFunction {
    /// A function named `name` whose `length` is `length`, which calls `f`.
    pub fn create(global: &JSGlobalObject, name: &str, f: JSHostFn, length: usize) -> JSValue {
        let context = global.cx();
        let function = FunctionObjectBuilder::new(context.realm(), native_function(f))
            .name(JsString::from(name))
            .length(length)
            .build();
        JSValue(function.into())
    }
}
