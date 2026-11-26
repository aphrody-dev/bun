#!/usr/bin/env bun
// Usage: bindgen-js.ts --sources=<crate>=<a.bind.ts>,.. --codegen-path=<dir>
//
// The `.bind.ts` schemas as Rust against `bun_js` (src/bun_js), the engine boundary on Boa: one
// `generated_bun_js_<crate>.rs` per crate, which the crate `include!`s. There is no C++: each host function converts
// its own arguments, with `bun_js::bindgen::from_js`. What it does not support yet it refuses by name, rather than
// generate something that differs from the JavaScriptCore build.
//
// Per schema file it writes a module named after the file, which holds:
//   - a `bun_js::JSHostFn` per function (`pub fn <name>(global, frame)`), and `create_<name>(global)`, its
//     function object;
//   - `create_<namespace>(global)`, the namespace object;
//   - `impl bun_js::JsClass` for each class's payload;
//   - `exports(global)`: every function, namespace and class constructor by the name JavaScript knows it by.

import path from "node:path";
import {
  AnyType,
  BoolType,
  ClassType,
  Construction,
  DefaultType,
  DoubleType,
  type FunctionDefinition,
  type Getter,
  IntegerType,
  type Method,
  type NamespaceDefinition,
  OptionalType,
  RestType,
  type Signature,
  snakeCase,
  type Type,
  UnitType,
  Utf8Type,
  VariadicType,
} from "./bindgen-lib-internal.ts";
import { loadSchemaFile, sortTypes } from "./bindgen-load.ts";
import type { SchemaFile } from "./bindgen-emit.ts";
import { argParse, writeIfNotChanged } from "./helpers.ts";

class Unsupported extends Error {}

/** What `b.<type>` the emitter does not convert yet. */
const refuse = (what: string): never => {
  throw new Unsupported(`${what} is not supported by the bun_js emitter yet`);
};

const KEYWORDS = new Set(
  "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while abstract become box do final macro override priv typeof unsized virtual yield try gen".split(
    " ",
  ),
);
const ident = (name: string) => (KEYWORDS.has(name) ? `r#${name}` : name);
const rustString = (s: string) => JSON.stringify(s);

// ───────────────────────────── types ─────────────────────────────

/** The `from_js` function for an argument of `type` that is not optional. */
function converter(type: Type): string {
  if (type instanceof BoolType) return type.valueKinds === "any" ? "loose_bool" : "strict_bool";
  if (type instanceof IntegerType) {
    const integer = type.idlType.match(/^Bun::Bindgen::IDL(Strict|Loose)Integer<([^,>]+)>$/);
    if (!integer) return refuse(`${type.idlType} (an integer with a range, .wrap or .allowBigInt)`);
    return `${integer[1] === "Strict" ? "strict" : "loose"}_integer::<${type.rustName}>`;
  }
  if (type instanceof DoubleType) {
    if (type.idlType === "Bun::Bindgen::IDLStrictDouble") return "strict_f64";
    if (type.idlType === "WebCore::IDLUnrestrictedDouble") return "loose_f64";
    return refuse(type.idlType);
  }
  if (type instanceof Utf8Type) return type.isLoose ? "loose_utf8" : "strict_utf8";
  if (type instanceof AnyType) {
    if (type.cppPrepare("value") !== null) return refuse("b.any.stringObjectAsString");
    return "any";
  }
  return refuse(`${type.constructor.name} (${type.tsType})`);
}

/** Rust that converts `value` (a `JSValue`) to an argument of `type`, an expression of `bindgen::Result<'a, T>`. */
function convert(type: Type, value: string, name: string): string {
  const call = (inner: Type) => `bindgen::from_js::${converter(inner)}(heap, &${value}, ${rustString(name)})`;
  if (type instanceof OptionalType) {
    const absent = { optional: `${value}.is_undefined()`, nullable: `${value}.is_undefined_or_null()` }[
      type.modifier as "optional" | "nullable"
    ];
    if (!absent) return refuse(`.${type.modifier}`);
    return `if ${absent} { Ok(None) } else { ${call(type.inner)}.map(Some) }`;
  }
  if (type instanceof DefaultType) {
    const absent = type.onNull ? `${value}.is_undefined_or_null()` : `${value}.is_undefined()`;
    // A byte string literal is an array: the slice is what the conversion gives.
    const fallback =
      type.inner instanceof Utf8Type ? `&${type.inner.toRust(type.value)}[..]` : type.inner.toRust(type.value);
    return `if ${absent} { Ok(${fallback}) } else { ${call(type.inner)} }`;
  }
  return call(type);
}

/** Rust that converts `value` (what a body returns) to JS, an expression of `bindgen::Result<'a, JsLocal<'a>>`. */
function toJs(type: Type, value: string): string {
  if (type instanceof OptionalType) {
    if (type.modifier !== "nullable" && type.modifier !== "optional") return refuse(`a result of .${type.modifier}`);
    const absent = type.modifier === "nullable" ? "bun_js::JSValue::NULL" : "bun_js::JSValue::UNDEFINED";
    return `match ${value} { Some(value) => ${toJs(type.inner, "value")}, None => Ok(bindgen::JsLocal::new(heap, ${absent})) }`;
  }
  if (type instanceof UnitType) {
    if (type.unit !== "undefined") return refuse("a result of b.null");
    return `bindgen::to_js::undefined(heap, ${value})`;
  }
  if (type instanceof BoolType || type instanceof DoubleType) return `bindgen::to_js::${type.rustName}(heap, ${value})`;
  if (type instanceof IntegerType) {
    if (!/^Bun::Bindgen::IDL(Strict|Loose)Integer<[^,>]+>$/.test(type.idlType))
      return refuse(`a result of ${type.idlType}`);
    return `bindgen::to_js::${type.rustName}(heap, ${value})`;
  }
  if (type instanceof Utf8Type) return `bindgen::to_js::utf8(heap, ${value})`;
  if (type instanceof AnyType) return `bindgen::to_js::any(heap, ${value})`;
  return refuse(`a result of ${type.constructor.name} (${type.tsType})`);
}

// ───────────────────────────── calls ─────────────────────────────

const capabilityArgument = { none: [], heap: ["&cx.heap"], js: ["cx"] } as const;

/** What a host function returns for the body's `result`. */
const returnConverted = (signature: Signature) => (result: string) =>
  `bindgen::host_fn_return(cx, ${result}, |heap, value| ${toJs(signature.returns, "value")})`;

/** The statements that convert a signature's arguments from `frame`, then run `beforeCall` (which borrows a class's
 * payload, once nothing else can run script), then `call` with them, then `finish`. */
function body(
  signature: Signature,
  call: (args: string[]) => string,
  finish: (result: string) => string = returnConverted(signature),
  beforeCall: string[] = [],
): string {
  const frame = "frame";
  if (signature.args.some(arg => arg.type instanceof RestType || arg.type instanceof VariadicType))
    refuse("b.rest and .variadic");
  if (signature.rejects) refuse("rejects");
  if (signature.namedAsBound || signature.replacesBoundFunction) refuse("namedAsBound and replacesBoundFunction");
  for (const arg of signature.args)
    if (arg.expected !== undefined || arg.validatorExpects !== undefined)
      refuse(`${arg.name}: expected and validatorExpects (Node's ERR_INVALID_ARG_TYPE text)`);
  const lines: string[] = ["let heap = &cx.heap;"];
  if (signature.requiredArguments > 0) {
    // As bindgen-emit.ts's `cppThrowIfArgumentsMissing`: the message names every required argument.
    const names = signature.args.slice(0, signature.requiredArguments).map(arg => `"${arg.name}"`);
    const message = `The ${new Intl.ListFormat("en").format(names)} argument${names.length > 1 ? "s" : ""} must be specified`;
    lines.push(
      `if ${frame}.arguments_count() < ${signature.requiredArguments} { return bindgen::from_js::missing_arguments(heap, ${rustString(message)}).into_host_fn_return(cx); }`,
    );
  }
  const names = signature.args.map(arg => ident(snakeCase(arg.name)));
  for (const index of signature.conversionOrder) {
    const arg = signature.args[index];
    lines.push(
      `let ${names[index]} = match ${convert(arg.type, `${frame}.argument(${index})`, arg.name)} { Ok(value) => value, Err(error) => return error.into_host_fn_return(cx) };`,
    );
  }
  lines.push(...beforeCall);
  const invoked = call([...capabilityArgument[signature.capability], ...names]);
  const result = signature.throws
    ? invoked
    : signature.returns instanceof UnitType && !(signature instanceof Construction)
      ? `{ ${invoked}; Ok::<_, core::convert::Infallible>(()) }`
      : `Ok::<_, core::convert::Infallible>(${invoked})`;
  lines.push(`let result = ${result};`);
  lines.push(finish("result"));
  return lines.join("\n");
}

const scoped = (global: string, statements: string) =>
  `bindgen::JsScope::with(${global}, |cx| {\n${indent(statements, 4)}\n})`;

function indent(text: string, by: number): string {
  const pad = " ".repeat(by);
  return text
    .split("\n")
    .map(line => (line ? pad + line : line))
    .join("\n");
}

function onlySignature(fn: FunctionDefinition, what: string): Signature {
  if (fn.signatures.length !== 1) refuse(`${what}: b.overloads`);
  const signature = fn.signatures[0];
  if (signature.cppSymbol) refuse(`${what}: cppSymbol`);
  if (fn.passThis || fn.passCallee) refuse(`${what}: passThis and passCallee`);
  return signature;
}

function rustForFunction(fn: FunctionDefinition, file: SchemaFile): string {
  const signature = onlySignature(fn, fn.name);
  const modulePath = file.moduleDefinition?.rustPath;
  const target =
    signature.rustPath ??
    (modulePath ? `${modulePath}::${ident(snakeCase(fn.name))}` : refuse(`${fn.name}: a rustPath`));
  const host = ident(snakeCase(fn.name));
  return `
/// \`${fn.jsName}\`
pub fn ${host}(global: &JSGlobalObject, frame: &CallFrame<'_>) -> JsResult<JSValue> {
${indent(
  scoped(
    "global",
    body(signature, args => `${target}(${args.join(", ")})`),
  ),
  4,
)}
}

/// The function object of \`${fn.jsName}\`.
pub fn create_${snakeCase(fn.name)}(global: &JSGlobalObject) -> JSValue {
    bun_js::JSFunction::create(global, ${rustString(fn.jsName)}, ${host}, ${fn.length})
}
`;
}

function rustForNamespace(namespace: NamespaceDefinition): string {
  if (namespace.call) refuse(`${namespace.name}: a namespace that is callable`);
  const puts = namespace.functions
    // As C++'s namespace object: each function is named by its key, not its `jsName`.
    .map(
      fn =>
        `object.put(global, ${rustString(fn.name)}, bun_js::JSFunction::create(global, ${rustString(fn.name)}, ${ident(snakeCase(fn.name))}, ${fn.length}));`,
    )
    .join("\n");
  return `
/// The namespace \`${namespace.name}\`.
pub fn create_${snakeCase(namespace.name)}(global: &JSGlobalObject) -> JsResult<JSValue> {
    let object = bun_js::JSObject::create(global);
${indent(puts, 4)}
    Ok(object.to_js())
}
`;
}

// ───────────────────────────── classes ─────────────────────────────

function rustForClass(type: ClassType): string {
  const what = `class ${type.name}`;
  const unsupported = (
    [
      ["extends", type.base],
      ["refCounted", type.refCounted],
      ["passStructure", type.passStructure],
      ["webIDL", type.webIDL],
      ["call", type.call],
      ["callInstance", type.callInstance],
      ["internalFields", type.internalFields.length],
      ["staticGetters", type.staticGetters.length],
      ["calledFromCpp", type.calledFromCpp.length],
      ["calledFromRust", type.calledFromRust.length],
      ["protoCaller", type.protoCaller],
      ["prototypeBase", type.prototypeBase],
      ["builtins", type.builtins.length],
      ["constants", type.constants.length],
      ["finalize", type.finalize],
      ["thisValue", type.thisValue],
      ["forBind", type.forBind],
      ["overridesToJS", type.overridesToJS],
    ] as const
  ).filter(([, value]) => value);
  if (unsupported.length) refuse(`${what}: ${unsupported.map(([key]) => key).join(", ")}`);
  const rustPath = type.rustPath;
  const members: string[] = [];
  const attributes = (enumerable = type.enumerable, configurable = type.configurable) =>
    `bun_js::class::Attributes { writable: ${type.writable}, enumerable: ${enumerable}, configurable: ${configurable} }`;
  const named = (name: string) => {
    // A key such as `@@iterator` is a well-known symbol in C++, which this emitter does not define yet.
    if (name.startsWith("@@")) refuse(`${what}.${name}: a symbol-keyed member`);
    return rustString(name);
  };
  /** Borrows the payload as the member's receiver wants it: after every conversion, for the body alone. */
  const borrow = (receiver: string) =>
    receiver === "mut"
      ? ["let mut this = this.borrow_mut(global)?;", "let this = &mut *this;"]
      : ["let this = this.borrow(global)?;", "let this = &*this;"];

  for (const fn of type.functions) {
    const method = onlySignature(fn, `${type.name}.${fn.name}`) as Method;
    if (method.passThis || method.answersThis || method.privateSymbol || method.functionName)
      refuse(`${what}.${fn.name}: passThis, answersThis, privateSymbol, functionName`);
    if (method.invalidThis !== "throw") refuse(`${what}.${fn.name}: invalidThis "${method.invalidThis}"`);
    const call = (args: string[]) => `<${rustPath}>::${method.rustName}(${["this", ...args].join(", ")})`;
    const statements = body(method, call, returnConverted(method), borrow(method.receiver));
    members.push(
      `class.method(${named(fn.name)}, ${fn.length}, ${attributes()}, |this, global, frame| ${scoped("global", statements)})?;`,
    );
  }

  for (const fn of type.staticFunctions) {
    const method = onlySignature(fn, `${type.name}.${fn.name}`) as Method;
    const call = (args: string[]) => `<${rustPath}>::${method.rustName}(${args.join(", ")})`;
    members.push(
      `class.static_method(${named(fn.name)}, ${fn.length}, ${attributes()}, |global, frame| ${scoped("global", body(method, call))})?;`,
    );
  }

  for (const getter of type.getters) {
    named(getter.name);
    members.push(rustForGetter(type, getter, attributes(getter.enumerable, getter.configurable), borrow));
  }

  const [construction, ...more] = type.illegalConstructor ? [] : type.constructions;
  if (more.length) refuse(`${what}: overloaded constructors`);
  const construct = construction
    ? `
    fn construct(global: &JSGlobalObject, frame: &CallFrame<'_>) -> JsResult<Self> {
${indent(
  scoped(
    "global",
    body(
      construction,
      args => `<${rustPath}>::${construction.rustName}(${args.join(", ")})`,
      result => `bindgen::host_fn_construct_return(cx, ${result})`,
    ),
  ),
  8,
)}
    }
`
    : "";

  return `
impl bun_js::JsClass for ${rustPath} {
    const NAME: &'static str = ${rustString(type.name)};
    const LENGTH: usize = ${type.construct?.length ?? 0};
${construct}
    fn define(class: &mut bun_js::class::ClassBuilder<'_, Self>) -> JsResult<()> {
${indent(members.join("\n"), 8)}
        Ok(())
    }
}
`;
}

function rustForGetter(
  type: ClassType,
  getter: Getter,
  attributes: string,
  borrow: (receiver: string) => string[],
): string {
  const what = `${type.name}.${getter.name}`;
  if (getter.internalField || getter.passThis || getter.isStatic) refuse(`${what}: cache, passThis, static`);
  const capability = (c: keyof typeof capabilityArgument) => capabilityArgument[c];
  const get = `<${type.rustPath}>::${getter.rustName}(${["this", ...capability(getter.capability)].join(", ")})`;
  // The borrow lasts until the result is converted: what a getter returns may be a slice of the payload.
  const getBody = [
    "let heap = &cx.heap;",
    ...borrow(getter.receiver),
    `let result = ${getter.throws ? get : `Ok::<_, core::convert::Infallible>(${get})`};`,
    `bindgen::host_fn_return(cx, result, |heap, value| ${toJs(getter.type, "value")})`,
  ].join("\n");
  const getterClosure = `|this, global| ${scoped("global", getBody)}`;
  if (!getter.hasSetter) return `class.getter(${rustString(getter.name)}, ${attributes}, ${getterClosure})?;`;
  const set = `<${type.rustPath}>::${getter.setterRustName}(${["this", ...capability(getter.setterCapability), "value"].join(", ")})`;
  const setBody = [
    "let heap = &cx.heap;",
    `let value = match ${convert(getter.type, "value", getter.name)} { Ok(value) => value, Err(error) => return error.into_host_fn_return(cx) };`,
    ...borrow(getter.setterReceiver),
    `let result = ${getter.throws ? set : `{ ${set}; Ok::<_, core::convert::Infallible>(()) }`};`,
    "bindgen::host_fn_return(cx, result, bindgen::to_js::undefined).map(drop)",
  ].join("\n");
  return `class.accessor(${rustString(getter.name)}, ${attributes}, ${getterClosure}, |this, global, value| ${scoped("global", setBody)})?;`;
}

// ───────────────────────────── files ─────────────────────────────

function rustForFile(file: SchemaFile): string {
  const items: string[] = [];
  const exports: string[] = [];
  for (const type of file.types) {
    if (!(type instanceof ClassType)) refuse(`${type.constructor.name} ${type.name}`);
    items.push(rustForClass(type));
    // As C++'s `$bindgen`: a class with `construct`, an illegal one included, has its constructor exported.
    if (type.construct)
      exports.push(`(${rustString(type.name)}, bun_js::class::constructor::<${type.rustPath}>(global)?),`);
  }
  // As C++'s `$bindgen(file, name)`: every function by its key, those of a namespace too.
  for (const fn of file.functions) {
    if (fn.isValue) refuse(`${fn.name}: b.value`);
    items.push(rustForFunction(fn, file));
    exports.push(`(${rustString(fn.name)}, create_${snakeCase(fn.name)}(global)),`);
  }
  const namespaces: string[] = [];
  for (const namespace of file.namespaces) {
    items.push(rustForNamespace(namespace));
    namespaces.push(`(${rustString(namespace.name)}, create_${snakeCase(namespace.name)}(global)?),`);
  }
  return `
/// \`${file.origin.stem}.bind.ts\`
#[allow(non_snake_case, unused_variables, clippy::redundant_closure, clippy::needless_return)]
pub mod ${file.origin.rustModule} {
    use bun_js::{CallFrame, JSGlobalObject, JSValue, JsResult, bindgen};
${indent(items.join(""), 4)}
    /// What \`$bindgen(file, name)\` answers in the JavaScriptCore build: each function and class constructor by name.
    pub fn exports(global: &JSGlobalObject) -> JsResult<Vec<(&'static str, JSValue)>> {
        Ok(vec![
${indent(exports.join("\n"), 12)}
        ])
    }

    /// The namespace objects, by name, which the embedder puts where JavaScript finds them (\`Bun.semver\`).
    pub fn namespaces(global: &JSGlobalObject) -> JsResult<Vec<(&'static str, JSValue)>> {
        Ok(vec![
${indent(namespaces.join("\n"), 12)}
        ])
    }
}
`;
}

/** As bindgen.ts: a type or an `exportName` belongs to one schema. */
function assertOwnersUnique(files: readonly SchemaFile[]) {
  const owners = new Map<string, string>();
  const own = (name: string, file: SchemaFile, what: string) => {
    const owner = owners.get(name);
    if (owner !== undefined) throw new Error(`${what} ${name} is in ${owner}.bind.ts and ${file.origin.stem}.bind.ts`);
    owners.set(name, file.origin.stem);
  };
  for (const file of files) {
    for (const { name } of file.types) own(name, file, "type");
    for (const { exportName } of file.functions) if (exportName) own(exportName, file, "export");
  }
}

export function rustForCrate(files: readonly SchemaFile[]): string {
  assertOwnersUnique(files);
  return (
    "// Generated by src/codegen/bindgen-js.ts. Do not edit.\n" +
    files
      .map(file => {
        try {
          return rustForFile(file);
        } catch (error) {
          if (error instanceof Unsupported) throw new Error(`${file.origin.stem}.bind.ts: ${error.message}`);
          throw error;
        }
      })
      .join("")
  );
}

if (import.meta.main) {
  const args = argParse(["sources", "codegen-path"]);
  const sources = (args.sources as string).split(",").filter(Boolean).sort();
  const files: SchemaFile[] = [];
  for (const source of sources) files.push(await loadSchemaFile(source));
  files.forEach(sortTypes);
  for (const [crate, crateFiles] of Map.groupBy(files, file => file.origin.crate)) {
    writeIfNotChanged(path.join(args["codegen-path"], `generated_bun_js_${crate}.rs`), rustForCrate(crateFiles));
  }
}
