// Tests across the whole cluster: n1 builds one nextest archive, then every host runs its own partition of it.
// Only n1 holds a Bun clone; the others get the archive (from n1) and this package's tests/ (from here), which
// --workspace-remap stands in for the package root (CARGO_MANIFEST_DIR).
//   bun run test:cluster [--features build]
import { $ } from "bun";
import { join } from "node:path";

const hosts = ["n1", "n2", "n3"];
const features = Bun.argv.slice(2);
const repo = "bun-nojsc-test";
const dir = `/srv/xbuild/scratch/${repo}`;
const archive = `${dir}/archive.tar.zst`;
const workspace = `${dir}/workspace`;
const ssh = (host: string, ...cmd: string[]) => $`ssh -o BatchMode=yes ${host} ${cmd}`;

// nextest only checks that the remapped workspace has a Cargo.toml; an empty one does.
await Promise.all(
  hosts.map(host =>
    ssh(host, "mkdir", "-p", `${workspace}/tests`, `${dir}/extract`, "&&", "touch", `${workspace}/Cargo.toml`),
  ),
);
await $`bun ${import.meta.dir}/remote.ts cargo nextest archive ${features} --archive-file ${archive}`.env({
  ...Bun.env,
  BUN_BUILD_HOST: "n1",
  BUN_REMOTE_REPO: repo,
  GIT_NO_LAZY_FETCH: "1",
});
// n1 knows the others under other aliases, so it is pointed at the address this machine resolves them to.
const address = async (host: string) => {
  const config = Object.fromEntries((await $`ssh -G ${host}`.text()).split("\n").map(line => line.split(" ", 2)));
  return `${config.user}@${config.hostname}`;
};
await Promise.all(
  hosts.map(async (host, i) => {
    await $`rsync -az --delete ${join(import.meta.dir, "../tests")}/ ${host}:${workspace}/tests/`;
    if (i)
      await ssh(hosts[0], "rsync", "-az", "-e", "'ssh -o BatchMode=yes'", archive, `${await address(host)}:${archive}`);
  }),
);
const results = await Promise.all(
  hosts.map((host, i) =>
    ssh(
      host,
      "cargo-nextest",
      "nextest",
      "run",
      "--archive-file",
      archive,
      "--workspace-remap",
      workspace,
      "--extract-to",
      `${dir}/extract`,
      "--extract-overwrite",
      "--partition",
      `hash:${i + 1}/${hosts.length}`,
      "--no-tests=pass",
    )
      .nothrow()
      .then(r => ({ host, code: r.exitCode })),
  ),
);
console.log(results.map(r => `${r.host}:${r.code}`).join(" "));
process.exit(results.some(r => r.code) ? 1 : 0);
