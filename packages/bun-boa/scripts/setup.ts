// Prepares a checkout for `cargo` on the build host, every step concurrently: Bun's Rust vendor deps (from
// scripts/build/deps/*.ts), this package's patched crates (vendor.ts), and the generated Rust sources that Bun's
// crates `include!` (the codegen commands of build/debug/build.ninja, run without building anything native).
import { $ } from "bun";
import { join } from "node:path";
import { lolhtml } from "../../../scripts/build/deps/lolhtml.ts";
import { rustArgon2 } from "../../../scripts/build/deps/rust-argon2.ts";
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
  await Promise.all(commands.map(c => $`sh -c ${c}`.cwd(join(repo, "build/debug")).quiet()));
  return `codegen: ${commands.length} generators`;
}

const steps = [rustDep(lolhtml, "lolhtml"), rustDep(rustArgon2, "rust-argon2"), $`bun scripts/vendor.ts`.cwd(join(import.meta.dir, "..")).text(), generated()];
for (const r of await Promise.allSettled(steps)) console.log(r.status === "fulfilled" ? String(r.value).trim() : `error: ${r.reason}`);
