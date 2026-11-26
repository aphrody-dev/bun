// The build baseline of packages/bun-boa, measured on the build host (n1): clean and incremental build time of the
// release and dev profiles, binary size, startup (`bun -e 0` under hyperfine), and the crate graph (crates, and the
// ones present in more than one version). One record per run is appended to build/build-bench.jsonl.
//   bun run build:bench              BUN_BUILD_HOST=n1, BUN_REMOTE_REPO=bun-nojsc-bench (defaults)
// Clean builds run without sccache, in a target dir of their own, so they measure compilation and not the cache.
import { $ } from "bun";
import { appendFile } from "node:fs/promises";
import { cpus, loadavg } from "node:os";
import { join } from "node:path";

const root = join(import.meta.dir, "..");

if (Bun.argv[2] === "--on-host") {
  // cwd: packages/bun-boa on the build host, with remote.ts's environment.
  const target = Bun.env.CARGO_TARGET_DIR!;
  // Other sessions on the host reinstall the shared rustup toolchain, deleting rustc under a running build: the
  // bench builds with a copy of it of its own.
  const toolchain = `${target}.toolchain`;
  const sysroot = (await $`rustc --print sysroot`.text()).trim();
  await $`rm -rf ${toolchain} && cp -a --reflink=auto ${sysroot} ${toolchain}`;
  const env = { ...Bun.env, RUSTC_WRAPPER: "", RUSTUP_TOOLCHAIN: toolchain };
  const log = (s: string) => console.error(`build-bench: ${s}`);
  const cargo = async (args: string[]) => {
    const start = performance.now();
    const proc = Bun.spawn(["cargo", ...args], { env, stdio: ["ignore", "ignore", "inherit"] });
    if ((await proc.exited) !== 0) throw new Error(`cargo ${args.join(" ")}: exit ${proc.exitCode}`);
    return Math.round(performance.now() - start) / 1000;
  };
  const touch = async (path: string) => {
    const file = Bun.file(path);
    await Bun.write(path, await file.bytes());
  };

  const host = {
    load: loadavg(),
    cpus: cpus().length,
    cpu: cpus()[0]?.model,
    rustc: (await $`rustc -V`.env(env).text()).trim(),
  };
  await $`cargo fetch --quiet`.env(env);
  await $`rm -rf ${target}`;

  const profiles: Record<string, unknown> = {};
  for (const [name, flags, dir] of [
    ["release", ["--release"], "release"],
    ["dev", [], "debug"],
  ] as const) {
    const build = ["build", "--bin", "bun", ...flags];
    log(`${name}: clean`);
    const clean = await cargo(build);
    log(`${name}: incremental`);
    await touch("src/main.rs");
    const leaf = await cargo(build);
    await touch("../../src/bun_core/lib.rs");
    const core = await cargo(build);
    const bin = join(target, dir, "bun");
    const json = join(target, `hyperfine-${name}.json`);
    await $`hyperfine -N --warmup 5 --runs 50 --style none --export-json ${json} ${`${bin} -e 0`}`.quiet();
    const [run] = (await Bun.file(json).json()).results;
    profiles[name] = {
      clean_s: clean,
      incremental_s: { leaf, bun_core: core },
      size_bytes: Bun.file(bin).size,
      startup_ms: { mean: run.mean * 1e3, stddev: run.stddev * 1e3, min: run.min * 1e3, max: run.max * 1e3 },
    };
  }

  // The graph `cargo build --bin bun` compiles: normal and build edges from bun-boa, no dev-dependencies.
  type Node = { id: string; deps: { pkg: string; dep_kinds: { kind: string | null }[] }[] };
  type Package = { id: string; name: string; version: string };
  const meta = (await $`cargo metadata --format-version 1 --filter-platform x86_64-unknown-linux-gnu`
    .env(env)
    .quiet()
    .json()) as { packages: Package[]; resolve: { nodes: Node[] } };
  const nodes = new Map(meta.resolve.nodes.map(n => [n.id, n]));
  const packages = new Map(meta.packages.map(p => [p.id, p]));
  const seen = new Set<string>();
  const stack = [meta.packages.find(p => p.name === "bun-boa")!.id];
  while (stack.length) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id)!.deps)
      if (dep.dep_kinds.some(k => k.kind !== "dev")) stack.push(dep.pkg);
  }
  const versions = new Map<string, string[]>();
  for (const id of seen) {
    const { name, version } = packages.get(id)!;
    versions.set(name, [...(versions.get(name) ?? []), version]);
  }
  const duplicates = Object.fromEntries(
    [...versions].filter(([, v]) => v.length > 1).sort(([a], [b]) => a.localeCompare(b)).map(([n, v]) => [n, v.sort()]),
  );
  const local = [...seen].filter(id => id.startsWith("path+")).length;

  console.log(
    JSON.stringify({
      host,
      profiles,
      crates: { total: seen.size, local, registry: seen.size - local, duplicates: Object.keys(duplicates).length },
      duplicates,
    }),
  );
  process.exit(0);
}

const host = Bun.env.BUN_BUILD_HOST ?? "n1";
const remote = Bun.spawn(["bun", "scripts/remote.ts", "bun", "../../scripts/build-bench.ts", "--on-host"], {
  cwd: join(root, "packages/bun-boa"),
  env: { ...Bun.env, BUN_BUILD_HOST: host, BUN_REMOTE_REPO: Bun.env.BUN_REMOTE_REPO ?? "bun-nojsc-bench" },
  stdout: "pipe",
  stderr: "inherit",
});
const stdout = await remote.stdout.text();
if ((await remote.exited) !== 0) throw new Error(`remote bench failed (exit ${remote.exitCode})`);
const line = stdout.split("\n").findLast(l => l.startsWith("{"));
if (!line) throw new Error("the remote run printed no record");
const record = {
  date: new Date().toISOString(),
  commit: (await $`git -C ${root} rev-parse HEAD`.text()).trim(),
  dirty: (await $`git -C ${root} status --porcelain`.text()).trim().length > 0,
  build_host: host,
  ...JSON.parse(line),
};
const out = join(root, "build/build-bench.jsonl");
await appendFile(out, JSON.stringify(record) + "\n");
const { duplicates, ...summary } = record;
console.log(JSON.stringify(summary, null, 2));
console.log(`appended to ${out}`);
