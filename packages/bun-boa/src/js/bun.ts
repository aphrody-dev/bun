// Installs `Bun` and `process` on top of the native operations that runtime.rs puts on `globalThis[OPS]`.
// It is a call, not a bare function expression: the parser drops expression statements without side effects.
const OPS = "__bun_native_ops__";
(function install(native: {
  printStdout(text: string): void;
  printStderr(text: string): void;
  exit(code: number): never;
  cwd(): string;
  nanoseconds(): number;
  semver: { satisfies(version: unknown, range: unknown): boolean; order(left: unknown, right: unknown): -1 | 0 | 1 };
  // bun_runtime_js::webcore::blob::exports
  Blob: typeof globalThis.Blob;
  File: typeof globalThis.File;
  FormData: typeof globalThis.FormData;
  file(path: unknown, options?: unknown): Blob;
  write(destination: unknown, data: unknown, options?: unknown): Promise<number>;
  stdin: Blob;
  stdout: Blob;
  stderr: Blob;
  createObjectURL(blob: unknown): string;
  revokeObjectURL(url: unknown): void;
  resolveObjectURL(url: unknown): Blob | undefined;
  serve(options: unknown): unknown;
  gc(): void;
  // bun_runtime_js::bun_object
  Glob: new (pattern: string) => unknown;
  deepEquals(a: unknown, b: unknown, strict?: boolean, skipPrototype?: boolean): boolean;
  env: Record<string, string>;
  argv: string[];
  main: string;
  version: string;
  revision: string;
  platform: string;
  arch: string;
  pid: number;
}) {
  const Bun = {
    version: native.version,
    revision: native.revision,
    env: native.env,
    argv: native.argv,
    main: native.main,
    file: native.file,
    write: native.write,
    stdin: native.stdin,
    stdout: native.stdout,
    stderr: native.stderr,
    sleep: (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms)),
    nanoseconds: native.nanoseconds,
    cwd: native.cwd,
    semver: native.semver,
    Glob: native.Glob,
    deepEquals: native.deepEquals,
    fileURLToPath(url: unknown): string {
      const parsed = url instanceof URL ? url : new URL(String(url));
      if (parsed.protocol !== "file:") throw new TypeError("The URL must be of scheme file");
      return decodeURIComponent(parsed.pathname);
    },
    pathToFileURL(path: unknown): URL {
      return new URL(`file://${encodeURI(String(path)).replace(/[?#]/g, encodeURIComponent)}`);
    },
    serve: native.serve,
    fetch: globalThis.fetch,
    gc: (_force?: boolean) => native.gc(),
  };

  const process: any = {
    argv: native.argv,
    env: native.env,
    exitCode: undefined as number | undefined,
    platform: native.platform,
    arch: native.arch,
    pid: native.pid,
    version: "v24.3.0",
    versions: { bun: native.version },
    release: { name: "node" },
    cwd: native.cwd,
    exit(code?: number): never {
      return native.exit(code ?? process.exitCode ?? 0);
    },
    nextTick(fn: (...args: unknown[]) => void, ...args: unknown[]) {
      queueMicrotask(() => fn(...args));
    },
    hrtime: Object.assign(
      (prev?: [number, number]) => {
        const ns = native.nanoseconds();
        const now: [number, number] = [Math.floor(ns / 1e9), ns % 1e9];
        return prev ? [now[0] - prev[0], now[1] - prev[1]] : now;
      },
      { bigint: () => BigInt(Math.floor(native.nanoseconds())) },
    ),
    stdout: { write: (s: unknown) => (native.printStdout(String(s)), true) },
    stderr: { write: (s: unknown) => (native.printStderr(String(s)), true) },
  };

  Object.defineProperties(globalThis, {
    Bun: { value: Bun, writable: true, configurable: true },
    process: { value: process, writable: true, configurable: true },
    global: { value: globalThis, writable: true, configurable: true },
    Blob: { value: native.Blob, writable: true, configurable: true },
    File: { value: native.File, writable: true, configurable: true },
    FormData: { value: native.FormData, writable: true, configurable: true },
  });
  if (typeof URL === "function")
    Object.defineProperties(URL, {
      createObjectURL: { value: native.createObjectURL, writable: true, enumerable: true, configurable: true },
      revokeObjectURL: { value: native.revokeObjectURL, writable: true, enumerable: true, configurable: true },
    });
  // Boa's runtime has timers but no immediates: one is a timer that is due at once.
  if (typeof (globalThis as any).setImmediate !== "function")
    Object.defineProperties(globalThis, {
      setImmediate: {
        value: function setImmediate(fn: (...args: unknown[]) => void, ...args: unknown[]) {
          return setTimeout(() => fn(...args), 0);
        },
        writable: true,
        configurable: true,
      },
      clearImmediate: {
        value: function clearImmediate(id: ReturnType<typeof setTimeout>) {
          clearTimeout(id);
        },
        writable: true,
        configurable: true,
      },
    });
})((globalThis as any)[OPS]);
delete (globalThis as any)[OPS];
