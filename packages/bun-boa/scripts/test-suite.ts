// Runs Bun's own test files with the Boa build's `bun test`, one process per file, all cores at once, and prints one
// JSON line: per directory and per file, the counts `bun test` printed. Runs on the build host (scripts/test-boa.ts).
//   bun scripts/test-suite.ts <bun-boa binary> [dir…]
import { Glob } from "bun";
import { availableParallelism } from "node:os";
import { join } from "node:path";

const [exe, ...dirs] = Bun.argv.slice(2);
if (!exe) throw new Error("usage: bun scripts/test-suite.ts <bun-boa binary> [dir…]");
const repo = join(import.meta.dir, "../../..");
const roots = dirs.length ? dirs : ["test/js/bun", "test/js/node", "test/js/web", "test/cli"];
const TIMEOUT_MS = Number(Bun.env.BUN_BOA_TEST_TIMEOUT ?? 30_000);

type Result = { file: string; pass: number; fail: number; skip: number; todo: number; exit: number | null; ms: number };
const files: string[] = [];
for (const root of roots)
  for await (const f of new Glob(`${root}/**/*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}`).scan({ cwd: repo }))
    if (!f.includes("/node_modules/")) files.push(f);
files.sort();

const count = (text: string, label: string) => Number(text.match(new RegExp(`^ (\\d+) ${label}$`, "m"))?.[1] ?? 0);
async function run(file: string): Promise<Result> {
  const start = performance.now();
  const proc = Bun.spawn([exe, "test", `./${file}`], {
    cwd: repo,
    stdin: "ignore",
    stdout: "ignore",
    stderr: "pipe",
    timeout: TIMEOUT_MS,
    killSignal: "SIGKILL",
    env: { ...Bun.env, NO_COLOR: "1" },
  });
  const stderr = await proc.stderr.text();
  const exit = await proc.exited;
  const killed = proc.signalCode !== null;
  return {
    file,
    pass: count(stderr, "pass"),
    fail: killed ? Math.max(1, count(stderr, "fail")) : count(stderr, "fail") || (exit === 0 ? 0 : 1),
    skip: count(stderr, "skip"),
    todo: count(stderr, "todo"),
    exit: killed ? null : exit,
    ms: Math.round(performance.now() - start),
  };
}

const results: Result[] = [];
let next = 0;
await Promise.all(
  Array.from({ length: availableParallelism() }, async () => {
    while (next < files.length) results.push(await run(files[next++]));
  }),
);
results.sort((a, b) => a.file.localeCompare(b.file));
console.log(JSON.stringify({ roots, results }));
