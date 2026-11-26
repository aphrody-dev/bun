// toMatchSnapshot() and toMatchInlineSnapshot(): a port of src/runtime/test_runner/snapshot.rs for Bun's format and
// of expect.rs's `snapshot` and `inline_snapshot`. A file's snapshots are `__snapshots__/<file>.snap` next to it.
import { deepMatch } from "./equals.ts";
import { diffStrings, type M, type SnapshotHooks } from "./expect.ts";
import { isAsymmetric, isError, showUnquoted, snapshotText } from "./format.ts";
import { native, runtime } from "./state.ts";

const HEADER = "// Bun Snapshot v1, https://bun.sh/docs/test/snapshots\n";

export const counts = { total: 0, passed: 0, failed: 0, added: 0 };

let file: { path: string; text: string; values: Map<string, string>; dirty: boolean } | undefined;
const seen = new Map<string, number>();

function snapshotPath(test: string): string {
  const slash = test.lastIndexOf("/");
  return `${test.slice(0, slash)}/__snapshots__/${test.slice(slash + 1)}.snap`;
}

/** The entries of a snapshot file: it is a script that assigns `exports[name] = text`. */
function load() {
  if (file) return file;
  const path = snapshotPath(runtime.path);
  const text = native.readFile(path);
  const values = new Map<string, string>();
  if (text !== undefined) {
    const exports: Record<string, string> = {};
    try {
      new Function("exports", text)(exports);
    } catch {
      throw new Error(`Failed to parse snapshot file for: ${runtime.path}`);
    }
    for (const [key, value] of Object.entries(exports)) values.set(key, String(value));
  }
  file = { path, text: text || HEADER, values, dirty: false };
  return file;
}

/** `write_backtick_string`'s escapes: a backtick, a backslash and `${`. */
const escape = (text: string) => text.replace(/[`\\]|\$(?=\{)/g, c => `\\${c}`);

function nameOf(hint: string): string {
  const sequence = runtime.sequence;
  if (!sequence) throw new Error("Snapshot matchers cannot be used outside of a test");
  return hint ? `${sequence.testName}: ${hint}` : sequence.testName;
}

function addCount(name: string): number {
  counts.total++;
  const n = (seen.get(name) ?? 0) + 1;
  seen.set(name, n);
  return n;
}

/** The value with what `propertyMatchers` matched replaced by the matchers, as the snapshot shows it. */
function withMatchers(value: any, matchers: any): unknown {
  if (typeof value !== "object" || value === null || typeof matchers !== "object" || matchers === null) return value;
  const out: any = Array.isArray(value) ? [...value] : { ...value };
  for (const key of Object.keys(matchers)) {
    const matcher = matchers[key];
    out[key] = isAsymmetric(matcher) ? matcher : withMatchers(value[key], matcher);
  }
  return out;
}

/** `value_to_snapshot` for Bun's format: what was thrown is its message, if it is an error that has one. */
function valueToSnapshot(received: { thrown: boolean; value: unknown }): unknown {
  if (!received.thrown) return received.value;
  if (!isError(received.value)) return undefined;
  const message = (received.value as Error).message;
  return message ? message : undefined;
}

function printed(m: M, value: unknown, propertyMatchers: unknown): string {
  if (propertyMatchers === undefined) return snapshotText(value);
  if (typeof value !== "object" || value === null)
    m.failf("<b>Matcher error: <red>received<r> values must be an object when the matcher has <green>properties<r>\n");
  if (!deepMatch(value, propertyMatchers))
    m.withParams("<green>propertyMatchers<r>").failf(
      "Expected <green>propertyMatchers<r> to match properties from received object\n\nReceived: {}\n",
      showUnquoted(value),
    );
  return snapshotText(withMatchers(value, propertyMatchers));
}

/** `trim_leading_whitespace_for_inline_snapshot`: the indentation of a template literal's lines removed. */
export function trimInlineSnapshot(saved: string): string {
  const firstNewline = saved.indexOf("\n");
  if (firstNewline === -1 || /[^ \t]/.test(saved.slice(0, firstNewline))) return saved;
  let src = saved.slice(firstNewline + 1);
  const indent = /^[ \t]*/.exec(src)![0].length;
  if (indent === 0) return saved;
  let out = "\n";
  src = src.slice(indent);
  const nl = src.indexOf("\n");
  if (nl === -1) return saved;
  out += src.slice(0, nl + 1);
  src = src.slice(nl + 1);
  while (src.length) {
    const max = Math.min(src.length, indent);
    const lineIndent = /^[ \t]*/.exec(src.slice(0, max))![0].length;
    src = src.slice(lineIndent);
    if (lineIndent < max) {
      if (!src.length) break;
      if (src[0] === "\n") {
        out += "\n";
        src = src.slice(1);
        continue;
      }
      return saved;
    }
    const lineEnd = src.indexOf("\n");
    if (lineEnd === -1) {
      if (/[^ \t]/.test(src)) return saved;
      break;
    }
    out += src.slice(0, lineEnd + 1);
    src = src.slice(lineEnd + 1);
  }
  const last = saved.lastIndexOf("\n");
  if (/[^ \t]/.test(saved.slice(last + 1))) return saved;
  return out;
}

const CI_INLINE =
  "<b>Matcher error<r>: Inline snapshot creation is disabled in CI environments unless --update-snapshots is used.\nTo override, set the environment variable CI=false.\n\nReceived: {}";

export const hooks: SnapshotHooks = {
  toMatchSnapshot(m, received, propertyMatchers, hint) {
    const text = printed(m, valueToSnapshot(received), propertyMatchers);
    const snapshots = load();
    const name = nameOf(hint);
    const key = `${name} ${addCount(name)}`;
    const saved = snapshots.values.get(key);
    if (saved !== undefined && !native.options.updateSnapshots) {
      if (saved === text) {
        counts.passed++;
        return;
      }
      counts.failed++;
      m.withParams("<green>expected<r>").fail(diffStrings(text, saved) + "\n");
    }
    if (saved === undefined && native.isCI && !native.options.updateSnapshots)
      throw new Error(
        `Snapshot creation is disabled in CI environments unless --update-snapshots is used\nTo override, set the environment variable CI=false.\n\nSnapshot name: "${key}"\nReceived: ${text}`,
      );
    if (saved === undefined) counts.added++;
    else counts.passed++;
    snapshots.values.set(key, text);
    snapshots.text += `\nexports[\`${escape(key)}\`] = \`${escape(text)}\`;\n`;
    snapshots.dirty = true;
  },
  toMatchInlineSnapshot(m, received, propertyMatchers, saved) {
    const text = printed(m, valueToSnapshot(received), propertyMatchers);
    if (runtime.sequence) addCount(nameOf(""));
    if (saved !== undefined) {
      const trimmed = trimInlineSnapshot(saved);
      if (trimmed === text || native.options.updateSnapshots) {
        counts.passed++;
        return;
      }
      counts.failed++;
      m.withParams("<green>expected<r>").fail(diffStrings(text, trimmed) + "\n");
    }
    if (native.isCI && !native.options.updateSnapshots) m.withParams("").failf(CI_INLINE, text);
    // Bun writes the new snapshot into the test file; this build counts it and leaves the file as it is.
    counts.added++;
  },
};

/** At the end of the file: the snapshots it added, written back. */
export function save() {
  if (!file?.dirty) return;
  const dir = file.path.slice(0, file.path.lastIndexOf("/"));
  if (!native.writeFile(file.path, file.text)) throw new Error(`Failed to make snapshot directory for test file: ${dir}`);
}
