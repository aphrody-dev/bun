use std::path::PathBuf;
use std::process::{Command, Output};

fn bun() -> Command {
    Command::new(std::env::var_os("NEXTEST_BIN_EXE_bun").unwrap_or(env!("CARGO_BIN_EXE_bun").into()))
}

fn fixtures() -> PathBuf {
    PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or(env!("CARGO_MANIFEST_DIR").into())).join("tests/fixtures")
}

fn text(output: &Output) -> (String, String) {
    (String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn version() {
    let out = bun().arg("--version").output().unwrap();
    let (stdout, stderr) = text(&out);
    assert!(stdout.trim().starts_with(|c: char| c.is_ascii_digit()), "{stdout}{stderr}");
    assert!(out.status.success());
    let out = bun().arg("--revision").output().unwrap();
    assert!(text(&out).0.contains('+'), "{:?}", text(&out));
}

#[test]
fn runs_typescript_modules_with_bun_apis() {
    let _ = std::fs::remove_file(fixtures().join("out.txt"));
    let out = bun()
        .args(["run", "main.ts", "a", "b"])
        .current_dir(fixtures())
        .env("BUN_BOA_TEST", "1")
        .output()
        .unwrap();
    let (stdout, stderr) = text(&out);
    let expected = "hello bun!
json boa 3
write 20 written by Bun.write true
json-file boa
missing false
timer done, version string, env true
argv a,b
main true
";
    assert_eq!(stdout, expected, "stderr: {stderr}");
    assert_eq!(out.status.code(), Some(3), "stderr: {stderr}");
}

#[test]
fn file_without_run_and_eval() {
    let out = bun().arg(fixtures().join("greet.ts")).output().unwrap();
    assert!(out.status.success(), "{:?}", text(&out));
    let out = bun().args(["-e", "const n: number = 40 + 2; console.log(n); process.exit(7)"]).output().unwrap();
    assert_eq!(text(&out).0, "42\n");
    assert_eq!(out.status.code(), Some(7));
}

#[test]
fn reports_errors() {
    let out = bun().args(["-e", "throw new TypeError('boom')"]).output().unwrap();
    let (_, stderr) = text(&out);
    assert!(stderr.contains("TypeError") && stderr.contains("boom"), "{stderr}");
    assert_eq!(out.status.code(), Some(1));
}

/// `ldd` + `nm`, in Rust: no C++ runtime among the shared libraries, and no C++ ABI or `std::` symbol defined or
/// imported. glibc's `__cxa_atexit`/`__cxa_finalize`/`__cxa_thread_atexit_impl` are C runtime and Rust's std uses them.
#[test]
fn binary_contains_no_cxx() {
    use object::{Object, ObjectSymbol};
    const GLIBC: [&str; 3] = ["__cxa_atexit", "__cxa_finalize", "__cxa_thread_atexit_impl"];
    const CXX_LIBS: [&str; 3] = ["libstdc++", "libc++", "libc++abi"];
    let bytes = std::fs::read(std::env::var_os("NEXTEST_BIN_EXE_bun").unwrap_or(env!("CARGO_BIN_EXE_bun").into())).unwrap();
    let file = object::File::parse(&*bytes).unwrap();

    let object::File::Elf64(elf) = &file else { panic!("expected a 64-bit ELF") };
    let needed: Vec<String> = elf_needed(elf);
    let cxx_libs: Vec<&String> = needed.iter().filter(|l| CXX_LIBS.iter().any(|c| l.starts_with(c))).collect();
    assert!(cxx_libs.is_empty(), "links C++ runtime: {cxx_libs:?} (needed: {needed:?})");

    let cxx_symbols: Vec<String> = file
        .symbols()
        .chain(file.dynamic_symbols())
        .filter_map(|s| s.name().ok().map(str::to_owned))
        .filter(|n| {
            (n.starts_with("__cxa_") && !GLIBC.contains(&n.as_str()))
                || n.starts_with("__gxx_personality")
                || ["_ZNSt", "_ZSt", "_ZTI", "_ZTV", "_ZTS", "_Znw", "_Zdl", "_Zna", "_Zda"].iter().any(|p| n.starts_with(p))
        })
        .collect();
    assert!(cxx_symbols.is_empty(), "C++ symbols: {:?}", &cxx_symbols[..cxx_symbols.len().min(20)]);
}

fn elf_needed(elf: &object::read::elf::ElfFile64) -> Vec<String> {
    use object::read::elf::{Dyn, FileHeader, SectionHeader};
    let endian = elf.endian();
    let data = elf.data();
    let sections = elf.elf_header().sections(endian, data).unwrap();
    let mut out = Vec::new();
    for section in sections.iter() {
        let Some((entries, link)) = section.dynamic(endian, data).unwrap() else { continue };
        let strings = sections.strings(endian, data, link).unwrap();
        for d in entries.iter().filter(|d| d.tag32(endian) == Some(object::elf::DT_NEEDED)) {
            out.push(String::from_utf8_lossy(d.string(endian, strings).unwrap()).into_owned());
        }
    }
    out
}

const MESSY_FORMATTED: &str = "const greeting: string = \"hi\";
function add(a: number, b: number) {
  return a + b;
}
export const obj = { a: 1, b: [1, 2, 3] };
";

#[test]
fn fmt_formats_files_and_stdin() {
    let dir = std::env::temp_dir().join("bun-boa-e2e").join("fmt");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(fixtures().join("fmt/messy.ts"), dir.join("messy.ts")).unwrap();

    let out = bun().args(["fmt", "--check", "messy.ts"]).current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{:?}", text(&out));

    let out = bun().args(["fmt", "messy.ts"]).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "{:?}", text(&out));
    assert_eq!(std::fs::read_to_string(dir.join("messy.ts")).unwrap(), MESSY_FORMATTED);

    let out = bun()
        .args(["fmt", "--stdin-filepath", "messy.ts"])
        .current_dir(&dir)
        .stdin(std::fs::File::open(fixtures().join("fmt/messy.ts")).unwrap())
        .output()
        .unwrap();
    assert_eq!(text(&out).0, MESSY_FORMATTED, "{:?}", text(&out));
    assert!(out.status.success());
}

#[test]
fn fmt_reads_bunfig() {
    let dir = std::env::temp_dir().join("bun-boa-e2e").join("fmt-bunfig");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bunfig.toml"), "[fmt]\nsemi = false\n").unwrap();
    let out = bun()
        .args(["fmt", "--stdin-filepath", "a.ts"])
        .current_dir(&dir)
        .stdin(std::fs::File::open(fixtures().join("fmt/messy.ts")).unwrap())
        .output()
        .unwrap();
    let (stdout, stderr) = text(&out);
    assert!(stdout.starts_with("const greeting: string = \"hi\"\n"), "{stdout}{stderr}");
    assert!(out.status.success(), "{stderr}");
}

#[test]
fn lint_reports_and_passes() {
    let dir = std::env::temp_dir().join("bun-boa-e2e").join("lint");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bad.ts"), "debugger;\nif (/a+b/.test(\"aab\")) console.log(1);\n").unwrap();
    std::fs::write(dir.join("good.ts"), "export const x = /a(b)?/.exec(\"ab\");\n").unwrap();
    let out = bun().args(["lint", "--rule", r#"{"no-debugger":"error"}"#, "bad.ts"]).current_dir(&dir).output().unwrap();
    let (stdout, stderr) = text(&out);
    assert!(format!("{stdout}{stderr}").contains("no-debugger"), "{stdout}{stderr}");
    assert_eq!(out.status.code(), Some(1), "{stdout}{stderr}");
    let out = bun().args(["lint", "good.ts"]).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "{:?}", text(&out));
}

/// A rule whose option is a regular expression: only the identifier that the pattern rejects is reported, so the
/// pattern ran, through `bun_yarr`'s C ABI on regress (`src/yarr.rs`).
#[test]
fn lint_runs_regular_expressions_through_yarr() {
    let dir = std::env::temp_dir().join("bun-boa-e2e").join("lint-regex");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ids.ts"), "export const snake_ok = 1;\nexport const BadName = 2;\n").unwrap();
    let rule = r#"{"id-match":["error","^[a-z]+(_[a-z]+)*$"]}"#;
    let out = bun().args(["lint", "--rule", rule, "ids.ts"]).current_dir(&dir).output().unwrap();
    let (stdout, stderr) = text(&out);
    let all = format!("{stdout}{stderr}");
    assert!(all.contains("id-match") && all.contains("BadName"), "{all}");
    // Not `snake_ok` alone: the code frame above `BadName` shows its line.
    assert!(!all.contains("'snake_ok'"), "{all}");
    assert_eq!(out.status.code(), Some(1), "{all}");
    let rule = r#"{"id-match":["error","^([a-z]+|[A-Z][a-zA-Z]+)(_[a-z]+)*$"]}"#;
    let out = bun().args(["lint", "--rule", rule, "ids.ts"]).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "{:?}", text(&out));
}

#[test]
fn runs_package_json_scripts() {
    let dir = std::env::temp_dir().join("bun-boa-e2e").join("scripts");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("node_modules/.bin")).unwrap();
    std::fs::write(
        dir.join("package.json"),
        r#"{"name":"p","scripts":{"prehi":"echo pre","hi":"echo hi $npm_lifecycle_event","posthi":"echo post","tool":"mytool","fail":"exit 4","code":"bun -e 'console.log(6*7)'"}}"#,
    )
    .unwrap();
    let tool = dir.join("node_modules/.bin/mytool");
    std::fs::write(&tool, "#!/bin/sh\necho tool \"$@\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();

    let out = bun().args(["run", "hi", "a b"]).current_dir(&dir).output().unwrap();
    assert_eq!(text(&out).0, "pre\nhi hi a b\npost\n", "{:?}", text(&out));
    assert!(out.status.success());
    let out = bun().args(["tool", "x"]).current_dir(&dir).output().unwrap();
    assert_eq!(text(&out).0, "tool x\n", "{:?}", text(&out));
    let out = bun().args(["run", "fail"]).current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let out = bun().args(["run", "code"]).current_dir(&dir).output().unwrap();
    assert_eq!(text(&out).0, "42\n", "{:?}", text(&out));
    let out = bun().arg("run").current_dir(&dir).output().unwrap();
    assert!(text(&out).0.contains("bun run hi"), "{:?}", text(&out));
}

#[test]
fn a_path_is_a_file_not_a_script() {
    let dir = std::env::temp_dir().join("bun-boa-e2e").join("script-path");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("package.json"), r#"{"scripts":{"./x.js":"echo script","x.js":"echo script"}}"#).unwrap();
    std::fs::write(dir.join("x.js"), "console.log('file');\n").unwrap();
    for args in [&["./x.js"][..], &["run", "./x.js"]] {
        let out = bun().args(args).current_dir(&dir).output().unwrap();
        assert_eq!(text(&out).0, "file\n", "{args:?}: {:?}", text(&out));
    }
    // A bare name is still a script first.
    let out = bun().args(["run", "x.js"]).current_dir(&dir).output().unwrap();
    assert_eq!(text(&out).0, "script\n", "{:?}", text(&out));
}

/// `bun build` bundles TypeScript across files through Bun's bundler, and the bundle runs on Boa.
#[cfg(feature = "build")]
#[test]
fn build_bundles_and_runs() {
    let dir = std::env::temp_dir().join("bun-boa-e2e").join("build");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.ts"), "export const x: number = 2;\n").unwrap();
    std::fs::write(dir.join("e.ts"), "import { x } from \"./a.ts\";\nconsole.log(x * 21);\n").unwrap();
    let out = bun().args(["build", "e.ts", "--outfile", "out/e.js"]).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "{:?}", text(&out));
    let js = std::fs::read_to_string(dir.join("out/e.js")).unwrap();
    assert!(!js.contains("import ") && !js.contains(": number"), "{js}");
    let out = bun().arg("out/e.js").current_dir(&dir).output().unwrap();
    assert_eq!(text(&out).0, "42\n", "{:?}", text(&out));
    let out = bun().args(["build", "e.ts", "--minify"]).current_dir(&dir).output().unwrap();
    assert!(out.status.success() && text(&out).0.len() < js.len(), "{:?}", text(&out));
    let out = bun().args(["build", "e.ts", "a.ts"]).current_dir(&dir).output().unwrap();
    assert!(!out.status.success(), "{:?}", text(&out));
}

/// File errors read as Bun's (bun_sys): errno name, message, syscall and path.
#[test]
fn file_errors_read_as_bun() {
    let code = "try { await Bun.file('./no-such-file.txt').text() } catch (e) { console.log(String(e.message)) }";
    let out = bun().args(["-e", code]).output().unwrap();
    let (stdout, _) = text(&out);
    assert!(stdout.contains("ENOENT") && stdout.contains("no-such-file.txt"), "{stdout:?}");
}

/// `bun test` on Boa: discovery, `bun:test` imports and globals, hooks, async and `done` tests, mocks, `test.each`,
/// skip/todo, a failing test that sets the exit code, `-t`, and Bun's summary.
#[test]
fn test_runs_bun_test_files() {
    let dir = fixtures().join("test");
    let out = bun().arg("test").current_dir(&dir).output().unwrap();
    let (stdout, stderr) = text(&out);
    let all = format!("{stdout}{stderr}");
    for line in [
        "(pass) math > adds",
        "(pass) math > nested > async",
        "(pass) mocks",
        "(pass) 2 + 3 = 5",
        "(pass) globals > works",
        "(fail) fails on purpose",
        "(skip) skipped",
        "(todo) later",
        " 7 pass",
        " 1 skip",
        " 1 todo",
        " 1 fail",
        "Ran 10 tests across 2 files.",
    ] {
        assert!(all.contains(line), "missing {line:?} in:\n{all}");
    }
    assert!(all.contains("Expected: 5") && all.contains("Received: 4"), "{all}");
    assert!(!all.contains("must not run") && !all.contains("beforeEach ran"), "{all}");
    assert_eq!(out.status.code(), Some(1), "{all}");

    let out = bun().args(["test", "nested"]).current_dir(&dir).output().unwrap();
    let all = format!("{}{}", text(&out).0, text(&out).1);
    assert!(all.contains(" 1 pass") && all.contains("across 1 file."), "{all}");
    assert!(out.status.success(), "{all}");

    let out = bun().args(["test", "math.test.ts", "-t", "^mocks$"]).current_dir(&dir).output().unwrap();
    let all = format!("{}{}", text(&out).0, text(&out).1);
    assert!(all.contains("(pass) mocks") && !all.contains("(pass) math > adds"), "{all}");

    let out = bun().args(["test", "no-such-filter"]).current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{:?}", text(&out));
    assert!(text(&out).1.contains("No tests found"), "{:?}", text(&out));
}

#[test]
fn semver_is_bound_through_bun_js() {
    let out = bun()
        .args(["test", "semver.test.ts"])
        .current_dir(fixtures().join("semver"))
        .output()
        .unwrap();
    let all = format!("{}{}", text(&out).0, text(&out).1);
    assert!(all.contains(" 4 pass") && all.contains(" 0 fail"), "{all}");
    assert!(out.status.success(), "{all}");
}

/// Bun's builtin modules (src/js), `require` and `__dirname` in an ES module, CommonJS modules, tsconfig.json
/// `paths`, and the helpers the transpiler imports from `bun:wrap` (`using`).
#[test]
fn builtin_modules_require_and_commonjs() {
    let out = bun().args(["run", "main.ts"]).current_dir(fixtures().join("modules")).output().unwrap();
    let (stdout, stderr) = text(&out);
    let expected = "function / function true true
mapped 1 2 counter.cjs
true 3 modules /
using true
";
    assert_eq!(stdout, expected, "stderr: {stderr}");
    assert!(out.status.success(), "stderr: {stderr}");
}
