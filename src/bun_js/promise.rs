//! `JSPromise`: the promises Rust makes already settled.

use boa_engine::object::builtins::JsPromise;

use crate::{JSGlobalObject, JSValue};

pub struct JSPromise;

impl JSPromise {
    /// `Promise.resolve(value)`: a thenable is followed.
    pub fn resolved_promise_value(global: &JSGlobalObject, value: JSValue) -> JSValue {
        match JsPromise::resolve(value.0, global.cx()) {
            Ok(promise) => JSValue(promise.into()),
            // A thenable whose `then` getter throws: the promise is rejected with that, as JavaScriptCore's is.
            Err(error) => {
                let reason = error.into_opaque(global.cx()).unwrap_or_default();
                Self::rejected_promise_value(global, JSValue(reason))
            }
        }
    }

    /// A promise rejected with `reason`.
    pub fn rejected_promise_value(global: &JSGlobalObject, reason: JSValue) -> JSValue {
        let (promise, resolvers) = JsPromise::new_pending(global.cx());
        let _ = resolvers
            .reject
            .call(&boa_engine::JsValue::undefined(), &[reason.0], global.cx());
        JSValue(promise.into())
    }
}
