// Runs a command on the build host (never locally). The host keeps its own partial clone of oven-sh/bun at this
// checkout's HEAD (fetched from GitHub, not uploaded); only the files that differ from HEAD go over the mesh.
// Then setup.ts prepares the tree and the command runs in packages/bun-boa with every cache on XFS.
//
//   bun run remote cargo build --bin bun        BUN_BUILD_HOST=n1 (default)
import { $ } from "bun";
import { join } from "node:path";

const host = process.env.BUN_BUILD_HOST ?? "n1";
const xfs = "/srv/xbuild";
// one remote tree (and target dir) per concurrent user: BUN_REMOTE_REPO=bun-nojsc-<name>
const remoteRepo = `${xfs}/src/${Bun.env.BUN_REMOTE_REPO ?? "bun-nojsc"}`;
const repo = join(import.meta.dir, "../../..");
const args = Bun.argv.slice(2);
if (!args.length) throw new Error("usage: bun run remote <command…>");

const quote = (s: string) => `'${s.replaceAll("'", `'\\''`)}'`;
const ssh = (script: string) => $`ssh -o BatchMode=yes ${host} ${script}`;

// The upstream commit this branch starts from: the host's clone has it, the branch's own commits it may not.
const head = (await $`git -C ${repo} merge-base HEAD ${process.env.BUN_BASE_BRANCH ?? "main"}`.text()).trim();
const changed = [
  ...new Set([
    ...(await $`git -C ${repo} diff --name-only --diff-filter=d ${head}`.text()).split("\n"),
    ...(await $`git -C ${repo} ls-files -o --exclude-standard`.text()).split("\n"),
  ]),
].filter(Boolean);
// A worktree of the host's existing Bun clone: the base costs no download beyond the blobs that changed.
const mirror = `${xfs}/src/bun`;
await ssh(
  `set -e; git -C ${mirror} cat-file -e ${head}^{commit} 2>/dev/null || git -C ${mirror} fetch -q upstream ${head}; ` +
    `if [ -e ${remoteRepo}/.git ]; then git -C ${remoteRepo} checkout -q --detach -f ${head}; ` +
    `else git -C ${mirror} worktree add -q --detach -f ${remoteRepo} ${head}; fi`,
);
const list = join(repo, "build/remote-files.txt");
await Bun.write(list, changed.join("\n") + "\n");
await $`rsync -az --files-from=${list} ${`${repo}/`} ${`${host}:${remoteRepo}/`}`;

const env = {
  CARGO_HOME: `${xfs}/cargo-home`,
  CARGO_TARGET_DIR: `${xfs}/target/${Bun.env.BUN_REMOTE_REPO ? `bun-boa-${Bun.env.BUN_REMOTE_REPO}` : "bun-boa"}`,
  BUN_CODEGEN_DIR: `${remoteRepo}/build/debug/codegen`,
  RUSTC_WRAPPER: "sccache",
  SCCACHE_CONF: `${xfs}/sccache-dist-client.toml`,
};
const exports = Object.entries(env).map(([k, v]) => `${k}=${quote(v)}`).join(" ");
const script = `cd ${remoteRepo}/packages/bun-boa && bun scripts/setup.ts && env ${exports} ${args.map(quote).join(" ")}`;
const proc = Bun.spawn(["ssh", "-o", "BatchMode=yes", host, script], { stdio: ["inherit", "inherit", "inherit"] });
process.exit(await proc.exited);
