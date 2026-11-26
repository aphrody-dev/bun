// Prepares a checkout for `cargo` on the build host, every step concurrently: Bun's Rust vendor deps (from
// scripts/build/deps/*.ts), this package's patched crates (vendor.ts), and the generated Rust sources that Bun's
// crates `include!` (the codegen commands of build/debug/build.ninja, run without building anything native).
import { $ } from "bun";
import { join } from "node:path";
import { lolhtml } from "../../../scripts/build/deps/lolhtml.ts";
import { rustArgon2 } from "../../../scripts/build/deps/rust-argon2.ts";
import { WEBKIT_VERSION } from "../../../scripts/build/deps/webkit.ts";
import type { Dependency } from "../../../scripts/build/source.ts";

const repo = join(import.meta.dir, "../../..");
const codegen = join(repo, "build/debug/codegen");
const GENERATORS = ["generate-removable-features.ts", "generate-node-errors.ts"];

async function rustDep(dep: Dependency, dir: string) {
  if (await Bun.file(join(repo, "vendor", dir, "Cargo.toml")).exists()) return `vendor/${dir}: present`;
  const source = dep.source(undefined as never);
  if (source.kind !== "github-archive") throw new Error(`${dep.name}: unsupported source ${source.kind}`);
  const cache = join(repo, "build/vendor-cache");
  await $`bun scripts/build/fetch-cli.ts dep ${dep.name} ${source.repo} ${source.commit} vendor/${dir} ${cache} ${dep.patches ?? []}`.cwd(repo).quiet();
  return `vendor/${dir}: fetched`;
}

async function generated() {
  if (!(await Bun.file(join(codegen, "build_options.rs")).exists())) await $`bun run build --configure-only`.cwd(repo).quiet();
  const commands = (await $`ninja -C build/debug -t commands`.cwd(repo).quiet().text())
    .split("\n")
    .filter(line => GENERATORS.some(g => line.includes(g)));
  await Promise.all([...commands.map(c => $`sh -c ${c}`.cwd(join(repo, "build/debug")).quiet()), aotSchema()]);
  return `codegen: ${commands.length + 1} generators`;
}

// `bun_aot_interface` includes the Rust half of WebKit's AOT schema. Its generator is TypeScript in WebKit's tree:
// only `JavaScriptCore/aot/ir`'s schema and generator are fetched, at the pinned commit, nothing native.
async function aotSchema() {
  if (await Bun.file(join(codegen, "aot_schema_interface.rs")).exists()) return;
  const dir = join(repo, `build/vendor-cache/webkit-aot-${WEBKIT_VERSION}`);
  const tree = `repos/oven-sh/WebKit/git/trees/${WEBKIT_VERSION}:Source/JavaScriptCore/aot/ir?recursive=1`;
  const files = ((await $`gh api ${tree}`.json()) as { tree: { type: string; path: string }[] }).tree
    .filter(e => e.type === "blob" && /^(generator\/|schema\/|AOTSchema\.def$)/.test(e.path))
    .map(e => e.path);
  const raw = `https://raw.githubusercontent.com/oven-sh/WebKit/${WEBKIT_VERSION}/Source/JavaScriptCore/aot/ir`;
  await Promise.all(files.map(async f => Bun.write(join(dir, f), await fetch(`${raw}/${f}`).then(r => (r.ok ? r : Promise.reject(new Error(`${f}: ${r.status}`)))))));
  await $`bun run ${join(dir, "generator/index.ts")} --schema ${join(dir, "AOTSchema.def")} --rust-out ${codegen}`.cwd(repo).quiet();
}

// The OFL faces kitesurf-cdp embeds (no system fonts), from kitesurf's font set.
const FONTS_BASE = "https://raw.githubusercontent.com/cloudflare/kitesurf/0affe70c9202b80396e3c5bf147018cb760af633/packages/engine/public/fonts";
const FONTS = ["Arimo.ttf", "Arimo-Italic.ttf", "Tinos-Regular.ttf", "Tinos-Bold.ttf", "Tinos-Italic.ttf", "NotoSansMono.ttf", "NotoSans.ttf", "NotoSansSymbols2.ttf", "OFL.txt"];

async function fonts() {
  const dir = join(import.meta.dir, "../vendor/fonts");
  const missing = [];
  for (const name of FONTS) if (!(await Bun.file(join(dir, name)).exists())) missing.push(name);
  await Promise.all(
    missing.map(async name => {
      const res = await fetch(`${FONTS_BASE}/${name}`);
      if (!res.ok) throw new Error(`fonts/${name}: ${res.status}`);
      await Bun.write(join(dir, name), res);
    }),
  );
  return `vendor/fonts: ${missing.length} fetched`;
}

const steps = [rustDep(lolhtml, "lolhtml"), rustDep(rustArgon2, "rust-argon2"), $`bun scripts/vendor.ts`.cwd(join(import.meta.dir, "..")).text(), generated(), fonts()];
for (const r of await Promise.allSettled(steps)) console.log(r.status === "fulfilled" ? String(r.value).trim() : `error: ${r.reason}`);
