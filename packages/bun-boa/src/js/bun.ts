// Installs `Bun` and `process` on top of the native operations from runtime.rs. Evaluates to the installer.
(function install(native: {
  readText(path: string): string;
  readBytes(path: string): Uint8Array;
  exists(path: string): boolean;
  size(path: string): number;
  write(path: string, data: string | Uint8Array): number;
  stdout(text: string): void;
  stderr(text: string): void;
  exit(code: number): never;
  cwd(): string;
  nanoseconds(): number;
  env: Record<string, string>;
  argv: string[];
  main: string;
  version: string;
  revision: string;
  platform: string;
  arch: string;
  pid: number;
}) {
  const MIME: Record<string, string> = {
    json: "application/json;charset=utf-8",
    js: "text/javascript;charset=utf-8",
    mjs: "text/javascript;charset=utf-8",
    ts: "text/javascript;charset=utf-8",
    txt: "text/plain;charset=utf-8",
    html: "text/html;charset=utf-8",
    css: "text/css;charset=utf-8",
  };

  class BunFile {
    readonly name: string;
    constructor(path: string) {
      this.name = path;
    }
    get size(): number {
      return native.size(this.name);
    }
    get type(): string {
      return MIME[this.name.split(".").pop() ?? ""] ?? "application/octet-stream";
    }
    async text(): Promise<string> {
      return native.readText(this.name);
    }
    async json(): Promise<any> {
      return JSON.parse(native.readText(this.name));
    }
    async bytes(): Promise<Uint8Array> {
      return native.readBytes(this.name);
    }
    async arrayBuffer(): Promise<ArrayBuffer> {
      return native.readBytes(this.name).buffer as ArrayBuffer;
    }
    async exists(): Promise<boolean> {
      return native.exists(this.name);
    }
    get [Symbol.toStringTag]() {
      return "BunFile";
    }
  }

  const path = (dest: unknown): string => (dest instanceof BunFile ? dest.name : dest instanceof URL ? dest.pathname : String(dest));

  async function write(dest: unknown, data: unknown): Promise<number> {
    if (data instanceof BunFile) data = await data.bytes();
    if (data instanceof ArrayBuffer) data = new Uint8Array(data);
    else if (ArrayBuffer.isView(data) && !(data instanceof Uint8Array))
      data = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    return native.write(path(dest), data instanceof Uint8Array ? data : String(data));
  }

  const Bun = {
    version: native.version,
    revision: native.revision,
    env: native.env,
    argv: native.argv,
    main: native.main,
    file: (p: unknown) => new BunFile(path(p)),
    write,
    sleep: (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms)),
    nanoseconds: native.nanoseconds,
    cwd: native.cwd,
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
    hrtime: Object.assign((prev?: [number, number]) => {
      const ns = native.nanoseconds();
      const now: [number, number] = [Math.floor(ns / 1e9), ns % 1e9];
      return prev ? [now[0] - prev[0], now[1] - prev[1]] : now;
    }, { bigint: () => BigInt(Math.floor(native.nanoseconds())) }),
    stdout: { write: (s: unknown) => (native.stdout(String(s)), true) },
    stderr: { write: (s: unknown) => (native.stderr(String(s)), true) },
  };

  Object.defineProperties(globalThis, {
    Bun: { value: Bun, writable: true, configurable: true },
    process: { value: process, writable: true, configurable: true },
    global: { value: globalThis, writable: true, configurable: true },
  });
});
