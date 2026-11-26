// Bun's own test suite (test/js/bun, test/js/node, test/js/web, test/cli) against the Boa build, on the cluster:
// n1 builds packages/bun-boa (scripts/remote.ts), every host gets this checkout's tree (remote.ts's sync) and the
// binary (from n1), and runs its nextest-style partition of the test files with packages/bun-boa/scripts/test-suite.ts,
// shares weighted by core count. Writes build/test-report.json: per directory the files and tests that pass, fail and
// skip, the pass rate, the revision, and the previous report's rate to compare with.
//   bun run test:boa [dir…]          BUN_TEST_HOSTS=n1,n2,n3  BUN_REMOTE_REPO=bun-nojsc-test (defaults)
import { $ } from "bun";
import { join } from "node:path";
import { sync } from "../packages/bun-boa/scripts/remote.ts";

const root = join(import.meta.dir, "..");
const out = join(root, "build/test-report.json");
const name = Bun.env.BUN_REMOTE_REPO ?? "bun-nojsc-test";
const hosts = (Bun.env.BUN_TEST_HOSTS ?? "n1,n2,n3").split(",");
const builder = hosts[0];
const dirs = Bun.argv.slice(2);
const scratch = `/srv/xbuild/scratch/${name}`;
const exe = `${scratch}/bun-boa`;
type Result = { file: string; pass: number; fail: number; skip: number; todo: number; exit: number | null; ms: number };

const ssh = (host: string, script: string) => $`ssh -o BatchMode=yes ${host} ${script}`;
// The builder knows the other hosts under other aliases: it is pointed at the address this machine resolves them to.
const address = async (host: string) => {
  const config = Object.fromEntries((await $`ssh -G ${host}`.text()).split("\n").map(line => line.split(" ", 2)));
  return `${config.user}@${config.hostname}`;
};
const log = (msg: string) => console.error(`[test:boa] ${msg}`);

async function build() {
  const proc = Bun.spawn(
    ["bun", "scripts/remote.ts", "bash", "-c", 'cargo build --bin bun >&2 && echo "$CARGO_TARGET_DIR/debug/bun"'],
    {
      cwd: join(root, "packages/bun-boa"),
      env: { ...Bun.env, BUN_BUILD_HOST: builder, BUN_REMOTE_REPO: name },
      stdout: "pipe",
      stderr: "inherit",
    },
  );
  const stdout = await proc.stdout.text();
  if ((await proc.exited) !== 0) throw new Error(`build on ${builder} failed (exit ${proc.exitCode})`);
  const binary = stdout.trim().split("\n").at(-1)!;
  // Without debug info the binary is a fraction of the size to copy; the symbol table stays for backtraces.
  await ssh(builder, `set -e; mkdir -p ${scratch}; objcopy --strip-debug ${binary} ${exe}.tmp; mv ${exe}.tmp ${exe}`);
  log(`built ${binary} on ${builder}`);
}

// The root's and test/'s dependencies (test/ links some from ../node_modules), as upstream CI installs them;
// lifecycle scripts (browser downloads) are skipped.
async function prepare(host: string) {
  const tree = host === builder ? `/srv/xbuild/src/${name}` : await sync(host, name);
  for (const dir of [tree, `${tree}/test`]) {
    const install = await ssh(host, `cd ${dir} && bun install --frozen-lockfile --ignore-scripts --silent`).nothrow();
    if (install.exitCode)
      log(`${host}: ${dir} install failed (exit ${install.exitCode}): ${install.stderr.toString().trim()}`);
  }
  return { host, tree, cores: Number((await ssh(host, "nproc").text()).trim()) };
}

// The builder's tree is synced by remote.ts inside build(); its install waits for that.
const [, ...others] = await Promise.all([build(), ...hosts.slice(1).map(prepare)]);
const ready = [await prepare(builder), ...others];
await Promise.all(
  ready.slice(1).map(async ({ host }) => {
    await ssh(host, `mkdir -p ${scratch}`);
    await ssh(builder, `rsync -az -e 'ssh -o BatchMode=yes' ${exe} ${await address(host)}:${exe}`);
  }),
);

const shards = ready.reduce((n, h) => n + h.cores, 0);
let first = 1;
const started = performance.now();
const runs = await Promise.all(
  ready.map(async ({ host, tree, cores }) => {
    const partition = `hash:${first}-${first + cores - 1}/${shards}`;
    first += cores;
    log(`${host}: ${partition}`);
    const start = performance.now();
    const r = await ssh(
      host,
      `cd ${tree} && bun packages/bun-boa/scripts/test-suite.ts ${exe} --partition ${partition} ${dirs.join(" ")}`,
    ).nothrow();
    // A test that was killed at its timeout can leave the processes it spawned behind.
    await ssh(host, `pkill -KILL -f '^${exe} ' || true`).nothrow();
    const line = r.stdout
      .toString()
      .split("\n")
      .findLast(l => l.startsWith("{"));
    if (r.exitCode || !line)
      throw new Error(`${host}: test-suite exit ${r.exitCode}\n${r.stderr.toString().slice(-4000)}`);
    const { roots, results } = JSON.parse(line) as { roots: string[]; results: Result[] };
    log(`${host}: ${results.length} files in ${Math.round((performance.now() - start) / 1000)}s`);
    return { host, partition, roots, results, ms: Math.round(performance.now() - start) };
  }),
);
const roots = runs[0].roots;
const results = runs.flatMap(r => r.results).sort((a, b) => a.file.localeCompare(b.file));

const summary = (rs: Result[]) => {
  const pass = rs.reduce((t, r) => t + r.pass, 0);
  const fail = rs.reduce((t, r) => t + r.fail, 0);
  return {
    files: rs.length,
    files_passed: rs.filter(r => r.exit === 0 && r.pass > 0).length,
    timeouts: rs.filter(r => r.exit === null).length,
    pass,
    fail,
    skip: rs.reduce((t, r) => t + r.skip, 0),
    todo: rs.reduce((t, r) => t + r.todo, 0),
    pass_rate: pass + fail ? Number((pass / (pass + fail)).toFixed(4)) : 0,
  };
};
// Each root, and each directory directly under it.
const groups = new Map<string, Result[]>();
for (const r of results) {
  const top = roots.find(d => r.file.startsWith(d + "/"))!;
  const rest = r.file.slice(top.length + 1).split("/");
  for (const key of [top, rest.length > 1 ? `${top}/${rest[0]}` : undefined])
    if (key) groups.set(key, [...(groups.get(key) ?? []), r]);
}

const previous = (await Bun.file(out).exists()) ? await Bun.file(out).json() : undefined;
const report = {
  revision: (await $`git -C ${root} rev-parse --short HEAD`.text()).trim(),
  date: new Date().toISOString(),
  hosts: runs.map(({ host, partition, results, ms }) => ({ host, partition, files: results.length, ms })),
  wall_ms: Math.round(performance.now() - started),
  total: summary(results),
  previous: previous
    ? {
        revision: previous.revision,
        date: previous.date,
        pass_rate: previous.total?.pass_rate,
        pass: previous.total?.pass,
      }
    : null,
  by_dir: Object.fromEntries([...groups.keys()].sort().map(k => [k, summary(groups.get(k)!)])),
  files: results,
};
await Bun.write(out, JSON.stringify(report, null, 2) + "\n");

const t = report.total;
const pct = (rate: number) => `${(rate * 100).toFixed(2)}%`;
console.log(
  `${report.revision}: ${t.pass} pass / ${t.fail} fail / ${t.skip} skip (${pct(t.pass_rate)}), ` +
    `${t.files_passed}/${t.files} files green, ${t.timeouts} timeouts, ${Math.round(report.wall_ms / 1000)}s` +
    (report.previous ? `; previous ${report.previous.revision}: ${pct(report.previous.pass_rate ?? 0)}` : ""),
);
for (const d of roots) {
  const s = report.by_dir[d];
  if (s)
    console.log(`  ${d.padEnd(14)} ${s.pass} pass, ${s.fail} fail, ${s.skip} skip, ${s.files_passed}/${s.files} files`);
}
