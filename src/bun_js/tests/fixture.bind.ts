// What tests/bindgen.rs calls through the Rust that src/codegen/bindgen-js.ts writes from it: one of each kind of
// member and conversion the emitter supports.
import * as b from "../../codegen/bindgen-lib.ts";

export default b.module({ rustPath: "crate::fixture" });

export const add = b.fn({
  args: { a: b.i32, b: b.i32.loose },
  returns: b.i32,
  requiredArguments: 1,
});

export const greet = b.fn({
  args: { name: b.string.utf8, punctuation: b.string.loose.utf8.default("!") },
  returns: b.string.utf8,
  capability: "heap",
  throws: true,
});

export const call = b.fn({
  args: { callback: b.any, flag: b.bool.optional },
  returns: b.any,
  capability: "js",
  throws: true,
});

export const math = b.namespace({
  half: { args: { value: b.f64 }, returns: b.f64 },
  isTruthy: { args: { value: b.bool.loose }, returns: b.bool },
});

export const Counter = b.class({
  rustPath: "crate::fixture::Counter",
  construct: { args: { start: b.u32.default(0) } },
  proto: {
    count: { getter: b.u32 },
    label: { getter: b.string.utf8, setter: true },
    increment: { args: { by: b.u32.default(1) }, returns: b.u32, mutates: true },
    describe: { returns: b.string.utf8, capability: "heap", throws: true },
  },
  static: {
    zero: { returns: b.u32 },
  },
});
