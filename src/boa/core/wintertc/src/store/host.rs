//! Serializable platform objects (HTML §2.7.3, `[Serializable]`): an embedder's own objects, such as a `Blob`, that
//! the structured clone algorithm serializes with the embedder's steps instead of as plain objects.

use std::fmt::Debug;

use boa_engine::{Context, JsObject, JsResult, JsValue};

/// What an embedder's serialization steps made of one of its objects: it is [`Send`], as the rest of a
/// [`JsValueStore`](super::JsValueStore) is, and its deserialization steps make the object again in any context.
pub trait HostSerialized: Debug + Send + Sync {
    /// The deserialization steps: a new object of the platform object's interface, in `context`.
    ///
    /// # Errors
    /// Whatever making the object throws.
    fn deserialize(&self, context: &mut Context) -> JsResult<JsValue>;
}

/// The serialization steps of an embedder's platform objects: `Ok(None)` for an object that is not one of them,
/// which is then cloned as the algorithm clones any other.
pub type HostSerializer =
    fn(object: &JsObject, context: &mut Context) -> JsResult<Option<Box<dyn HostSerialized>>>;

struct Serializer(HostSerializer);

/// Makes `serializer` the serialization steps of `context`'s platform objects, replacing any set before.
pub fn set_host_serializer(context: &mut Context, serializer: HostSerializer) {
    context.insert_data(Serializer(serializer));
}

pub(super) fn serialize(
    object: &JsObject,
    context: &mut Context,
) -> JsResult<Option<Box<dyn HostSerialized>>> {
    match context.get_data::<Serializer>().map(|serializer| serializer.0) {
        Some(serializer) => serializer(object, context),
        None => Ok(None),
    }
}
