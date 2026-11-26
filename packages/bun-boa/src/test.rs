//! `bun test [filters…] [-t pattern] [--bail]`: Bun's test runner on Boa. Each test file runs in a realm of its
//! own with `bun:test` (js/test.ts) loaded first, so `describe`/`test`/`expect` are globals as in Bun; the counts
//! and the summary are printed the way `bun test` prints them.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use boa_engine::builtins::promise::PromiseState;
use boa_engine::{Context, JsError, JsResult, JsValue, Module, Source, js_string};

use crate::loader::BunModuleLoader;

const SUFFIXES: &[&str] = &[".test", "_test", ".spec", "_spec"];
const EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
const SKIP_DIRS: &[&bun_paths::OSPathSlice] = &[b"node_modules", b".git"];

#[derive(Default, Clone, Copy)]
struct Counts {
    pass: u64,
    fail: u64,
    skip: u64,
    todo: u64,
    expects: u64,
}

fn is_test_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    EXTENSIONS.contains(&ext) && SUFFIXES.iter().any(|s| stem.ends_with(s))
}

#[cfg(not(windows))]
fn scan(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(root) = bun_sys::Dir::open(dir.as_os_str().as_encoded_bytes()) else {
        return;
    };
    let Ok(mut walker) = bun_sys::walker_skippable::walk_owned(root, &[], SKIP_DIRS) else {
        return;
    };
    while let Ok(Some(entry)) = walker.next() {
        if entry.kind != bun_sys::FileKind::File {
            continue;
        }
        let rel = String::from_utf8_lossy(entry.path.as_bytes()).into_owned();
        if is_test_file(&rel) {
            files.push(dir.join(rel));
        }
    }
}

#[cfg(windows)]
fn scan(_: &Path, _: &mut Vec<PathBuf>) {}

/// The files `bun test` runs: an argument that is a file runs as is, a directory is scanned, anything else filters
/// the test files under the working directory by substring, as Bun does.
fn files(filters: &[String], cwd: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut substrings = Vec::new();
    for filter in filters {
        let path = cwd.join(filter);
        if path.is_file() {
            out.push(path);
        } else if path.is_dir() && (filter.starts_with('.') || filter.starts_with('/')) {
            scan(&path, &mut out);
        } else {
            substrings.push(filter.as_str());
        }
    }
    if out.is_empty() || !substrings.is_empty() {
        let mut all = Vec::new();
        scan(cwd, &mut all);
        all.retain(|p| {
            let rel = p.strip_prefix(cwd).unwrap_or(p).to_string_lossy();
            substrings.is_empty() || substrings.iter().any(|s| rel.contains(s))
        });
        out.extend(all);
    }
    out.sort();
    out.dedup();
    out
}

fn settle(
    promise: boa_engine::object::builtins::JsPromise,
    context: &mut Context,
) -> JsResult<JsValue> {
    context.run_jobs()?;
    match promise.state() {
        PromiseState::Fulfilled(value) => Ok(value),
        PromiseState::Rejected(reason) => Err(JsError::from_opaque(reason)),
        PromiseState::Pending => Err(boa_engine::JsNativeError::error()
            .with_message("the test file never finished (a promise that never settles)")
            .into()),
    }
}

fn error_text(err: &JsError, context: &mut Context) -> String {
    let value = err
        .clone()
        .into_opaque(context)
        .unwrap_or_else(|e| JsValue::from(js_string!(e.to_string())));
    let stack = value
        .as_object()
        .and_then(|o| o.get(js_string!("stack"), context).ok())
        .filter(JsValue::is_string);
    stack
        .unwrap_or(value)
        .display()
        .to_string()
        .trim_matches('"')
        .to_string()
}

fn number(object: &JsValue, key: &str, context: &mut Context) -> u64 {
    object
        .as_object()
        .and_then(|o| o.get(js_string!(key), context).ok())
        .and_then(|v| v.as_number())
        .map_or(0, |n| n as u64)
}

fn run_file(path: &Path, pattern: Option<&str>) -> Counts {
    let loader = Rc::new(BunModuleLoader::default());
    let argv = vec![
        std::env::args().next().unwrap_or_else(|| "bun".into()),
        path.display().to_string(),
    ];
    let mut context = match crate::runtime::context(loader.clone(), None, argv) {
        Ok(context) => context,
        Err(err) => {
            eprintln!("error: {err}");
            return Counts {
                fail: 1,
                ..Counts::default()
            };
        }
    };
    let result = (|| -> JsResult<JsValue> {
        let runner = loader
            .builtin("bun:test", &mut context)
            .expect("bun:test is built in")?;
        settle(runner.load_link_evaluate(&mut context), &mut context)?;
        let module: Module = loader.load(path, &mut context)?;
        settle(module.load_link_evaluate(&mut context), &mut context)?;
        let pattern = serde_json::to_string(&pattern).unwrap_or_else(|_| "null".into());
        let run = context.eval(Source::from_bytes(&format!(
            "globalThis[Symbol.for(\"bun:test:run\")]({pattern} ?? undefined)"
        )))?;
        let promise = run
            .as_object()
            .and_then(|o| boa_engine::object::builtins::JsPromise::from_object(o.clone()).ok())
            .ok_or_else(|| {
                boa_engine::JsNativeError::typ()
                    .with_message("bun:test runner did not return a promise")
            })?;
        settle(promise, &mut context)
    })();
    match result {
        Ok(stats) => Counts {
            pass: number(&stats, "pass", &mut context),
            fail: number(&stats, "fail", &mut context),
            skip: number(&stats, "skip", &mut context),
            todo: number(&stats, "todo", &mut context),
            expects: number(&stats, "expects", &mut context),
        },
        Err(err) => {
            eprintln!(
                "{}\n(fail) (file) {}",
                error_text(&err, &mut context),
                path.display()
            );
            Counts {
                fail: 1,
                ..Counts::default()
            }
        }
    }
}

pub fn main(args: &[String]) -> i32 {
    let mut filters = Vec::new();
    let mut pattern: Option<String> = None;
    let mut bail = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-t" | "--test-name-pattern" => pattern = iter.next().cloned(),
            "--bail" => bail = true,
            "--timeout" | "--preload" | "--rerun-each" | "--reporter" | "--reporter-outfile"
            | "--coverage-dir" => {
                iter.next();
            }
            a if a.starts_with("--test-name-pattern=") => {
                pattern = Some(a["--test-name-pattern=".len()..].to_string())
            }
            a if a.starts_with('-') => {}
            a => filters.push(a.to_string()),
        }
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let files = files(&filters, &cwd);
    if files.is_empty() {
        eprintln!("No tests found!");
        return 1;
    }
    let start = std::time::Instant::now();
    let mut total = Counts::default();
    for file in &files {
        let rel = file.strip_prefix(&cwd).unwrap_or(file);
        eprintln!("\n{}:", rel.display());
        let c = run_file(file, pattern.as_deref());
        total.pass += c.pass;
        total.fail += c.fail;
        total.skip += c.skip;
        total.todo += c.todo;
        total.expects += c.expects;
        if bail && c.fail > 0 {
            break;
        }
    }
    eprintln!();
    for (n, label) in [
        (total.pass, "pass"),
        (total.skip, "skip"),
        (total.todo, "todo"),
        (total.fail, "fail"),
    ] {
        if n > 0 || label == "pass" || label == "fail" {
            eprintln!(" {n} {label}");
        }
    }
    eprintln!(" {} expect() calls", total.expects);
    let tests = total.pass + total.fail + total.skip + total.todo;
    eprintln!(
        "Ran {tests} test{} across {} file{}. [{:.2}ms]",
        if tests == 1 { "" } else { "s" },
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        start.elapsed().as_secs_f64() * 1000.0
    );
    i32::from(total.fail > 0)
}
