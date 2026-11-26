// Bun's own test suite (test/js/bun, test/js/node, test/js/web, test/cli) against the Boa build, on the cluster:
// builds packages/bun-boa on the build host, runs every test file there with its `bun test`
// (packages/bun-boa/scripts/test-suite.ts), and writes build/test-report.json: per directory the files and tests that
// pass, the pass rate, and the previous report's rate to compare with.
//   bun run test:boa [dir…]          BUN_BUILD_HOST=n1, BUN_REMOTE_REPO=bun-nojsc-test (defaults)
import { join } from "node:path";

const root = join(import.meta.dir, "..");
const out = join(root, "build/test-report.json");
type Result = { file: string; pass: number; fail: number; skip: number; todo: number; exit: number | null; ms: number };

const remote = Bun.spawn(
  [
    "bun",
    "scripts/remote.ts",
    "bash",
    "-c",
    'cargo build --bin bun >&2 && bun scripts/test-suite.ts "$CARGO_TARGET_DIR/debug/bun" "$@"',
    "test-suite",
    ...Bun.argv.slice(2),
  ],
  {
    cwd: join(root, "packages/bun-boa"),
    env: { ...Bun.env, BUN_REMOTE_REPO: Bun.env.BUN_REMOTE_REPO ?? "bun-nojsc-test" },
    stdout: "pipe",
    stderr: "inherit",
  },
);
const stdout = await remote.stdout.text();
if ((await remote.exited) !== 0) throw new Error(`remote test run failed (exit ${remote.exitCode})`);
const line = stdout.split("\n").findLast(l => l.startsWith("{"));
if (!line) throw new Error("the remote run printed no report");
const { roots, results } = JSON.parse(line) as { roots: string[]; results: Result[] };

const summary = (rs: Result[]) => {
  const tests = rs.reduce((t, r) => t + r.pass + r.fail, 0);
  const pass = rs.reduce((t, r) => t + r.pass, 0);
  return {
    files: rs.length,
    files_passed: rs.filter(r => r.exit === 0 && r.pass > 0).length,
    timeouts: rs.filter(r => r.exit === null).length,
    pass,
    fail: rs.reduce((t, r) => t + r.fail, 0),
    skip: rs.reduce((t, r) => t + r.skip, 0),
    todo: rs.reduce((t, r) => t + r.todo, 0),
    pass_rate: tests ? Number((pass / tests).toFixed(4)) : 0,
  };
};
const previous = (await Bun.file(out).exists()) ? await Bun.file(out).json() : undefined;
const report = {
  revision: (await Bun.$`git -C ${root} rev-parse --short HEAD`.text()).trim(),
  date: new Date().toISOString(),
  host: Bun.env.BUN_BUILD_HOST ?? "n1",
  total: summary(results),
  previous: previous ? { revision: previous.revision, pass_rate: previous.total?.pass_rate, pass: previous.total?.pass } : null,
  by_dir: Object.fromEntries(roots.map(r => [r, summary(results.filter(x => x.file.startsWith(r + "/")))])),
  files: results,
};
await Bun.write(out, JSON.stringify(report, null, 2));
const t = report.total;
console.log(
  `${t.pass} pass / ${t.fail} fail (${(t.pass_rate * 100).toFixed(2)}%), ${t.files_passed}/${t.files} files green, ${t.timeouts} timeouts` +
    (report.previous ? `; previous ${report.previous.revision}: ${((report.previous.pass_rate ?? 0) * 100).toFixed(2)}%` : ""),
);
for (const [dir, s] of Object.entries(report.by_dir))
  console.log(`  ${dir.padEnd(14)} ${s.pass} pass, ${s.fail} fail, ${s.files_passed}/${s.files} files`);
