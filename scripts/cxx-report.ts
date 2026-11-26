// The C++ (and C) budget of the fork: lines of C/C++ under src/ per directory, and the native dependencies of
// scripts/build/deps/ with the language they bring. Written to build/cxx-report.json; the goal is C++ = 0.
//   bun run cxx:report
import { Glob } from "bun";
import { join, relative } from "node:path";

const root = join(import.meta.dir, "..");
const CXX = /\.(cpp|cc|cxx|hpp|hh|mm)$/;
const C = /\.(c|h)$/;
// Native dependencies (scripts/build/deps/*.ts) and the language they compile into bun.
const DEPS: Record<string, "c++" | "c" | "rust"> = {
  webkit: "c++",
  highway: "c++",
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

const lines = async (path: string) => (await Bun.file(path).text()).split("\n").length;
const src: Record<string, { cxx: number; c: number; files: number }> = {};
for await (const path of new Glob("src/**/*.{cpp,cc,cxx,hpp,hh,mm,c,h}").scan({ cwd: root })) {
  if (path.includes("/benches/") || path.includes("/tests/")) continue;
  const dir = path.split("/").slice(0, 3).join("/");
  const entry = (src[dir] ??= { cxx: 0, c: 0, files: 0 });
  const n = await lines(join(root, path));
  // A .h is C++ when it sits with C++ sources (jsc/bindings), C otherwise.
  if (CXX.test(path) || (path.endsWith(".h") && dir.startsWith("src/jsc"))) entry.cxx += n;
  else if (C.test(path)) entry.c += n;
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
  by_dir: Object.fromEntries(Object.entries(src).sort((a, b) => b[1].cxx + b[1].c - (a[1].cxx + a[1].c))),
};
await Bun.write(join(root, "build/cxx-report.json"), JSON.stringify(report, null, 2));
console.log(`C++ ${total.cxx} lines, C ${total.c} lines in src/; C++ deps: ${report.cxx_deps.join(", ")}; C deps: ${report.c_deps.join(", ")}`);
for (const [dir, e] of Object.entries(report.by_dir).slice(0, 12)) console.log(`  ${relative(".", dir).padEnd(28)} c++ ${e.cxx}  c ${e.c}`);
