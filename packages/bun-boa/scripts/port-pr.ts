// Ports an upstream pull request into the patch series: one patch per crate the PR touches, named after the PR.
//
//   bun scripts/port-pr.ts apply boa-dev/boa#5683     PR diff → vendor/<crate>-<version>/ working trees
//                                                     (rejected hunks land as *.rej next to the file)
//   bun scripts/port-pr.ts commit boa-dev/boa#5683 [note…]
//                                                     working trees → commit (PR author, PR title) →
//                                                     patches/<crate>-<version>/NNNN-<title>.patch
//
// Files outside a vendored crate (workspace Cargo.lock, repo-level test suites) are left out and listed.
import { $ } from "bun";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { env, materialise } from "./vendor.ts";

const root = join(import.meta.dir, "..");

// Upstream directory → crates.io crate name.
const layout: Record<string, Record<string, string>> = {
  "boa-dev/boa": {
    "core/ast/": "boa_ast",
    "core/engine/": "boa_engine",
    "core/gc/": "boa_gc",
    "core/interner/": "boa_interner",
    "core/parser/": "boa_parser",
    "core/runtime/": "boa_runtime",
    "core/string/": "boa_string",
    "core/wintertc/": "boa_wintertc",
  },
  "DioxusLabs/blitz": {
    "packages/blitz-dom/": "blitz-dom",
    "packages/blitz-html/": "blitz-html",
    "packages/blitz-paint/": "blitz-paint",
    "packages/blitz-traits/": "blitz-traits",
    "packages/stylo_taffy/": "stylo_taffy",
  },
};

// Every crate of one upstream repo is released at one version: a crate not vendored yet takes its siblings'.
function vendorName(repo: string, crate: string): string {
  const series = [...new Bun.Glob("*").scanSync({ cwd: join(root, "patches"), onlyFiles: false })];
  const own = series.find(n => n.startsWith(`${crate}-`) && /^\d/.test(n.slice(crate.length + 1)));
  if (own) return own;
  for (const sibling of Object.values(layout[repo]!)) {
    const n = series.find(n => n.startsWith(`${sibling}-`) && /^\d/.test(n.slice(sibling.length + 1)));
    if (n) return `${crate}-${n.slice(sibling.length + 1)}`;
  }
  throw new Error(`${crate}: no vendored sibling to take a version from`);
}

type Split = { crates: Map<string, string>; outside: string[] };

function split(repo: string, diff: string): Split {
  const crates = new Map<string, string>();
  const outside: string[] = [];
  for (const block of diff.split(/^(?=diff --git )/m).filter(Boolean)) {
    const path = block.slice("diff --git a/".length, block.indexOf(" b/"));
    const prefix = Object.keys(layout[repo]!).find(p => path.startsWith(p));
    if (!prefix) {
      outside.push(path);
      continue;
    }
    const name = vendorName(repo, layout[repo]![prefix]!);
    const hunks = block.indexOf("\n@@");
    const head = (hunks < 0 ? block : block.slice(0, hunks))
      .split("\n")
      .map(line =>
        line
          .replace(` a/${prefix}`, " a/")
          .replace(` b/${prefix}`, " b/")
          .replace(`--- a/${prefix}`, "--- a/")
          .replace(`+++ b/${prefix}`, "+++ b/")
          .replace(`rename from ${prefix}`, "rename from ")
          .replace(`rename to ${prefix}`, "rename to "),
      )
      .join("\n");
    crates.set(name, (crates.get(name) ?? "") + head + (hunks < 0 ? "" : block.slice(hunks)));
  }
  return { crates, outside };
}

const [command, ref, ...note] = Bun.argv.slice(2);
const [, repo, number] = /^([\w.-]+\/[\w.-]+)#(\d+)$/.exec(ref ?? "") ?? [];
if (!command || !repo || !number || !layout[repo]) throw new Error("usage: port-pr.ts apply|commit <owner/repo>#<n>");

const pr = JSON.parse(await $`gh pr view ${number} -R ${repo} --json title,url,commits`.quiet().text()) as {
  title: string;
  url: string;
  commits: { authors: { name: string; email: string }[]; committedDate: string }[];
};
const { crates, outside } = split(repo, await $`gh pr diff ${number} -R ${repo}`.quiet().text());

if (command === "apply" || command === "check") {
  for (const [name, patch] of crates) {
    const dir = join(root, "vendor", name);
    if (!(await Bun.file(join(dir, ".patch-sha8")).exists())) {
      await mkdir(join(root, "patches", name), { recursive: true });
      console.log(await materialise(name));
    }
    const file = join(root, "build", `${repo.replace("/", "-")}-${number}-${name}.diff`);
    await Bun.write(file, patch);
    const clean = await $`git apply --check ${file}`.cwd(dir).nothrow().quiet();
    const res =
      command === "check"
        ? clean
        : clean.exitCode === 0
          ? await $`git apply ${file}`.cwd(dir).nothrow()
          : await $`git apply --reject ${file}`.cwd(dir).nothrow().quiet();
    console.log(`${name}: ${clean.exitCode === 0 ? "applied" : `REJECTS\n${res.stderr.toString().trim()}`}`);
  }
  if (outside.length) console.log(`outside the vendored crates: ${outside.join(", ")}`);
} else if (command === "commit") {
  const author = pr.commits[0]!.authors[0]!;
  const date = pr.commits.at(-1)!.committedDate;
  const left = outside.length ? [`Left out, outside the crate: ${outside.join(", ")}.`] : [];
  const body = [`Port of ${pr.url}.`, ...note, ...left].join("\n\n");
  const names = crates.size ? [...crates.keys()] : [];
  for (const name of names) {
    const dir = join(root, "vendor", name);
    await $`git add -A -- . ':!.patch-sha8' ':!*.rej' ':!*.orig'`.cwd(dir);
    if ((await $`git diff --cached --quiet`.cwd(dir).nothrow()).exitCode === 0) {
      console.log(`${name}: nothing staged, skipped`);
      continue;
    }
    await $`git commit -q --author ${`${author.name} <${author.email}>`} --date ${date} -m ${pr.title} -m ${body}`
      .cwd(dir)
      .env({
        ...env,
        GIT_AUTHOR_NAME: author.name,
        GIT_AUTHOR_EMAIL: author.email,
        GIT_AUTHOR_DATE: date,
        GIT_COMMITTER_DATE: date,
      });
    const out = join(root, "patches", name);
    const next = [...new Bun.Glob("*.patch").scanSync(out)].length + 1;
    const file = (
      await $`git format-patch -q -1 HEAD --zero-commit --no-signature --start-number ${next} -o ${out}`.cwd(dir).text()
    ).trim();
    console.log(`${name}: ${file || `patches/${name}/${String(next).padStart(4, "0")}-*.patch`}`);
  }
} else {
  throw new Error(`unknown command ${command}`);
}
