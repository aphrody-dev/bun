// Runs a command on the build host (never locally). The host keeps its own partial clone of oven-sh/bun at this
// checkout's HEAD (fetched from GitHub, not uploaded); only the files that differ from HEAD go over the mesh.
// Then setup.ts prepares the tree and the command runs in packages/bun-boa with every cache on XFS.
//
//   bun run remote cargo build --bin bun        BUN_BUILD_HOST=n1 (default)
import { $ } from "bun";
import { join } from "node:path";

const host = process.env.BUN_BUILD_HOST ?? "n1";
const xfs = "/srv/xbuild";
const remoteRepo = `${xfs}/src/bun-nojsc`;
const repo = join(import.meta.dir, "../../..");
const args = Bun.argv.slice(2);
if (!args.length) throw new Error("usage: bun run remote <command…>");

const quote = (s: string) => `'${s.replaceAll("'", `'\\''`)}'`;
const ssh = (script: string) => $`ssh -o BatchMode=yes ${host} ${script}`;

const head = (await $`git -C ${repo} rev-parse HEAD`.text()).trim();
const changed = (await $`git -C ${repo} ls-files -m -o --exclude-standard`.text()).split("\n").filter(Boolean);
await ssh(
  `set -e; mkdir -p ${remoteRepo}; cd ${remoteRepo}; [ -d .git ] || git init -q; ` +
    `git cat-file -e ${head}^{commit} 2>/dev/null || git fetch -q --filter=blob:none --depth=1 https://github.com/oven-sh/bun.git ${head}; ` +
    `git reset -q --hard ${head}`,
);
const list = join(repo, "build/remote-files.txt");
await Bun.write(list, changed.join("\n") + "\n");
await $`rsync -az --files-from=${list} ${`${repo}/`} ${`${host}:${remoteRepo}/`}`;

const env = {
  CARGO_HOME: `${xfs}/cargo-home`,
  CARGO_TARGET_DIR: `${xfs}/target/bun-boa`,
  BUN_CODEGEN_DIR: `${remoteRepo}/build/debug/codegen`,
  RUSTC_WRAPPER: "sccache",
  SCCACHE_CONF: `${xfs}/sccache-dist-client.toml`,
};
const exports = Object.entries(env).map(([k, v]) => `${k}=${quote(v)}`).join(" ");
const script = `cd ${remoteRepo}/packages/bun-boa && bun scripts/setup.ts && env ${exports} ${args.map(quote).join(" ")}`;
const proc = Bun.spawn(["ssh", "-o", "BatchMode=yes", host, script], { stdio: ["inherit", "inherit", "inherit"] });
process.exit(await proc.exited);
