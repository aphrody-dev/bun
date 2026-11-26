// Materialises each patches/<crate>-<version>/ series into vendor/<crate>-<version>/, every crate concurrently.
// The crates.io tarball becomes a git repo tagged `vendor-base` with the series applied on top by `git am`
// (authors and messages kept); a `.patch-sha8` marker skips a series that did not change.
// A crate that is not on crates.io names its source in patches/<crate>-<version>/SOURCE as `<git-url>@<ref>:<path>`;
// that directory of the repo at that ref becomes the base instead (forks stay out of the dependency graph).
// To change a crate: commit in vendor/<crate>-<version>/, then `git format-patch vendor-base -o ../../patches/<crate>-<version>/`.
import { $ } from "bun";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = join(import.meta.dir, "..");
export const env = {
  ...process.env,
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_CONFIG_SYSTEM: "/dev/null",
  GIT_AUTHOR_NAME: "vendor",
  GIT_AUTHOR_EMAIL: "vendor@localhost",
  GIT_COMMITTER_NAME: "vendor",
  GIT_COMMITTER_EMAIL: "vendor@localhost",
  GIT_AUTHOR_DATE: "2000-01-01T00:00:00Z",
  GIT_COMMITTER_DATE: "2000-01-01T00:00:00Z",
};

async function fromCratesIo(crate: string, version: string, dir: string): Promise<string> {
  const res = await fetch(`https://static.crates.io/crates/${crate}/${crate}-${version}.crate`);
  if (!res.ok) throw new Error(`${crate}-${version}: crates.io answered ${res.status}`);
  const tarball = `${dir}.crate`;
  await Bun.write(tarball, res);
  await $`tar -xzf ${tarball} -C ${dir} --strip-components=1 && rm ${tarball}`;
  return `${crate} ${version} from crates.io`;
}

async function fromGit(source: string, dir: string): Promise<string> {
  const [, url, ref, path] = /^(\S+)@([^@:\s]+):(\S+)$/.exec(source) ?? [];
  if (!url || !ref || !path) throw new Error(`SOURCE: expected <git-url>@<ref>:<path>, got ${source}`);
  const clone = await mkdtemp(join(tmpdir(), "vendor-git-"));
  try {
    await $`git clone -q --depth 1 --branch ${ref} --filter=blob:none --no-checkout ${url} ${clone}`.env(env);
    await $`git sparse-checkout set --no-cone ${`/${path}/`} && git checkout -q`.cwd(clone).env(env);
    await $`cp -a ${`${join(clone, path)}/.`} ${dir}`;
  } finally {
    await $`rm -rf ${clone}`;
  }
  return `${path} at ${url}@${ref}`;
}

export async function materialise(name: string): Promise<string> {
  const [, crate, version] = /^(.+)-(\d+\.\d+\.\d+.*)$/.exec(name) ?? [];
  if (!crate || !version) throw new Error(`patches/${name}: expected <crate>-<version>`);
  const series = [...new Bun.Glob("*.patch").scanSync(join(root, "patches", name))].sort();
  const sourceFile = Bun.file(join(root, "patches", name, "SOURCE"));
  const source = (await sourceFile.exists()) ? (await sourceFile.text()).trim() : undefined;
  const hasher = new Bun.CryptoHasher("sha256");
  hasher.update(source ?? "crates.io");
  for (const patch of series) hasher.update(await Bun.file(join(root, "patches", name, patch)).bytes());
  const sha8 = hasher.digest("hex").slice(0, 8);
  const dir = join(root, "vendor", name);
  const marker = Bun.file(join(dir, ".patch-sha8"));
  if ((await marker.exists()) && (await marker.text()).trim() === sha8) return `${name}: up to date`;

  await $`rm -rf ${dir} && mkdir -p ${dir}`;
  const message = source ? await fromGit(source, dir) : await fromCratesIo(crate, version, dir);
  await $`git init -q && git add -A && git commit -q -m ${message} && git tag vendor-base`.cwd(dir).env(env);
  if (series.length)
    await $`git am -q --committer-date-is-author-date ${series.map(p => join(root, "patches", name, p))}`
      .cwd(dir)
      .env(env);
  await Bun.write(marker, `${sha8}\n`);
  return `${name}: ${series.length} patches`;
}

// Cargo.toml's [patch.crates-io] table (the end of the file) lists every series, so a vendored crate is never left
// on its crates.io tarball while a patched dependent expects its changes.
async function patchTable(names: string[]) {
  const manifest = Bun.file(join(root, "Cargo.toml"));
  const text = await manifest.text();
  const header = "[patch.crates-io]\n";
  const at = text.indexOf(header);
  if (at < 0) throw new Error(`Cargo.toml: no ${header.trim()} table`);
  const rows = names
    .toSorted()
    .map(name => `${/^(.+)-\d+\.\d+\.\d+/.exec(name)![1]} = { path = "vendor/${name}" }\n`)
    .join("");
  const next = text.slice(0, at + header.length) + rows;
  if (next !== text) await Bun.write(manifest, next);
}

if (import.meta.main) {
  const names = [...new Bun.Glob("*/{*.patch,SOURCE}").scanSync(join(root, "patches"))]
    .map(p => p.split("/")[0]!)
    .filter((n, i, all) => all.indexOf(n) === i);
  const [results] = await Promise.all([Promise.allSettled(names.map(materialise)), patchTable(names)]);
  for (const r of results) {
    if (r.status === "fulfilled") console.log(r.value);
    else console.error(String(r.reason));
  }
  process.exit(results.some(r => r.status === "rejected") ? 1 : 0);
}
