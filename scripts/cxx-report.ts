// The C++ (and C) budget of the fork: lines of the fork's own C/C++ that the build compiles into bun (the `cxx` and
// `c` source lists of glob-sources.ts plus every header of the fork they include, resolved through bunIncludes), per
// directory, and the native dependencies of scripts/build/deps/ with the language they bring.
// Written to build/cxx-report.json; the goal is C++ = 0.
//   bun run cxx:report
import { Glob } from "bun";
import { dirname, join, relative } from "node:path";
import { bunIncludes } from "./build/flags.ts";
import { globAllSources } from "./glob-sources.ts";

const root = join(import.meta.dir, "..");
// Native dependencies (scripts/build/deps/*.ts) and the language they compile into bun.
const DEPS: Record<string, "c++" | "c" | "rust"> = {
  webkit: "c++",
  boringssl: "c",
  libuv: "c",
  lsquic: "c",
  lsqpack: "c",
  mimalloc: "c",
  sqlite: "c",
  tinycc: "c",
  lolhtml: "rust",
  "rust-argon2": "rust",
  "nodejs-headers": "c",
};

const sources = globAllSources() as Partial<ReturnType<typeof globAllSources>>;
const codegenDir = join(root, "build/debug/codegen");
const vendorDir = join(root, "vendor");
// Both platforms' include dirs: a header counts if any build of bun can reach it.
const includeDirs = [
  ...new Set([
    ...bunIncludes({ cwd: root, codegenDir, vendorDir, release: false, windows: false } as never),
    ...bunIncludes({ cwd: root, codegenDir, vendorDir, release: false, windows: true } as never),
  ]),
];
// The fork's own code: dependencies (vendor/) and generated code (build/) are not part of the budget.
const own = (path: string) =>
  path.startsWith(root + "/") && !/^(vendor|build|node_modules)\//.test(relative(root, path));

const include = /^\s*#\s*(?:include|import)\s*[<"]([^>"]+)[>"]/gm;
const lang = new Map<string, "cxx" | "c">();
const queue: [string, "cxx" | "c"][] = [
  ...(sources.cxx ?? []).map(p => [p, "cxx"] as [string, "cxx"]),
  ...(sources.c ?? []).map(p => [p, "c"] as [string, "c"]),
];
const text = new Map<string, string>();
while (queue.length) {
  const [path, l] = queue.pop()!;
  const seen = lang.get(path);
  if (seen === l || seen === "cxx") continue;
  lang.set(path, l);
  const body = text.get(path) ?? (await Bun.file(path).text());
  text.set(path, body);
  for (const [, name] of body.matchAll(include)) {
    for (const dir of [dirname(path), ...includeDirs]) {
      const candidate = join(dir, name);
      if (!(await Bun.file(candidate).exists())) continue;
      if (own(candidate)) queue.push([candidate, l]);
      break;
    }
  }
}

const src: Record<string, { cxx: number; c: number; files: number }> = {};
for (const [path, l] of lang) {
  if (!own(path)) continue;
  const rel = relative(root, path);
  const dir = rel.split("/").slice(0, 3).join("/");
  const entry = (src[dir] ??= { cxx: 0, c: 0, files: 0 });
  entry[l] += text.get(path)!.split("\n").length;
  entry.files++;
}
const deps = [];
for await (const file of new Glob("*.ts").scan({ cwd: join(root, "scripts/build/deps") })) {
  const name = file.replace(/\.ts$/, "");
  if (name === "index") continue;
  deps.push({ name, lang: DEPS[name] ?? "unknown" });
}
const total = Object.values(src).reduce((t, e) => ({ cxx: t.cxx + e.cxx, c: t.c + e.c }), { cxx: 0, c: 0 });
const report = {
  revision: (await Bun.$`git -C ${root} rev-parse --short HEAD`.text()).trim(),
  src_lines: total,
  cxx_deps: deps.filter(d => d.lang === "c++").map(d => d.name),
  c_deps: deps.filter(d => d.lang === "c").map(d => d.name),
  unknown_deps: deps.filter(d => d.lang === "unknown").map(d => d.name),
  by_dir: Object.fromEntries(Object.entries(src).sort((a, b) => b[1].cxx + b[1].c - (a[1].cxx + a[1].c))),
};
await Bun.write(join(root, "build/cxx-report.json"), JSON.stringify(report, null, 2));
console.log(
  `C++ ${total.cxx} lines, C ${total.c} lines compiled from the fork; C++ deps: ${report.cxx_deps.join(", ")}; C deps: ${report.c_deps.join(", ")}`,
);
for (const [dir, e] of Object.entries(report.by_dir).slice(0, 12))
  console.log(`  ${dir.padEnd(36)} c++ ${e.cxx}  c ${e.c}`);
