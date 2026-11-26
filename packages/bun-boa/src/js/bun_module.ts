// `import … from "bun"`: the global `Bun` that js/bun.ts installs.
const B = (globalThis as any).Bun;
export default B;
export const { version, revision, env, argv, main, file, write, sleep, nanoseconds, cwd, semver } = B;
