use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bun() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bun"))
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
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

#[test]
fn binary_contains_no_cxx() {
    let bytes = std::fs::read(env!("CARGO_BIN_EXE_bun")).unwrap();
    for needle in [&b"__cxa_"[..], b"__gxx_personality", b"libstdc++", b"libc++.so", b"_ZNSt"] {
        assert!(!bytes.windows(needle.len()).any(|w| w == needle), "found {}", String::from_utf8_lossy(needle));
    }
}
