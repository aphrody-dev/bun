//! node:vm's binding on bun_js (`$cpp("NodeVM.cpp", "Bun::createNodeVMBinding")`): contexts are realms of their own,
//! and a script sees the context object's properties as its globals and leaves its own on it.

use bun_js::JSGlobalObject;

fn check(cases: &[(&str, &str)]) {
    JSGlobalObject::with_new(|global| {
        let binding = bun_runtime_js::cpp(global, "NodeVM.cpp", "Bun::createNodeVMBinding")
            .expect("NodeVM.cpp's binding")
            .expect("made");
        global.to_js_value().put(global, "vm", binding);
        global
            .evaluate_script(
                "globalThis.r = f => { try { return String(f()) } catch (e) { return `throws ${e?.name}: ${e?.message} (${e?.code})` } }",
            )
            .expect("r");
        let mut failures = Vec::new();
        for &(source, expected) in cases {
            let actual = match global.evaluate_script(&format!("r(() => {{ {source} }})")) {
                Ok(value) => value.to_utf8(global).expect("a string"),
                Err(error) => format!("uncaught {}", global.take_exception(error).to_utf8(global).unwrap_or_default()),
            };
            if actual != expected {
                failures.push(format!("{source}\n    expected: {expected}\n    actual:   {actual}"));
            }
        }
        assert!(failures.is_empty(), "{} of {} differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
    });
}

#[test]
fn scripts_in_contexts() {
    check(&[
        ("const c = vm.createContext({ x: 2 }); return new vm.Script('x * 3').runInContext(c)", "6"),
        ("const c = vm.createContext({}); new vm.Script('var y = 5; z = 1').runInContext(c); return [c.y, c.z]", "5,1"),
        ("const c = vm.createContext({ n: 1 }); new vm.Script('n++').runInContext(c); new vm.Script('n++').runInContext(c); return c.n", "3"),
        ("const c = vm.createContext({ gone: 1 }); new vm.Script('delete globalThis.gone').runInContext(c); return 'gone' in c", "false"),
        ("const c = vm.createContext({}); return new vm.Script('this').runInContext(c) === c", "true"),
        ("const c = vm.createContext({}); return new vm.Script('Array').runInContext(c) !== Array", "true"),
        ("const c = vm.createContext({}); return new vm.Script('typeof Object.keys').runInContext(c)", "function"),
        ("return [vm.isContext(vm.createContext({})), vm.isContext({})]", "true,false"),
        ("return new vm.Script('q').runInNewContext({ q: 7 })", "7"),
        ("return new vm.Script('1 + 1').runInThisContext()", "2"),
        ("const g = vm.createContext(vm.DONT_CONTEXTIFY); return [vm.isContext(g), new vm.Script('typeof globalThis.Array').runInContext(g)]", "true,function"),
        ("try { new vm.Script('(') } catch (e) { return e.name }", "SyntaxError"),
        ("vm.createContext(1)", "throws TypeError: The \"contextObject\" argument must be of type object. Received type number (1) (ERR_INVALID_ARG_TYPE)"),
        ("new vm.Script('1').runInContext({})", "throws TypeError: The \"contextifiedObject\" argument must be an vm.Context (ERR_INVALID_ARG_TYPE)"),
        ("return vm.compileFunction('return a + b', ['a', 'b'])(1, 2)", "3"),
    ]);
}
