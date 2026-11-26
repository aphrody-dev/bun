// `bun:test` on Boa: describe/test/it, hooks, expect with Bun's matchers, mock/spyOn. test.rs evaluates a test file
// (which registers its tests here), then calls the runner it finds at globalThis[RUN] and reads the counts it returns.
const RUN = Symbol.for("bun:test:run");

type Fn = (...args: any[]) => any;
type Test = { name: string; fn?: Fn; mode: "run" | "skip" | "todo" | "only"; timeout?: number; failing?: boolean };
type Suite = {
  name: string;
  parent?: Suite;
  mode: "run" | "skip" | "todo" | "only";
  children: (Test | Suite)[];
  beforeAll: Fn[];
  afterAll: Fn[];
  beforeEach: Fn[];
  afterEach: Fn[];
};

const out = (s: string) => process.stderr.write(s);
const now = () => Bun.nanoseconds() / 1e6;
const isSuite = (x: Test | Suite): x is Suite => "children" in x;
const newSuite = (name: string, parent: Suite | undefined, mode: Suite["mode"]): Suite => ({
  name,
  parent,
  mode,
  children: [],
  beforeAll: [],
  afterAll: [],
  beforeEach: [],
  afterEach: [],
});
const root = newSuite("", undefined, "run");
let current = root;
let hasOnly = false;
let defaultTimeout = 5000;
let expectCalls = 0;
let assertionsExpected: number | undefined;
let assertionsAny = false;

// ── formatting ────────────────────────────────────────────────────────────────
function fmt(value: unknown, depth = 0, seen = new Set<unknown>()): string {
  if (typeof value === "string") return depth ? JSON.stringify(value) : JSON.stringify(value);
  if (typeof value === "bigint") return `${value}n`;
  if (typeof value === "symbol") return value.toString();
  if (typeof value === "function") return `[Function${value.name ? `: ${value.name}` : ""}]`;
  if (value === null || typeof value !== "object") return Object.is(value, -0) ? "-0" : String(value);
  if (seen.has(value)) return "[Circular]";
  if (depth > 4) return Array.isArray(value) ? "[Array]" : "[Object]";
  seen.add(value);
  try {
    if (value instanceof Error) return `${value.name}: ${value.message}`;
    if (value instanceof Date) return value.toISOString();
    if (value instanceof RegExp) return String(value);
    if (Array.isArray(value)) return `[ ${value.map(v => fmt(v, depth + 1, seen)).join(", ")} ]`;
    if (value instanceof Map)
      return `Map(${value.size}) { ${[...value].map(([k, v]) => `${fmt(k, depth + 1, seen)}: ${fmt(v, depth + 1, seen)}`).join(", ")} }`;
    if (value instanceof Set)
      return `Set(${value.size}) { ${[...value].map(v => fmt(v, depth + 1, seen)).join(", ")} }`;
    if (ArrayBuffer.isView(value))
      return `${value.constructor.name}(${(value as any).length}) [ ${[...(value as any)].join(", ")} ]`;
    const ctor = Object.getPrototypeOf(value)?.constructor?.name;
    const body = Object.keys(value)
      .map(k => `${/^[A-Za-z_$][\w$]*$/.test(k) ? k : JSON.stringify(k)}: ${fmt((value as any)[k], depth + 1, seen)}`)
      .join(", ");
    return `${ctor && ctor !== "Object" ? `${ctor} ` : ""}{${body ? ` ${body} ` : ""}}`;
  } finally {
    seen.delete(value);
  }
}

// ── equality ──────────────────────────────────────────────────────────────────
const ASYMMETRIC = Symbol.for("bun:test:asymmetric");
type Asymmetric = { [ASYMMETRIC]: true; match(v: unknown): boolean; toString(): string };
const isAsymmetric = (v: unknown): v is Asymmetric => !!v && typeof v === "object" && (v as any)[ASYMMETRIC] === true;

function equals(a: unknown, b: unknown, strict: boolean, seen: [unknown, unknown][] = []): boolean {
  if (isAsymmetric(b)) return b.match(a);
  if (isAsymmetric(a)) return a.match(b);
  if (Object.is(a, b)) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (seen.some(([x, y]) => x === a && y === b)) return true;
  seen = [...seen, [a, b]];
  if (strict && Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
  if (a instanceof Date || b instanceof Date)
    return a instanceof Date && b instanceof Date && Object.is(a.getTime(), b.getTime());
  if (a instanceof RegExp || b instanceof RegExp) return String(a) === String(b);
  if (a instanceof Error && b instanceof Error && (a.message !== b.message || a.name !== b.name)) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (a instanceof Map || b instanceof Map) {
    if (!(a instanceof Map && b instanceof Map) || a.size !== b.size) return false;
    for (const [k, v] of a) if (!b.has(k) || !equals(v, b.get(k), strict, seen)) return false;
    return true;
  }
  if (a instanceof Set || b instanceof Set) {
    if (!(a instanceof Set && b instanceof Set) || a.size !== b.size) return false;
    outer: for (const v of a) {
      if (b.has(v)) continue;
      for (const w of b) if (equals(v, w, strict, seen)) continue outer;
      return false;
    }
    return true;
  }
  if (ArrayBuffer.isView(a) || ArrayBuffer.isView(b)) {
    if (!(ArrayBuffer.isView(a) && ArrayBuffer.isView(b)) || a.constructor !== b.constructor) return false;
    const x = a as any,
      y = b as any;
    if (x.length !== y.length) return false;
    for (let i = 0; i < x.length; i++) if (!Object.is(x[i], y[i])) return false;
    return true;
  }
  const keys = (o: object) =>
    [
      ...Object.keys(o),
      ...Object.getOwnPropertySymbols(o).filter(s => Object.prototype.propertyIsEnumerable.call(o, s)),
    ].filter(k => strict || (o as any)[k] !== undefined);
  const ka = keys(a),
    kb = keys(b);
  if (Array.isArray(a) && (a as unknown[]).length !== (b as unknown[]).length) return false;
  if (ka.length !== kb.length) return false;
  for (const k of ka) {
    if (strict && !Object.prototype.hasOwnProperty.call(b, k)) return false;
    if (!equals((a as any)[k], (b as any)[k], strict, seen)) return false;
  }
  if (strict && Array.isArray(a)) for (let i = 0; i < a.length; i++) if (i in a !== i in (b as unknown[])) return false;
  return true;
}

function matchObject(actual: unknown, expected: unknown): boolean {
  if (isAsymmetric(expected)) return expected.match(actual);
  if (typeof expected !== "object" || expected === null || typeof actual !== "object" || actual === null)
    return equals(actual, expected, false);
  if (Array.isArray(expected))
    return (
      Array.isArray(actual) && actual.length === expected.length && expected.every((e, i) => matchObject(actual[i], e))
    );
  return Object.keys(expected).every(
    k => k in (actual as object) && matchObject((actual as any)[k], (expected as any)[k]),
  );
}

// ── expect ────────────────────────────────────────────────────────────────────
class AssertionError extends Error {
  name = "AssertionError";
}

function asymmetric(name: string, match: (v: unknown) => boolean): Asymmetric {
  return { [ASYMMETRIC]: true, match, toString: () => name } as Asymmetric;
}

const customMatchers: Record<string, Fn> = {};
const isMock = (f: unknown): f is MockFn => typeof f === "function" && Array.isArray((f as any).mock?.calls);

function expect(actual: unknown, message?: string) {
  expectCalls++;
  const make = (negate: boolean, promise?: "resolves" | "rejects") => {
    const check = (pass: boolean, what: string, expected?: unknown, showExpected = true) => {
      if (pass === negate) {
        const lines = [
          message ?? `expect(received).${negate ? "not." : ""}${what}`,
          "",
          ...(showExpected ? [`Expected${negate ? " not" : ""}: ${fmt(expected)}`] : []),
          `Received: ${fmt(actual)}`,
        ];
        throw new AssertionError(lines.join("\n"));
      }
    };
    const matchers: Record<string, Fn> = {
      toBe: (e: unknown) => check(Object.is(actual, e), "toBe(expected)", e),
      toEqual: (e: unknown) => check(equals(actual, e, false), "toEqual(expected)", e),
      toStrictEqual: (e: unknown) => check(equals(actual, e, true), "toStrictEqual(expected)", e),
      toMatchObject: (e: unknown) => check(matchObject(actual, e), "toMatchObject(expected)", e),
      toBeTruthy: () => check(!!actual, "toBeTruthy()", undefined, false),
      toBeFalsy: () => check(!actual, "toBeFalsy()", undefined, false),
      toBeNull: () => check(actual === null, "toBeNull()", null),
      toBeUndefined: () => check(actual === undefined, "toBeUndefined()", undefined),
      toBeDefined: () => check(actual !== undefined, "toBeDefined()", undefined, false),
      toBeNil: () => check(actual == null, "toBeNil()", undefined, false),
      toBeNaN: () => check(Number.isNaN(actual), "toBeNaN()", NaN),
      toBeTrue: () => check(actual === true, "toBeTrue()", true),
      toBeFalse: () => check(actual === false, "toBeFalse()", false),
      toBeString: () => check(typeof actual === "string", "toBeString()", undefined, false),
      toBeNumber: () => check(typeof actual === "number", "toBeNumber()", undefined, false),
      toBeBoolean: () => check(typeof actual === "boolean", "toBeBoolean()", undefined, false),
      toBeFunction: () => check(typeof actual === "function", "toBeFunction()", undefined, false),
      toBeObject: () => check(typeof actual === "object" && actual !== null, "toBeObject()", undefined, false),
      toBeArray: () => check(Array.isArray(actual), "toBeArray()", undefined, false),
      toBeArrayOfSize: (n: number) =>
        check(Array.isArray(actual) && actual.length === n, "toBeArrayOfSize(expected)", n),
      toBeInteger: () => check(Number.isInteger(actual), "toBeInteger()", undefined, false),
      toBeFinite: () => check(Number.isFinite(actual), "toBeFinite()", undefined, false),
      toBePositive: () => check(typeof actual === "number" && actual > 0, "toBePositive()", undefined, false),
      toBeNegative: () => check(typeof actual === "number" && actual < 0, "toBeNegative()", undefined, false),
      toBeEven: () => check(typeof actual === "number" && actual % 2 === 0, "toBeEven()", undefined, false),
      toBeOdd: () => check(typeof actual === "number" && Math.abs(actual % 2) === 1, "toBeOdd()", undefined, false),
      toBeEmpty: () =>
        check(
          actual === "" ||
            (Array.isArray(actual) && actual.length === 0) ||
            ((actual instanceof Map || actual instanceof Set) && actual.size === 0) ||
            (typeof actual === "object" &&
              actual !== null &&
              !Array.isArray(actual) &&
              Object.keys(actual).length === 0),
          "toBeEmpty()",
          undefined,
          false,
        ),
      toBeTypeOf: (t: string) => check(typeof actual === t, "toBeTypeOf(expected)", t),
      toBeInstanceOf: (c: Fn) => check(actual instanceof c, "toBeInstanceOf(expected)", c),
      toBeOneOf: (list: unknown[]) =>
        check(
          list.some(v => equals(actual, v, false)),
          "toBeOneOf(expected)",
          list,
        ),
      toBeGreaterThan: (n: number) => check((actual as number) > n, "toBeGreaterThan(expected)", n),
      toBeGreaterThanOrEqual: (n: number) => check((actual as number) >= n, "toBeGreaterThanOrEqual(expected)", n),
      toBeLessThan: (n: number) => check((actual as number) < n, "toBeLessThan(expected)", n),
      toBeLessThanOrEqual: (n: number) => check((actual as number) <= n, "toBeLessThanOrEqual(expected)", n),
      toBeWithin: (lo: number, hi: number) =>
        check((actual as number) >= lo && (actual as number) < hi, "toBeWithin(start, end)", [lo, hi]),
      toBeCloseTo: (n: number, digits = 2) =>
        check(Math.abs((actual as number) - n) < 10 ** -digits / 2, "toBeCloseTo(expected)", n),
      toContain: (v: unknown) =>
        check(
          typeof actual === "string"
            ? actual.includes(String(v))
            : [...((actual as Iterable<unknown>) ?? [])].includes(v),
          "toContain(expected)",
          v,
        ),
      toContainEqual: (v: unknown) =>
        check(
          [...((actual as Iterable<unknown>) ?? [])].some(x => equals(x, v, false)),
          "toContainEqual(expected)",
          v,
        ),
      toInclude: (s: string) => check(String(actual).includes(s), "toInclude(expected)", s),
      toStartWith: (s: string) => check(typeof actual === "string" && actual.startsWith(s), "toStartWith(expected)", s),
      toEndWith: (s: string) => check(typeof actual === "string" && actual.endsWith(s), "toEndWith(expected)", s),
      toHaveLength: (n: number) => check((actual as any)?.length === n, "toHaveLength(expected)", n),
      toHaveProperty: (path: string | string[], ...value: unknown[]) => {
        const parts = Array.isArray(path) ? path : path.split(".");
        let obj: any = actual,
          found = true;
        for (const p of parts) {
          if (obj == null || !(p in Object(obj))) {
            found = false;
            break;
          }
          obj = obj[p];
        }
        check(found && (value.length === 0 || equals(obj, value[0], false)), "toHaveProperty(path)", path);
      },
      toMatch: (re: RegExp | string) =>
        check(
          typeof actual === "string" && (typeof re === "string" ? actual.includes(re) : re.test(actual)),
          "toMatch(expected)",
          re,
        ),
      toThrow: (expected?: unknown) => {
        let thrown: unknown,
          threw = false;
        if (promise) {
          threw = promise === "rejects";
          thrown = actual;
        } else {
          if (typeof actual !== "function")
            throw new AssertionError("expect(received).toThrow(): received is not a function");
          try {
            (actual as Fn)();
          } catch (e) {
            threw = true;
            thrown = e;
          }
        }
        const message = (thrown as any)?.message ?? String(thrown);
        const ok =
          threw &&
          (expected === undefined ||
            (typeof expected === "string" && String(message).includes(expected)) ||
            (expected instanceof RegExp && expected.test(String(message))) ||
            (typeof expected === "function" && thrown instanceof (expected as Fn)) ||
            (typeof expected === "object" &&
              expected !== null &&
              !(expected instanceof RegExp) &&
              (isAsymmetric(expected) ? expected.match(thrown) : message === (expected as any).message)));
        const saved = actual;
        actual = threw ? thrown : "(nothing thrown)";
        try {
          check(ok, "toThrow(expected)", expected);
        } finally {
          actual = saved;
        }
      },
      toHaveBeenCalled: () =>
        check(isMock(actual) && actual.mock.calls.length > 0, "toHaveBeenCalled()", undefined, false),
      toHaveBeenCalledTimes: (n: number) =>
        check(isMock(actual) && actual.mock.calls.length === n, "toHaveBeenCalledTimes(expected)", n),
      toHaveBeenCalledWith: (...args: unknown[]) =>
        check(
          isMock(actual) && actual.mock.calls.some(c => equals(c, args, false)),
          "toHaveBeenCalledWith(...expected)",
          args,
        ),
      toHaveBeenLastCalledWith: (...args: unknown[]) =>
        check(
          isMock(actual) && equals(actual.mock.calls.at(-1), args, false),
          "toHaveBeenLastCalledWith(...expected)",
          args,
        ),
      toHaveBeenNthCalledWith: (n: number, ...args: unknown[]) =>
        check(
          isMock(actual) && equals(actual.mock.calls[n - 1], args, false),
          "toHaveBeenNthCalledWith(n, ...expected)",
          args,
        ),
      toHaveReturned: () =>
        check(
          isMock(actual) && actual.mock.results.some(r => r.type === "return"),
          "toHaveReturned()",
          undefined,
          false,
        ),
      toHaveReturnedWith: (v: unknown) =>
        check(
          isMock(actual) && actual.mock.results.some(r => r.type === "return" && equals(r.value, v, false)),
          "toHaveReturnedWith(expected)",
          v,
        ),
      toMatchSnapshot: () => {},
      toMatchInlineSnapshot: (snapshot?: string) => {
        if (snapshot === undefined) return;
        const text = typeof actual === "string" ? JSON.stringify(actual) : fmt(actual);
        const strip = (s: string) => s.trim().replace(/\s+/g, " ");
        check(
          strip(text) === strip(snapshot) || strip(String(actual)) === strip(snapshot),
          "toMatchInlineSnapshot(expected)",
          snapshot,
        );
      },
      toSatisfy: (pred: Fn) => check(!!pred(actual), "toSatisfy(predicate)", pred),
      pass: () => check(true, "pass()"),
      fail: (msg?: string) => {
        throw new AssertionError(msg ?? "fail()");
      },
    };
    matchers.toThrowError = matchers.toThrow;
    matchers.toBeCalled = matchers.toHaveBeenCalled;
    matchers.toBeCalledTimes = matchers.toHaveBeenCalledTimes;
    matchers.toBeCalledWith = matchers.toHaveBeenCalledWith;
    matchers.toHaveBeenCalledOnce = () => matchers.toHaveBeenCalledTimes(1);
    for (const [name, fn] of Object.entries(customMatchers))
      matchers[name] = (...args: unknown[]) => {
        const r = fn.call({ isNot: negate, equals: (a: unknown, b: unknown) => equals(a, b, false) }, actual, ...args);
        if (!!r?.pass === negate)
          throw new AssertionError(
            typeof r?.message === "function" ? r.message() : (r?.message ?? `expect(received).${name}()`),
          );
      };
    if (!promise) return matchers;
    const wrapped: Record<string, Fn> = {};
    for (const [name, m] of Object.entries(matchers))
      wrapped[name] = async (...args: unknown[]) => {
        let value: unknown,
          rejected = false;
        try {
          value = await actual;
        } catch (e) {
          rejected = true;
          value = e;
        }
        if (rejected !== (promise === "rejects"))
          throw new AssertionError(
            `expect(received).${promise}.${name}()\n\nReceived promise ${rejected ? "rejected" : "resolved"} with ${fmt(value)}`,
          );
        const saved = actual;
        actual = value;
        try {
          return m(...args);
        } finally {
          actual = saved;
        }
      };
    return wrapped;
  };
  const self: any = make(false);
  self.not = make(true);
  Object.defineProperty(self, "resolves", {
    get: () => Object.assign(make(false, "resolves"), { not: make(true, "resolves") }),
  });
  Object.defineProperty(self, "rejects", {
    get: () => Object.assign(make(false, "rejects"), { not: make(true, "rejects") }),
  });
  return self;
}

const typeMatch = (ctor: unknown, v: unknown) =>
  ctor === String
    ? typeof v === "string" || v instanceof String
    : ctor === Number
      ? typeof v === "number" || v instanceof Number
      : ctor === Boolean
        ? typeof v === "boolean" || v instanceof Boolean
        : ctor === BigInt
          ? typeof v === "bigint"
          : ctor === Symbol
            ? typeof v === "symbol"
            : ctor === Function
              ? typeof v === "function"
              : ctor === Object
                ? typeof v === "object" && v !== null
                : v instanceof (ctor as Fn);
Object.assign(expect, {
  any: (ctor: unknown) => asymmetric(`Any<${(ctor as Fn)?.name}>`, v => typeMatch(ctor, v)),
  anything: () => asymmetric("Anything", v => v != null),
  stringContaining: (s: string) =>
    asymmetric(`StringContaining ${fmt(s)}`, v => typeof v === "string" && v.includes(s)),
  stringMatching: (re: RegExp | string) =>
    asymmetric(
      `StringMatching ${re}`,
      v => typeof v === "string" && (typeof re === "string" ? new RegExp(re) : re).test(v),
    ),
  objectContaining: (o: object) => asymmetric(`ObjectContaining ${fmt(o)}`, v => matchObject(v, o)),
  arrayContaining: (a: unknown[]) =>
    asymmetric(`ArrayContaining ${fmt(a)}`, v => Array.isArray(v) && a.every(x => v.some(y => equals(y, x, false)))),
  closeTo: (n: number, digits = 2) =>
    asymmetric(`CloseTo ${n}`, v => typeof v === "number" && Math.abs(v - n) < 10 ** -digits / 2),
  assertions: (n: number) => void (assertionsExpected = n),
  hasAssertions: () => void (assertionsAny = true),
  unreachable: (msg?: string) => {
    throw new AssertionError(msg ?? "reached unreachable code");
  },
  extend: (custom: Record<string, Fn>) => void Object.assign(customMatchers, custom),
});

// ── mocks ─────────────────────────────────────────────────────────────────────
type MockFn = Fn & {
  mock: {
    calls: unknown[][];
    results: { type: "return" | "throw"; value: unknown }[];
    instances: unknown[];
    lastCall?: unknown[];
  };
};
function mock(impl?: Fn): MockFn {
  let implementation = impl;
  const once: Fn[] = [];
  const f = function (this: unknown, ...args: unknown[]) {
    f.mock.calls.push(args);
    f.mock.lastCall = args;
    f.mock.instances.push(this);
    try {
      const run = once.shift() ?? implementation;
      const value = run ? run.apply(this, args) : undefined;
      f.mock.results.push({ type: "return", value });
      return value;
    } catch (e) {
      f.mock.results.push({ type: "throw", value: e });
      throw e;
    }
  } as MockFn & Record<string, Fn>;
  f.mock = { calls: [], results: [], instances: [] };
  f.mockImplementation = (fn: Fn) => ((implementation = fn), f);
  f.mockImplementationOnce = (fn: Fn) => (once.push(fn), f);
  f.mockReturnValue = (v: unknown) => ((implementation = () => v), f);
  f.mockReturnValueOnce = (v: unknown) => (once.push(() => v), f);
  f.mockResolvedValue = (v: unknown) => ((implementation = () => Promise.resolve(v)), f);
  f.mockRejectedValue = (v: unknown) => ((implementation = () => Promise.reject(v)), f);
  f.mockClear = () => ((f.mock.calls = []), (f.mock.results = []), (f.mock.instances = []), f);
  f.mockReset = () => (f.mockClear(), (implementation = undefined), f);
  f.mockRestore = f.mockReset;
  f.getMockImplementation = () => implementation as Fn;
  return f;
}
function spyOn(object: any, key: string) {
  const original = object[key];
  const spy = mock((...args: unknown[]) => original.apply(object, args)) as MockFn & Record<string, Fn>;
  spy.mockRestore = () => ((object[key] = original), spy);
  object[key] = spy;
  return spy;
}
Object.assign(mock, { module: () => {}, restore: () => {}, clearAllMocks: () => {} });

// ── registration ──────────────────────────────────────────────────────────────
type Mode = Test["mode"];
function register(mode: Mode) {
  return (name: unknown, fn?: Fn | object, options?: number | { timeout?: number }) => {
    if (typeof fn === "object" && fn !== null) [fn, options] = [options as unknown as Fn, fn as { timeout?: number }];
    if (mode === "only") hasOnly = true;
    const timeout = typeof options === "number" ? options : options?.timeout;
    current.children.push({
      name: typeof name === "function" ? name.name : String(name),
      fn: fn as Fn | undefined,
      mode,
      timeout,
    });
  };
}
function each(table: unknown[], make: (name: string, fn: Fn) => void) {
  return (name: string, fn: Fn) =>
    table.forEach((row, i) => {
      const args = Array.isArray(row) ? row : [row];
      let k = 0;
      const title = name
        .replace(/%[sdifjop#%]/g, m => (m === "%%" ? "%" : m === "%#" ? String(i) : fmt(args[k++])))
        .replace(/\$([\w.]+)/g, (m, p) => (typeof row === "object" && row && p in row ? fmt((row as any)[p]) : m));
      make(title, () => fn(...args));
    });
}
function makeTest(base: Mode) {
  const t: any = register(base);
  t.skip = register("skip");
  t.todo = register("todo");
  t.only = register("only");
  t.failing = (name: string, fn: Fn) => {
    register(base)(name, fn);
    (current.children.at(-1) as Test).failing = true;
  };
  t.concurrent = t.serial = t;
  t.if = (c: unknown) => (c ? t : t.skip);
  t.skipIf = (c: unknown) => (c ? t.skip : t);
  t.todoIf = (c: unknown) => (c ? t.todo : t);
  t.each = (table: unknown[]) => each(table, (n, f) => t(n, f));
  return t;
}
function registerSuite(mode: Mode) {
  return (name: unknown, fn?: Fn) => {
    if (mode === "only") hasOnly = true;
    const suite = newSuite(typeof name === "function" ? name.name : String(name), current, mode);
    current.children.push(suite);
    const parent = current;
    current = suite;
    try {
      const r = fn?.();
      if (r && typeof r.then === "function") r.then(undefined, (e: unknown) => suiteError(suite, e));
    } catch (e) {
      suiteError(suite, e);
    } finally {
      current = parent;
    }
  };
}
function suiteError(suite: Suite, e: unknown) {
  suite.children.push({
    name: "(describe callback)",
    fn: () => {
      throw e;
    },
    mode: "run",
  });
}
const test = makeTest("run");
const describe: any = registerSuite("run");
describe.skip = registerSuite("skip");
describe.todo = registerSuite("todo");
describe.only = registerSuite("only");
describe.if = (c: unknown) => (c ? describe : describe.skip);
describe.skipIf = (c: unknown) => (c ? describe.skip : describe);
describe.todoIf = (c: unknown) => (c ? describe.todo : describe);
describe.each = (table: unknown[]) => each(table, (n, f) => describe(n, f));
describe.concurrent = describe.serial = describe;
const hook = (key: "beforeAll" | "afterAll" | "beforeEach" | "afterEach") => (fn: Fn) => void current[key].push(fn);
const beforeAll = hook("beforeAll"),
  afterAll = hook("afterAll"),
  beforeEach = hook("beforeEach"),
  afterEach = hook("afterEach");
const setDefaultTimeout = (ms: number) => void (defaultTimeout = ms);

// ── running ───────────────────────────────────────────────────────────────────
function call(fn: Fn, timeout: number): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const done = (e?: unknown) => {
      if (timer !== undefined) clearTimeout(timer);
      e === undefined ? resolve() : reject(e);
    };
    if (timeout > 0 && Number.isFinite(timeout))
      timer = setTimeout(() => done(new Error(`this test timed out after ${timeout}ms.`)), timeout);
    try {
      if (fn.length > 0) {
        fn((e?: unknown) => done(e ?? undefined));
        return;
      }
      const r = fn();
      if (r && typeof r.then === "function")
        r.then(
          () => done(),
          (e: unknown) => done(e ?? new Error("rejected")),
        );
      else done();
    } catch (e) {
      done(e);
    }
  });
}

const stats = { pass: 0, fail: 0, skip: 0, todo: 0, expects: 0 };
const failures: string[] = [];
let namePattern: RegExp | undefined;

function errorText(e: unknown): string {
  if (e instanceof Error) {
    const stack =
      typeof e.stack === "string" && e.stack.includes(e.message)
        ? e.stack
        : `${e.name}: ${e.message}\n${e.stack ?? ""}`;
    return stack.trimEnd();
  }
  return `error: ${fmt(e)}`;
}

const chain = (s: Suite): Suite[] => (s.parent ? [...chain(s.parent), s] : [s]);
const title = (s: Suite, name: string) =>
  [
    ...chain(s)
      .map(x => x.name)
      .filter(Boolean),
    name,
  ].join(" > ");
const active = (s: Suite): boolean => (s.mode === "only" ? true : s.parent ? active(s.parent) : false);
const contains = (s: Suite, pred: (t: Test) => boolean): boolean =>
  s.children.some(c => (isSuite(c) ? contains(c, pred) : pred(c)));

async function runSuite(suite: Suite, inherited: Mode): Promise<void> {
  const mode: Mode =
    inherited === "skip" || inherited === "todo" ? inherited : suite.mode === "run" ? inherited : suite.mode;
  const runs = (t: Test) =>
    mode !== "skip" &&
    mode !== "todo" &&
    t.mode !== "skip" &&
    t.mode !== "todo" &&
    !!t.fn &&
    (!hasOnly || t.mode === "only" || mode === "only" || active(suite)) &&
    (!namePattern || namePattern.test(title(suite, t.name)));
  const anyRuns = (s: Suite): boolean => s.children.some(c => (isSuite(c) ? anyRuns(c) : runs(c)));
  const live = anyRuns(suite);
  let setupError: unknown;
  if (live)
    for (const fn of suite.beforeAll)
      try {
        await call(fn, defaultTimeout);
      } catch (e) {
        setupError = e;
        break;
      }
  for (const child of suite.children) {
    if (isSuite(child)) {
      await runSuite(child, mode);
      continue;
    }
    const name = title(suite, child.name);
    if (namePattern && !namePattern.test(name)) continue;
    if (child.mode === "todo" || mode === "todo" || (!child.fn && child.mode !== "skip")) {
      stats.todo++;
      out(`(todo) ${name}\n`);
      continue;
    }
    if (!runs(child)) {
      stats.skip++;
      if (!hasOnly) out(`(skip) ${name}\n`);
      continue;
    }
    expectCalls = 0;
    assertionsExpected = undefined;
    assertionsAny = false;
    const start = now();
    let error: unknown = setupError;
    if (error === undefined) {
      const each = chain(suite);
      try {
        for (const s of each) for (const fn of s.beforeEach) await call(fn, defaultTimeout);
        await call(child.fn!, child.timeout ?? defaultTimeout);
        if (assertionsExpected !== undefined && expectCalls !== assertionsExpected)
          throw new AssertionError(`expected ${assertionsExpected} assertions, but ${expectCalls} were called`);
        if (assertionsAny && expectCalls === 0) throw new AssertionError("expected at least one assertion");
      } catch (e) {
        error = e;
      }
      for (const s of each.reverse())
        for (const fn of s.afterEach)
          try {
            await call(fn, defaultTimeout);
          } catch (e) {
            error ??= e;
          }
    }
    stats.expects += expectCalls;
    const ms = `[${(now() - start).toFixed(2)}ms]`;
    const failed = child.failing ? error === undefined : error !== undefined;
    if (failed) {
      stats.fail++;
      const text = error === undefined ? "test marked with .failing passed" : errorText(error);
      out(`${text}\n(fail) ${name} ${ms}\n`);
      failures.push(name);
    } else {
      stats.pass++;
      out(`(pass) ${name} ${ms}\n`);
    }
  }
  if (live)
    for (const fn of suite.afterAll)
      try {
        await call(fn, defaultTimeout);
      } catch (e) {
        stats.fail++;
        out(`${errorText(e)}\n(fail) ${title(suite, "afterAll")}\n`);
      }
}

(globalThis as any)[RUN] = async (pattern?: string) => {
  if (pattern) namePattern = new RegExp(pattern);
  await runSuite(root, "run");
  return stats;
};

const jest = { fn: mock, spyOn, setTimeout: setDefaultTimeout, clearAllMocks: () => {}, restoreAllMocks: () => {} };
const vi = jest;
const it = test;
const xit = test.skip,
  xtest = test.skip,
  xdescribe = describe.skip;
const setSystemTime = () => {};
const onTestFinished = afterEach;
Object.assign(globalThis, {
  describe,
  test,
  it,
  expect,
  beforeAll,
  afterAll,
  beforeEach,
  afterEach,
  mock,
  spyOn,
  jest,
  vi,
});

export {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  jest,
  mock,
  onTestFinished,
  setDefaultTimeout,
  setSystemTime,
  spyOn,
  test,
  vi,
  xdescribe,
  xit,
  xtest,
};
