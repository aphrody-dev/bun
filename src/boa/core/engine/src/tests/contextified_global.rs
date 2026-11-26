use boa_macros::js_str;

use crate::{Context, JsObject, JsValue, Source, js_string, realm::Realm};

fn eval_in(context: &mut Context, realm: &Realm, source: &str) -> JsValue {
    let outer = context.enter_realm(realm.clone());
    let result = context.eval(Source::from_bytes(source));
    context.enter_realm(outer);
    result.unwrap_or_else(|error| panic!("{source}: {error}"))
}

fn setup() -> (Context, JsObject, Realm) {
    let mut context = Context::default();
    let target = JsObject::with_object_proto(context.intrinsics());
    target
        .set(js_str!("a"), 1, true, &mut context)
        .expect("set a");
    let realm = context
        .create_contextified_realm(target.clone())
        .expect("realm");
    (context, target, realm)
}

fn get(target: &JsObject, key: &str, context: &mut Context) -> JsValue {
    target.get(js_string!(key), context).expect("get")
}

#[test]
fn declarations_and_assignments_land_on_the_target() {
    let (mut context, target, realm) = setup();
    eval_in(
        &mut context,
        &realm,
        "var v = 1; function f() { return a; } w = 2; let l = 3; globalThis.p = 4;",
    );
    assert_eq!(get(&target, "v", &mut context), JsValue::from(1));
    assert!(get(&target, "f", &mut context).is_callable());
    assert_eq!(get(&target, "w", &mut context), JsValue::from(2));
    assert_eq!(get(&target, "p", &mut context), JsValue::from(4));
    assert!(
        !target
            .has_own_property(js_str!("l"), &mut context)
            .expect("has l")
    );
    assert_eq!(eval_in(&mut context, &realm, "l"), JsValue::from(3));
}

#[test]
fn reads_are_live() {
    let (mut context, target, realm) = setup();
    eval_in(&mut context, &realm, "function f() { return a; }");
    for _ in 0..3 {
        assert_eq!(
            eval_in(
                &mut context,
                &realm,
                "for (let i = 0; i < 20; i++) f(); f()"
            ),
            JsValue::from(1)
        );
    }
    target
        .set(js_str!("a"), 5, true, &mut context)
        .expect("set a");
    assert_eq!(eval_in(&mut context, &realm, "f()"), JsValue::from(5));
    assert_eq!(
        eval_in(&mut context, &realm, "globalThis.a"),
        JsValue::from(5)
    );
    target
        .set(js_str!("Array"), 6, true, &mut context)
        .expect("set Array");
    assert_eq!(eval_in(&mut context, &realm, "Array"), JsValue::from(6));
    target
        .delete_property_or_throw(js_str!("Array"), &mut context)
        .expect("delete Array");
    assert_eq!(
        eval_in(&mut context, &realm, "typeof Array"),
        JsValue::from(js_string!("function"))
    );
}

#[test]
fn builtins_are_the_realms_own() {
    let (mut context, target, realm) = setup();
    let array = eval_in(&mut context, &realm, "Array");
    let outer = context.eval(Source::from_bytes("Array")).expect("Array");
    assert!(array.is_callable());
    assert_ne!(array, outer);
    assert!(
        !target
            .has_own_property(js_str!("Array"), &mut context)
            .expect("has Array")
    );
}

#[test]
fn delete_has_and_keys_forward() {
    let (mut context, target, realm) = setup();
    assert_eq!(
        eval_in(&mut context, &realm, "'a' in globalThis"),
        JsValue::from(true)
    );
    assert_eq!(
        eval_in(
            &mut context,
            &realm,
            "var v; Object.keys(globalThis).join()"
        ),
        JsValue::from(js_string!("a,v"))
    );
    assert_eq!(
        eval_in(&mut context, &realm, "delete a"),
        JsValue::from(true)
    );
    assert!(
        !target
            .has_own_property(js_str!("a"), &mut context)
            .expect("has a")
    );
    assert_eq!(
        eval_in(&mut context, &realm, "typeof a"),
        JsValue::from(js_string!("undefined"))
    );
}

#[test]
fn the_target_reads_as_the_global() {
    let (mut context, target, realm) = setup();
    target
        .set(js_str!("self"), target.clone(), true, &mut context)
        .expect("set self");
    assert_eq!(
        eval_in(&mut context, &realm, "self === globalThis && self === this"),
        JsValue::from(true)
    );
}

#[test]
fn non_writable_builtins_stay_the_globals() {
    let (mut context, target, realm) = setup();
    assert_eq!(
        eval_in(
            &mut context,
            &realm,
            "Object.defineProperty(globalThis, 'NaN', { value: NaN }); typeof NaN"
        ),
        JsValue::from(js_string!("number"))
    );
    assert!(
        !target
            .has_own_property(js_str!("NaN"), &mut context)
            .expect("has NaN")
    );
}
