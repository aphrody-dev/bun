//! Bun's builtin modules (src/js: `node:*`, `bun:*`, `internal/*`) on Boa. build.rs embeds them as functions of
//! their intrinsics; `js/intrinsics.ts` makes the intrinsics, loads a module the first time something requires it,
//! and keeps its exports in the registry, as InternalModuleRegistry.cpp does for JavaScriptCore.

use boa_engine::builtins::promise::PromiseState;
use boa_engine::builtins::proxy::Proxy;
use boa_engine::object::ObjectInitializer;
use boa_engine::object::builtins::{
    JsArray, JsArrayBuffer, JsDataView, JsMap, JsPromise, JsRegExp, JsSet, JsTypedArray, JsWeakMap,
    JsWeakSet,
};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, JsArgs, JsNativeError, JsObject, JsResult, JsString, JsValue, Module, NativeFunction,
    Source, js_string,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NativeKind {
    /// `$bindgen(file, symbol)`: a function of a `.bind.ts` schema.
    Bind,
    /// `$cpp(file, symbol)`: what C++ makes for JavaScriptCore.
    Cpp,
}

pub struct Native {
    pub kind: NativeKind,
    pub file: &'static str,
    pub symbol: &'static str,
}

pub struct Builtin {
    /// Its file in src/js, which numbers it: `require` of it is its index in [`MODULES`].
    pub id: &'static str,
    /// What `import` names it by: `node:path`, `bun:sqlite`, `internal:validators`.
    pub specifier: &'static str,
    /// `(function (<intrinsics>) { return (function (){ … }) })`; `None` for a native module, which
    /// js/intrinsics.ts makes.
    pub source: Option<&'static str>,
    /// The parameters of `source`, without their `__intrinsic__` prefix.
    pub intrinsics: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/builtins.rs"));

/// The id of what `import "<specifier>"` names, if it is a builtin module that user code can import.
pub fn id_of(specifier: &str) -> Option<usize> {
    let public = |s: &str| {
        MODULES
            .iter()
            .position(|m| m.specifier == s && !m.specifier.starts_with("internal:"))
    };
    public(specifier).or_else(|| {
        // `path` is `node:path`, except where a package of that name is installed (the loader looks first).
        (!specifier.contains(':'))
            .then(|| public(&format!("node:{specifier}")))
            .flatten()
    })
}

/// `require(id)` and `esm(id)` of js/intrinsics.ts, in the realm's data: they are dropped with it.
struct Installed {
    require: JsObject,
    esm: JsObject,
}

fn call(which: fn(&Installed) -> &JsObject, id: usize, context: &mut Context) -> JsResult<JsValue> {
    let f = context
        .get_data::<Installed>()
        .map(|installed| which(installed).clone())
        .ok_or_else(|| {
            JsNativeError::error().with_message("the builtin modules are not installed")
        })?;
    f.call(&JsValue::undefined(), &[JsValue::from(id as u32)], context)
}

/// The exports of the builtin module `id`, which is evaluated the first time.
pub fn require(id: usize, context: &mut Context) -> JsResult<JsValue> {
    call(|i| &i.require, id, context)
}

/// An ES module of the builtin module `id`: `default` is its exports, and each of their own enumerable string
/// keys is a named export, as Bun's loader makes for `import { join } from "node:path"`.
pub fn module(id: usize, context: &mut Context) -> JsResult<Module> {
    let pairs = call(|i| &i.esm, id, context)?;
    let pairs = pairs
        .as_object()
        .and_then(|o| JsArray::from_object(o.clone()).ok())
        .ok_or_else(|| JsNativeError::typ().with_message("esm(id) returns [name, value][]"))?;
    let mut exports = Vec::new();
    for i in 0..pairs.length(context)? {
        let pair = pairs.get(i, context)?;
        let pair = pair
            .as_object()
            .ok_or_else(|| JsNativeError::typ().with_message("a [name, value] pair"))?;
        let name = pair.get(0, context)?.to_string(context)?;
        exports.push((name, pair.get(1, context)?));
    }
    let names: Vec<JsString> = exports.iter().map(|(name, _)| name.clone()).collect();
    let initializer =
        boa_engine::module::SyntheticModuleInitializer::from_copy_closure_with_captures(
            |module, exports: &Vec<(JsString, JsValue)>, _| {
                for (name, value) in exports {
                    module.set_export(name, value.clone())?;
                }
                Ok(())
            },
            exports,
        );
    Ok(Module::synthetic(&names, initializer, None, None, context))
}

fn index(args: &[JsValue], context: &mut Context) -> JsResult<usize> {
    Ok(args.get_or_undefined(0).to_u32(context)? as usize)
}

/// `compile(id)`: the function of the module's intrinsics, or `undefined` for a native module.
fn compile(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = index(args, context)?;
    let Some(source) = MODULES.get(id).and_then(|m| m.source) else {
        return Ok(JsValue::undefined());
    };
    let path = std::path::PathBuf::from(format!("builtin://{}", MODULES[id].id));
    context.eval(Source::from_bytes(source).with_path(&path))
}

fn strings<'a>(values: impl IntoIterator<Item = &'a str>, context: &mut Context) -> JsValue {
    let values: Vec<JsValue> = values
        .into_iter()
        .map(|s| JsString::from(s).into())
        .collect();
    JsArray::from_iter(values, context).into()
}

fn intrinsic_names(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = index(args, context)?;
    let names = MODULES.get(id).map_or(&[][..], |m| m.intrinsics);
    Ok(strings(names.iter().copied(), context))
}

/// `lazy(id)`: the value `$bindgen`/`$cpp` call `id` makes, or `undefined` if Boa has none yet.
fn lazy(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = index(args, context)?;
    let Some(native) = NATIVES.get(id) else {
        return Ok(JsValue::undefined());
    };
    let global = bun_js::JSGlobalObject::from_context(context);
    let value = match native.kind {
        NativeKind::Bind => crate::natives::bindgen(global, native.file, native.symbol),
        NativeKind::Cpp => crate::natives::cpp(global, native.file, native.symbol),
    };
    match value {
        Some(Ok(value)) => Ok(value.into_engine()),
        Some(Err(error)) => Err(global.into_engine_error(error)),
        None => Ok(JsValue::undefined()),
    }
}

/// `nativeName(id)`: `file:symbol`, what a missing native is called in its error.
fn native_name(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = index(args, context)?;
    Ok(NATIVES
        .get(id)
        .map_or(js_string!(), |n| {
            JsString::from(format!("{}:{}", n.file, n.symbol).as_str())
        })
        .into())
}

fn class_name(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = index(args, context)?;
    Ok(CLASSES
        .get(id)
        .map_or(JsValue::undefined(), |c| JsString::from(*c).into()))
}

/// `brand(value)`: what kind of built-in object `value` is, by its internal slots, as JavaScriptCore's
/// `@isPromise`, `@isMap`, … see it (a `Symbol.toStringTag` does not change it).
fn brand(_: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let Some(object) = args.get_or_undefined(0).as_object() else {
        return Ok(JsValue::undefined());
    };
    let o = || object.clone();
    let name = if JsPromise::from_object(o()).is_ok() {
        "Promise"
    } else if JsMap::from_object(o()).is_ok() {
        "Map"
    } else if JsSet::from_object(o()).is_ok() {
        "Set"
    } else if JsWeakMap::from_object(o()).is_ok() {
        "WeakMap"
    } else if JsWeakSet::from_object(o()).is_ok() {
        "WeakSet"
    } else if JsRegExp::from_object(o()).is_ok() {
        "RegExp"
    } else if object.is::<Proxy>() {
        "Proxy"
    } else if JsTypedArray::from_object(o()).is_ok() {
        "TypedArray"
    } else if JsDataView::from_object(o()).is_ok() {
        "DataView"
    } else if JsArrayBuffer::from_object(o()).is_ok() {
        "ArrayBuffer"
    } else if object.is_array() {
        "Array"
    } else {
        return Ok(JsValue::undefined());
    };
    Ok(JsString::from(name).into())
}

/// `promiseState(promise)`: 0 pending, 1 fulfilled, 2 rejected (`JSC::JSPromise::Status`).
fn promise_state(_: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let promise = args
        .get_or_undefined(0)
        .as_object()
        .and_then(|o| JsPromise::from_object(o.clone()).ok())
        .ok_or_else(|| JsNativeError::typ().with_message("not a promise"))?;
    Ok(match promise.state() {
        PromiseState::Pending => 0,
        PromiseState::Fulfilled(_) => 1,
        PromiseState::Rejected(_) => 2,
    }
    .into())
}

/// `promiseResult(promise)`: its value or reason, `undefined` while it is pending.
fn promise_result(_: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let promise = args
        .get_or_undefined(0)
        .as_object()
        .and_then(|o| JsPromise::from_object(o.clone()).ok())
        .ok_or_else(|| JsNativeError::typ().with_message("not a promise"))?;
    Ok(match promise.state() {
        PromiseState::Pending => JsValue::undefined(),
        PromiseState::Fulfilled(value) | PromiseState::Rejected(value) => value,
    })
}

/// `proxyField(proxy, 0 | 1)`: a proxy's target or handler, `null` once it is revoked.
fn proxy_field(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let field = args.get_or_undefined(1).to_u32(context)?;
    let Some(object) = args.get_or_undefined(0).as_object() else {
        return Ok(JsValue::undefined());
    };
    let Some(proxy) = object.downcast_ref::<Proxy>() else {
        return Ok(JsValue::undefined());
    };
    Ok(match proxy.target_and_handler() {
        Some((target, handler)) => if field == 0 { target } else { handler }.into(),
        None => JsValue::null(),
    })
}

/// `makeError(code, message)`: an error of `Bun::ErrorCode` number `code` (its constructor, `name` and `code`).
fn make_error(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let code = index(args, context)?;
    let message = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let code = *bun_js::ErrorCode::ALL
        .get(code)
        .ok_or_else(|| JsNativeError::range().with_message(format!("no error code {code}")))?;
    let global = bun_js::JSGlobalObject::from_context(context);
    Ok(code.fmt(global, format_args!("{message}")).into_engine())
}

/// `errorCode(code)`: the `code` of `Bun::ErrorCode` number `code` ("ERR_INVALID_ARG_TYPE").
fn error_code(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let code = index(args, context)?;
    Ok(bun_js::ErrorCode::ALL
        .get(code)
        .map_or(js_string!(), |code| JsString::from(code.code()))
        .into())
}

/// `createRequire(dir)`: `require` as a module in `dir` calls it (node:module's `createRequire`).
fn create_require(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let dir = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    Ok(crate::loader::make_require(std::path::Path::new(&dir), context).into())
}

const OPS: &str = "__bun_intrinsics_ops__";

/// Installs the builtin modules: evaluates js/intrinsics.ts with the native operations above.
pub fn install(context: &mut Context) -> JsResult<()> {
    let specifiers = strings(MODULES.iter().map(|m| m.specifier), context);
    let native: Vec<JsValue> = MODULES.iter().map(|m| m.source.is_none().into()).collect();
    let native = JsArray::from_iter(native, context);
    let mut ops = ObjectInitializer::new(context);
    for (name, f, len) in [
        (
            "compile",
            compile as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
            1,
        ),
        ("intrinsicNames", intrinsic_names, 1),
        ("lazy", lazy, 1),
        ("nativeName", native_name, 1),
        ("className", class_name, 1),
        ("brand", brand, 1),
        ("promiseState", promise_state, 1),
        ("promiseResult", promise_result, 1),
        ("proxyField", proxy_field, 2),
        ("makeError", make_error, 2),
        ("errorCode", error_code, 1),
        ("createRequire", create_require, 1),
    ] {
        ops.function(NativeFunction::from_fn_ptr(f), JsString::from(name), len);
    }
    ops.property(js_string!("specifiers"), specifiers, Attribute::all())
        .property(js_string!("native"), native, Attribute::all());
    let ops = ops.build();
    context.register_global_property(JsString::from(OPS), ops.clone(), Attribute::all())?;
    let source =
        crate::transpile::transpile(b"bun:intrinsics.ts", include_bytes!("js/intrinsics.ts"))
            .map_err(|err| JsNativeError::syntax().with_message(err))?;
    context.eval(Source::from_bytes(&source))?;
    let installed = ops.get(js_string!("installed"), context)?;
    let installed = installed
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("intrinsics.ts installs { require, esm }")
        })?
        .clone();
    let mut function = |key: &str| -> JsResult<JsObject> {
        let f = installed.get(JsString::from(key), context)?;
        f.as_object().filter(|o| o.is_callable()).ok_or_else(|| {
            JsNativeError::typ()
                .with_message(format!("intrinsics.ts: {key} is a function"))
                .into()
        })
    };
    let installed = Installed {
        require: function("require")?,
        esm: function("esm")?,
    };
    context.insert_data(installed);
    Ok(())
}
