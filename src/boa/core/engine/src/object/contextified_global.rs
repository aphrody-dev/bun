//! The global object of a realm that a host "contextifies" with an object, as Node's `vm.createContext(object)`
//! does: a script's globals are first looked up on that object, and what it declares, assigns, defines or deletes on
//! its global happens on that object. The realm's builtins stay the global's own properties, under the object's.
//! JavaScriptCore's `NodeVMGlobalObject` (src/jsc/bindings/NodeVM.cpp) does the same with its interceptors.

use boa_gc::{Finalize, GcRefCell, Trace};

use crate::{
    Context, JsResult, JsValue,
    object::{
        JsData, JsObject,
        internal_methods::{
            InternalMethodPropertyContext, InternalObjectMethods, ORDINARY_INTERNAL_METHODS,
            ordinary_define_own_property, ordinary_delete, ordinary_get, ordinary_get_own_property,
            ordinary_has_property, ordinary_own_property_keys, ordinary_set, ordinary_try_get,
        },
        shape::slot::SlotAttributes,
    },
    property::{PropertyDescriptor, PropertyKey},
    realm::Realm,
};

/// The data of a contextified global object: the object it forwards to, once the realm's builtins are in place.
#[derive(Debug, Trace, Finalize)]
pub(crate) struct ContextifiedGlobal {
    target: GcRefCell<Option<JsObject>>,
}

impl JsData for ContextifiedGlobal {
    fn internal_methods(&self) -> &'static InternalObjectMethods {
        static METHODS: InternalObjectMethods = InternalObjectMethods {
            __get_own_property__: get_own_property,
            __define_own_property__: define_own_property,
            __has_property__: has_property,
            __try_get__: try_get,
            __get__: get,
            __set__: set,
            __delete__: delete,
            __own_property_keys__: own_property_keys,
            ..ORDINARY_INTERNAL_METHODS
        };
        &METHODS
    }
}

fn target(global: &JsObject) -> Option<JsObject> {
    global
        .downcast_ref::<ContextifiedGlobal>()?
        .target
        .borrow()
        .clone()
}

/// The object `global` forwards to; a lookup that reaches it is not cached by the global's shape.
fn target_of(
    global: &JsObject,
    context: &mut InternalMethodPropertyContext<'_>,
) -> Option<JsObject> {
    let target = target(global)?;
    context.slot().attributes |= SlotAttributes::NOT_CACHEABLE;
    Some(target)
}

/// `[[GetOwnProperty]]`: the object's own property, else the global's.
fn get_own_property(
    obj: &JsObject,
    key: &PropertyKey,
    context: &mut InternalMethodPropertyContext<'_>,
) -> JsResult<Option<PropertyDescriptor>> {
    if let Some(target) = target_of(obj, context)
        && let Some(desc) = target.__get_own_property__(key, context)?
    {
        return Ok(Some(desc));
    }
    ordinary_get_own_property(obj, key, context)
}

/// `[[DefineOwnProperty]]` defines on the object, but a property the global has as neither writable nor configurable
/// (`undefined`, `NaN`, `Infinity`) stays the global's.
fn define_own_property(
    obj: &JsObject,
    key: &PropertyKey,
    desc: PropertyDescriptor,
    context: &mut InternalMethodPropertyContext<'_>,
) -> JsResult<bool> {
    let Some(target) = target_of(obj, context) else {
        return ordinary_define_own_property(obj, key, desc, context);
    };
    if let Some(own) = ordinary_get_own_property(obj, key, context)?
        && own.writable() == Some(false)
        && own.configurable() == Some(false)
    {
        return ordinary_define_own_property(obj, key, desc, context);
    }
    target.__define_own_property__(key, desc, context)
}

/// `[[HasProperty]]`: an own property of the object, or a property of the global.
fn has_property(
    obj: &JsObject,
    key: &PropertyKey,
    context: &mut InternalMethodPropertyContext<'_>,
) -> JsResult<bool> {
    if let Some(target) = target_of(obj, context)
        && target.__get_own_property__(key, context)?.is_some()
    {
        return Ok(true);
    }
    ordinary_has_property(obj, key, context)
}

/// `[[Get]]` of the object, with its prototypes, if it has the property; the object itself reads as the global.
fn get_from_target(
    obj: &JsObject,
    key: &PropertyKey,
    context: &mut InternalMethodPropertyContext<'_>,
) -> JsResult<Option<JsValue>> {
    let Some(target) = target_of(obj, context) else {
        return Ok(None);
    };
    let value = target.__try_get__(key, target.clone().into(), context)?;
    Ok(value.map(|value| match value.as_object() {
        Some(object) if object == target => obj.clone().into(),
        _ => value,
    }))
}

fn try_get(
    obj: &JsObject,
    key: &PropertyKey,
    receiver: JsValue,
    context: &mut InternalMethodPropertyContext<'_>,
) -> JsResult<Option<JsValue>> {
    if let Some(value) = get_from_target(obj, key, context)? {
        return Ok(Some(value));
    }
    ordinary_try_get(obj, key, receiver, context)
}

fn get(
    obj: &JsObject,
    key: &PropertyKey,
    receiver: JsValue,
    context: &mut InternalMethodPropertyContext<'_>,
) -> JsResult<JsValue> {
    if let Some(value) = get_from_target(obj, key, context)? {
        return Ok(value);
    }
    ordinary_get(obj, key, receiver, context)
}

/// `[[Set]]` sets on the object, with the object as the receiver.
fn set(
    obj: &JsObject,
    key: PropertyKey,
    value: JsValue,
    receiver: JsValue,
    context: &mut InternalMethodPropertyContext<'_>,
) -> JsResult<bool> {
    match target_of(obj, context) {
        Some(target) => target.__set__(key, value, target.clone().into(), context),
        None => ordinary_set(obj, key, value, receiver, context),
    }
}

/// `[[Delete]]` deletes from the object, and then from the global.
fn delete(
    obj: &JsObject,
    key: &PropertyKey,
    context: &mut InternalMethodPropertyContext<'_>,
) -> JsResult<bool> {
    if let Some(target) = target_of(obj, context)
        && !target.__delete__(key, context)?
    {
        return Ok(false);
    }
    ordinary_delete(obj, key, context)
}

/// `[[OwnPropertyKeys]]`: the object's keys, then the global's that the object does not have.
fn own_property_keys(obj: &JsObject, context: &mut Context) -> JsResult<Vec<PropertyKey>> {
    let mut keys = ordinary_own_property_keys(obj, context)?;
    if let Some(target) = target(obj) {
        let mut theirs = target.__own_property_keys__(context)?;
        keys.retain(|key| !theirs.contains(key));
        theirs.append(&mut keys);
        keys = theirs;
    }
    Ok(keys)
}

impl Context {
    /// A new realm with the default global bindings, whose global object is "contextified" with `target`: a script's
    /// globals are looked up on `target` first, and what it declares, assigns, defines or deletes on its global happens
    /// on `target`. What Node's `vm.createContext(target)` makes.
    ///
    /// # Errors
    ///
    /// The errors of creating a realm.
    pub fn create_contextified_realm(&mut self, target: JsObject) -> JsResult<Realm> {
        let hooks = self.host_hooks();
        let realm = Realm::create_with_global(hooks.as_ref(), self.root_shape(), |intrinsics| {
            JsObject::from_proto_and_data(
                intrinsics.constructors().object().prototype(),
                ContextifiedGlobal {
                    target: GcRefCell::new(None),
                },
            )
        })?;
        let outer = self.enter_realm(realm);
        let bound = crate::builtins::set_default_global_bindings(self);
        let realm = self.enter_realm(outer);
        bound?;
        if let Some(global) = realm.global_object().downcast_ref::<ContextifiedGlobal>() {
            *global.target.borrow_mut() = Some(target);
        }
        Ok(realm)
    }
}
