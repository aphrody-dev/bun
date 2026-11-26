//! The functions `bun_runtime_js::bun_object` puts on `Bun`, called from JavaScript. Each expected value is what
//! JavaScriptCore Bun answers for the same expression.

use bun_js::JSGlobalObject;
use bun_sema_standalone as _;

#[test]
fn deep_equals() {
    let cases: &[(&str, &str)] = &[
        ("Bun.deepEquals.length", "2"),
        ("Bun.deepEquals.name", "deepEquals"),
        ("Bun.deepEquals(1)", "throws TypeError: Expected 2 values to compare"),
        ("Bun.deepEquals({ a: 1 }, { a: 1 })", "true"),
        ("Bun.deepEquals({ a: 1, b: undefined }, { a: 1 }, true)", "false"),
        (
            "(() => { class A { constructor() { this.x = 1 } } return Bun.deepEquals(new A, { x: 1 }, true, true) })()",
            "true",
        ),
        // Only `true` is strict.
        (
            "(() => { class A { constructor() { this.x = 1 } } return Bun.deepEquals(new A, { x: 1 }, 1) })()",
            "true",
        ),
    ];
    JSGlobalObject::with_new(|global| {
        let bun = bun_js::JSObject::create(global);
        for (name, value) in bun_runtime_js::bun_object(global).expect("Bun's properties") {
            bun.put(global, name, value);
        }
        global.to_js_value().put(global, "Bun", bun.to_js());
        let failures: Vec<String> = cases
            .iter()
            .filter_map(|&(source, expected)| {
                let wrapped = format!(
                    "(() => {{ try {{ return String({source}) }} catch (e) {{ return `throws ${{e.name}}: ${{e.message}}` }} }})()"
                );
                let actual = global
                    .evaluate_script(&wrapped)
                    .expect("caught")
                    .to_utf8(global)
                    .expect("a string");
                (actual != expected).then(|| format!("{source}: {actual}, not {expected}"))
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
}
