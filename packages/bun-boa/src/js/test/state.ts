// What the runner and expect() share: the running test, the file, and the natives test.rs gives bun:test.
const NATIVE = "__bun_test_native__";

export interface Native {
  /** printDiff.rs (src/runtime/test_runner/diff), compiled into bun_runtime_js. */
  diff(not: boolean, received: string, expected: string, colors: boolean): string;
  /** 0 pending, 1 fulfilled, 2 rejected. */
  promiseState(promise: Promise<unknown>): number;
  promiseResult(promise: Promise<unknown>): unknown;
  readFile(path: string): string | undefined;
  writeFile(path: string, text: string): boolean;
  /** Bun's `enable_ansi_colors_stderr`. */
  colors: boolean;
  isAgent: boolean;
  isCI: boolean;
  /** The flags of `bun test`. */
  options: {
    timeout: number;
    updateSnapshots: boolean;
    onlyFailures: boolean;
    todo: boolean;
    namePattern?: string;
    randomize: boolean;
    retry: number;
    concurrent: boolean;
    maxConcurrency: number;
  };
  /** Set by js/test.ts for test.rs: runs the file's tests once it is evaluated. */
  run?: (path: string) => Promise<object>;
  /** Set by js/test.ts for test.rs: prints an error the file threw while it loaded. */
  unhandled?: (error: unknown) => void;
}

const g = globalThis as any;
export const native: Native = g[NATIVE];
delete g[NATIVE];

export type ExpectAssertions = { kind: "none" } | { kind: "exact"; count: number } | { kind: "atLeastOne" };

/** The test that runs: what Bun's ExecutionSequence counts for it. */
export interface Sequence {
  expectCalls: number;
  assertions: ExpectAssertions;
  /** The test's full name, the way snapshots and getState() name it. */
  testName: string;
  /** describe names, outermost first, and the test's. */
  names: string[];
  vitest: boolean;
  concurrent: boolean;
  /** Errors of expect.soft(), which fail the test when its callback is done. */
  softErrors: unknown[];
}

export const runtime = {
  sequence: undefined as Sequence | undefined,
  /** The absolute path of the test file. */
  path: "",
  phase: "collection" as "collection" | "execution" | "done",
  /** expect() calls outside of a sequence (concurrent tests, between tests). */
  strayExpects: 0,
};
