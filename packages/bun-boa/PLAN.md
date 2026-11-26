# Bun without JavaScriptCore and without C++

`bun-boa` builds a `bun` binary from Bun's Rust crates in which every C/C++ dependency is replaced by Rust.
It merges Bun, Dioxus/Blitz and kitesurf without breaking any of them and without weighing on Bun:

- `packages/bun-boa/` is a standalone Cargo workspace (the Dioxus layout: a crate in `packages/`). Bun's root
  workspace and lockfile are untouched; Bun's `src/*` crates are path dependencies.
- `patches/<crate>-<version>/` holds git-format-patch series against crates.io tarballs, kitesurf's series first
  (Boa, Blitz, Stylo, Taffy, Parley, vello_cpu, usvg), ours on top. `bun run vendor` materialises them into
  the gitignored `vendor/`, and `[patch.crates-io]` wires them in. A crate that is not on crates.io names its
  base in `patches/<crate>-<version>/SOURCE` (`<git-url>@<ref>:<path>`) and is a path dependency.
- `aphrody-dev/boa` and `aphrody-dev/blitz` are forks for upstream PRs only, never dependencies.
- Builds never run locally: `bun run build` / `bun run test` (`scripts/remote.ts`) give the build host (n1,
  `/srv/xbuild` XFS) its own clone of HEAD from GitHub, rsync only the local delta, run `scripts/setup.ts`
  (Bun's vendored Rust deps from `scripts/build/deps`, the patched crates, and the codegen commands from
  `build.ninja`), then cargo with sccache-dist across the cluster.

## Replacement map

| C/C++ today                                   | Rust replacement                                                      |
| --------------------------------------------- | --------------------------------------------------------------------- |
| WebKit JavaScriptCore                         | Boa (fork)                                                            |
| WebCore (DOM, HTML/CSS, WebView)              | Blitz (fork): blitz-dom/html, Stylo, Taffy, Parley, Vello / vello_cpu |
| WebCore streams, Blob, Response, events       | Rust on Boa in `src/boa`                                              |
| WTF strings, dtoa, `parseDouble`, URL         | Rust (`ryu-js`, `fast-float2`, `url`)                                 |
| ICU (bundled with WebKit)                     | ICU4X (already used by Boa)                                           |
| BoringSSL (TLS, WebCrypto, `node:crypto`)     | rustls + `rustls-rustcrypto`, RustCrypto crates                       |
| Highway SIMD kernels, simdutf                 | Rust (`std::simd`/scalar) in the native shim module                   |
| mimalloc                                      | Rust global allocator                                                 |
| uSockets / uWebSockets (`uws_sys`)            | Rust sockets on `mio`, `bun_http1`/`bun_hpack`/`bun_websocket`        |
| lsquic / ls-qpack (HTTP/3)                    | `quinn` + `h3`                                                        |
| SQLite (`bun:sqlite`)                         | Turso/Limbo (`turso_core`)                                            |
| TinyCC (`bun:ffi` `cc`)                       | Cranelift JIT                                                         |
| libuv (Windows event loop)                    | Rust IOCP (`mio`/`compio`)                                            |
| V8 C++ API shim for native addons             | Dropped. N-API (C ABI) gets implemented in Rust over Boa              |

Already Rust and reused as is: `js_parser`, `js_printer`, `ast`, `resolver`, `css`, `lint`, `format`,
`sourcemap`, `semver`, `glob`, `md`, `ini`, `dotenv`, `zstd`, lol-html, rust-argon2.

## Phases

0. **Infra**: Bun codegen without WebKit, forks as path deps, Rust native shims (start from
   `bun_sema_standalone::native`), and a test that the linked binary has no C++ in it.
1. **Core runtime**: Bun's transpiler, then Boa ES modules, an event loop (microtasks and timers),
   `console`, `process`, `Bun.file`/`Bun.write`/`Bun.env`/`Bun.version`, top-level await.
2. **CLI reuse**: `bun fmt`, `bun lint`, `bun run <script>`, `bun build` (decouple
   `bundler`→`event_loop`→`uws` from `bun_jsc_macros`/uWS).
3. **Web APIs**: `fetch` (Rust HTTP plus rustls), `URL`, `TextEncoder`/`TextDecoder`, streams, `Blob`,
   WebCrypto, `Bun.serve`.
4. **Node compat**: run Bun's own `src/js/node/*` builtins on Boa by providing their `$` intrinsics in Rust.
5. **Blitz**: headless `Bun.WebView` on Blitz (vello_cpu screenshots), `HTMLRewriter` on lol-html. First
   piece: `cdp/` (`kitesurf-cdp serve --host 127.0.0.1 --port N`), a headless browser on kitesurf's Blitz
   stack and `blitz-vibey-script` (Boa DOM bindings, a `SOURCE` series) behind the DevTools protocol, the
   `lightpanda serve` contract agent-browser launches. `bun run build:cdp`, `bun run test:cdp`.
6. **The rest**: `bun:sqlite` on Turso, `bun:ffi` on Cranelift, HTTP/3 on quinn, Windows I/O.
7. **Conformance**: run Bun's `test/js` suite with this binary and track the pass rate. Engine
   performance work happens in the Boa fork.
