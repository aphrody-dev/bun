// Tests across the whole cluster: n1 builds one nextest archive, then every host runs its own partition of it.
//   bun run test:cluster [--features build]
import { $ } from "bun";

const hosts = ["n1", "n2", "n3"];
const features = Bun.argv.slice(2);
const repo = "bun-nojsc-test";
const archive = `/srv/xbuild/archives/${repo}.tar.zst`;
const remote = (host: string, ...cmd: string[]) =>
  $`bun ${import.meta.dir}/remote.ts ${cmd}`.env({ ...Bun.env, BUN_BUILD_HOST: host, BUN_REMOTE_REPO: repo, GIT_NO_LAZY_FETCH: "1", RUSTC_WRAPPER: "" });

await $`ssh -o BatchMode=yes n1 mkdir -p /srv/xbuild/archives`;
await remote("n1", "cargo", "nextest", "archive", ...features, "--archive-file", archive);
// Every other host gets the archive straight from n1, and a source tree for the fixtures.
await Promise.all(
  hosts.slice(1).map(async host => {
    await $`ssh -o BatchMode=yes ${host} mkdir -p /srv/xbuild/archives`;
    await $`ssh -o BatchMode=yes n1 rsync -az ${archive} ${host}:${archive}`;
  }),
);
const results = await Promise.all(
  hosts.map((host, i) =>
    remote(host, "cargo", "nextest", "run", "--archive-file", archive, "--workspace-remap", ".", "--extract-to", `/tmp/${repo}-archive`, "--partition", `hash:${i + 1}/${hosts.length}`, "--no-tests=pass")
      .nothrow()
      .then(r => ({ host, code: r.exitCode })),
  ),
);
console.log(results.map(r => `${r.host}:${r.code}`).join(" "));
process.exit(results.some(r => r.code) ? 1 : 0);
