// Materialises each patches/<crate>-<version>/ series into vendor/<crate>-<version>/, every crate concurrently.
// The crates.io tarball becomes a git repo tagged `vendor-base` with the series applied on top by `git am`
// (authors and messages kept); a `.patch-sha8` marker skips a series that did not change.
// To change a crate: commit in vendor/<crate>-<version>/, then `git format-patch vendor-base -o ../../patches/<crate>-<version>/`.
import { $ } from "bun";
import { join } from "node:path";

const root = join(import.meta.dir, "..");
const env = {
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

async function materialise(name: string): Promise<string> {
  const [, crate, version] = /^(.+)-(\d+\.\d+\.\d+.*)$/.exec(name) ?? [];
  if (!crate || !version) throw new Error(`patches/${name}: expected <crate>-<version>`);
  const series = [...new Bun.Glob("*.patch").scanSync(join(root, "patches", name))].sort();
  const hasher = new Bun.CryptoHasher("sha256");
  for (const patch of series) hasher.update(await Bun.file(join(root, "patches", name, patch)).bytes());
  const sha8 = hasher.digest("hex").slice(0, 8);
  const dir = join(root, "vendor", name);
  const marker = Bun.file(join(dir, ".patch-sha8"));
  if ((await marker.exists()) && (await marker.text()).trim() === sha8) return `${name}: up to date`;

  const res = await fetch(`https://static.crates.io/crates/${crate}/${crate}-${version}.crate`);
  if (!res.ok) throw new Error(`${name}: crates.io answered ${res.status}`);
  const tarball = `${dir}.crate`;
  await $`rm -rf ${dir} && mkdir -p ${dir}`;
  await Bun.write(tarball, res);
  await $`tar -xzf ${tarball} -C ${dir} --strip-components=1 && rm ${tarball}`;
  await $`git init -q && git add -A && git commit -q -m ${`${crate} ${version} from crates.io`} && git tag vendor-base`.cwd(dir).env(env);
  if (series.length) await $`git am -q --committer-date-is-author-date ${series.map(p => join(root, "patches", name, p))}`.cwd(dir).env(env);
  await Bun.write(marker, `${sha8}\n`);
  return `${name}: ${series.length} patches`;
}

const names = [...new Bun.Glob("*/*.patch").scanSync(join(root, "patches"))].map(p => p.split("/")[0]!).filter((n, i, all) => all.indexOf(n) === i);
const results = await Promise.allSettled(names.map(materialise));
for (const r of results) r.status === "fulfilled" ? console.log(r.value) : console.error(String(r.reason));
process.exit(results.some(r => r.status === "rejected") ? 1 : 0);
