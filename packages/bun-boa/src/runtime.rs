//! The realm a Bun program runs in: Boa's WinterTC runtime (console, timers, encoding, URL, structuredClone),
//! native operations in Rust, and `Bun`/`process` built on them by `js/bun.ts`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{
    Context, JsArgs, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source, js_string,
};

use crate::loader::BunModuleLoader;

static MAIN: OnceLock<PathBuf> = OnceLock::new();
static ARGV: OnceLock<Vec<String>> = OnceLock::new();

pub fn main_path() -> Option<&'static Path> {
    MAIN.get().map(PathBuf::as_path)
}

fn string(args: &[JsValue], i: usize, context: &mut Context) -> JsResult<String> {
    Ok(args
        .get_or_undefined(i)
        .to_string(context)?
        .to_std_string_escaped())
}

fn print(args: &[JsValue], context: &mut Context, out: &mut dyn Write) -> JsResult<JsValue> {
    let text = string(args, 0, context)?;
    let _ = out.write_all(text.as_bytes());
    Ok(JsValue::undefined())
}

fn stdout(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    print(args, context, &mut std::io::stdout().lock())
}

fn stderr(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    print(args, context, &mut std::io::stderr().lock())
}

fn exit(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let code = args.get_or_undefined(0).to_i32(context)?;
    let _ = std::io::stdout().flush();
    std::process::exit(code)
}

fn cwd(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let dir = std::env::current_dir().unwrap_or_default();
    Ok(JsString::from(dir.display().to_string().as_str()).into())
}

fn nanoseconds(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    Ok((START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as f64)
        .into())
}

fn strings(values: impl IntoIterator<Item = String>, context: &mut Context) -> JsValue {
    let values: Vec<JsValue> = values
        .into_iter()
        .map(|s| JsString::from(s.as_str()).into())
        .collect();
    boa_engine::object::builtins::JsArray::from_iter(values, context).into()
}

fn native(context: &mut Context) -> JsResult<JsValue> {
    let env = ObjectInitializer::new(context).build();
    for (key, value) in std::env::vars_os() {
        let (key, value) = (
            key.to_string_lossy().into_owned(),
            value.to_string_lossy().into_owned(),
        );
        env.set(
            JsString::from(key.as_str()),
            JsString::from(value.as_str()),
            false,
            context,
        )?;
    }
    let argv = strings(ARGV.get().cloned().unwrap_or_default(), context);
    let main = JsString::from(
        main_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .as_str(),
    );
    let version = JsString::from(bun_core::package_json_version);
    let revision = JsString::from(bun_core::env::GIT_SHA);
    let mut ops = ObjectInitializer::new(context);
    for (name, f, len) in [
        (
            "printStdout",
            stdout as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
            1,
        ),
        ("printStderr", stderr, 1),
        ("exit", exit, 1),
        ("cwd", cwd, 0),
        ("nanoseconds", nanoseconds, 0),
    ] {
        ops.function(NativeFunction::from_fn_ptr(f), JsString::from(name), len);
    }
    // The crates on bun_js: the namespaces of each `.bind.ts` (`Bun.semver`), and Blob, File, Bun.file, Bun.write
    // and the standard files.
    let global = bun_js::JSGlobalObject::from_context(ops.context());
    let mut bound = bun_semver_js::generated_bindgen::semver_object::namespaces(global)
        .map_err(|error| global.into_engine_error(error))?;
    let clock = bun_runtime_js::generated_bindgen::performance::namespaces(global)
        .map_err(|error| global.into_engine_error(error))?;
    bound.extend(
        bun_runtime_js::webcore::blob::exports(global)
            .map_err(|error| global.into_engine_error(error))?,
    );
    for (name, value) in bound.into_iter().chain(clock) {
        ops.property(JsString::from(name), value.into_engine(), Attribute::all());
    }
    ops.property(js_string!("env"), env, Attribute::all())
        .property(js_string!("argv"), argv, Attribute::all())
        .property(js_string!("main"), main, Attribute::all())
        .property(js_string!("version"), version, Attribute::all())
        .property(js_string!("revision"), revision, Attribute::all())
        .property(
            js_string!("platform"),
            JsString::from(std::env::consts::OS),
            Attribute::all(),
        )
        .property(js_string!("arch"), JsString::from(arch()), Attribute::all())
        .property(js_string!("pid"), std::process::id(), Attribute::all());
    Ok(ops.build().into())
}

fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// A realm with Bun's globals; `main` is the entry point and `argv` is `process.argv`.
pub fn context(
    loader: Rc<BunModuleLoader>,
    main: Option<PathBuf>,
    argv: Vec<String>,
) -> JsResult<Context> {
    if let Some(main) = main {
        let _ = MAIN.set(main);
    }
    let _ = ARGV.set(argv);
    // What node:path (bun_node_path) completes a relative path with, as Bun's VM sets it at startup.
    if let Ok(cwd) = std::env::current_dir() {
        bun_paths::fs::FileSystem::init(cwd.as_os_str().as_encoded_bytes());
    }
    let mut context = Context::builder()
        .module_loader(loader.clone())
        .job_executor(Rc::new(bun_js::AsyncContextJobExecutor::default()))
        .build()
        .map_err(|err| JsNativeError::error().with_message(err.to_string()))?;
    loader.install(&mut context);
    boa_runtime::register(
        boa_runtime::extensions::ConsoleExtension(boa_runtime::DefaultLogger),
        None,
        &mut context,
    )?;
    bun_runtime_js::web::performance::start();
    let ops = native(&mut context)?;
    context.register_global_property(js_string!("__bun_native_ops__"), ops, Attribute::all())?;
    let performance = crate::transpile::transpile(b"bun:performance.ts", include_bytes!("js/performance.ts"))
        .map_err(|err| JsNativeError::syntax().with_message(err))?;
    context.eval(Source::from_bytes(&performance))?;
    let prelude = crate::transpile::transpile(b"bun:prelude.ts", include_bytes!("js/bun.ts"))
        .map_err(|err| JsNativeError::syntax().with_message(err))?;
    context.eval(Source::from_bytes(&prelude))?;
    crate::builtins::install(&mut context)?;
    Ok(context)
}
