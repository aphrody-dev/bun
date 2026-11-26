//! The realm a Bun program runs in: Boa's WinterTC runtime (console, timers, encoding, URL, structuredClone),
//! native operations in Rust, and `Bun`/`process` built on them by `js/bun.ts`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use boa_engine::object::ObjectInitializer;
use boa_engine::object::builtins::JsUint8Array;
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

fn sys_error(err: bun_sys::Error) -> boa_engine::JsError {
    // bun_sys::Error prints as Bun does: "ENOENT: no such file or directory, open 'path'".
    JsNativeError::error().with_message(err.to_string()).into()
}

fn read_text(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = string(args, 0, context)?;
    let bytes = bun_sys::File::read_from(bun_sys::Fd::cwd(), path.as_bytes()).map_err(sys_error)?;
    Ok(JsString::from(String::from_utf8_lossy(&bytes).as_ref()).into())
}

fn read_bytes(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = string(args, 0, context)?;
    let bytes = bun_sys::File::read_from(bun_sys::Fd::cwd(), path.as_bytes()).map_err(sys_error)?;
    Ok(JsUint8Array::from_iter(bytes, context)?.into())
}

fn exists(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(bun_sys::exists(string(args, 0, context)?.as_bytes()).into())
}

fn size(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = string(args, 0, context)?;
    let size = bun_sys::File::openat(bun_sys::Fd::cwd(), path.as_bytes(), bun_sys::O::RDONLY, 0)
        .and_then(|f| f.stat())
        .map(|st| st.st_size.max(0) as u64)
        .unwrap_or(0);
    Ok((size as f64).into())
}

fn write(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = string(args, 0, context)?;
    let data = args.get_or_undefined(1);
    let bytes = match data
        .as_object()
        .and_then(|o| JsUint8Array::from_object(o.clone()).ok())
    {
        Some(array) => array.iter(context).collect::<Vec<u8>>(),
        None => data
            .to_string(context)?
            .to_std_string_escaped()
            .into_bytes(),
    };
    // make_open creates the parent directories, as Bun.write does.
    let file = bun_sys::File::make_open(
        path.as_bytes(),
        bun_sys::O::WRONLY | bun_sys::O::CREAT | bun_sys::O::TRUNC,
        0o664,
    )
    .map_err(sys_error)?;
    file.write_all(&bytes).map_err(sys_error)?;
    Ok((bytes.len() as f64).into())
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
            "readText",
            read_text as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
            1,
        ),
        ("readBytes", read_bytes, 1),
        ("exists", exists, 1),
        ("size", size, 1),
        ("write", write, 2),
        ("stdout", stdout, 1),
        ("stderr", stderr, 1),
        ("exit", exit, 1),
        ("cwd", cwd, 0),
        ("nanoseconds", nanoseconds, 0),
    ] {
        ops.function(NativeFunction::from_fn_ptr(f), JsString::from(name), len);
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
    let mut context = Context::builder()
        .module_loader(loader)
        .build()
        .map_err(|err| JsNativeError::error().with_message(err.to_string()))?;
    boa_runtime::register(
        boa_runtime::extensions::ConsoleExtension(boa_runtime::DefaultLogger),
        None,
        &mut context,
    )?;
    let ops = native(&mut context)?;
    context.register_global_property(js_string!("__bun_native_ops__"), ops, Attribute::all())?;
    let prelude = crate::transpile::transpile(b"bun:prelude.ts", include_bytes!("js/bun.ts"))
        .map_err(|err| JsNativeError::syntax().with_message(err))?;
    context.eval(Source::from_bytes(&prelude))?;
    Ok(context)
}
