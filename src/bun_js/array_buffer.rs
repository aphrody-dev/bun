//! The bytes of typed arrays, `DataView`s and their buffers.

use boa_engine::object::builtins::{
    JsArrayBuffer, JsDataView, JsSharedArrayBuffer, JsTypedArray, JsUint8Array,
};
use boa_engine::JsObject;

use crate::{JSGlobalObject, JSValue, JsResult};

impl JSValue {
    /// A copy of the bytes a typed array or a `DataView` views, as C++ reads a `JSArrayBufferView`: `None` for
    /// anything else, and for a view whose buffer is detached.
    pub fn array_buffer_view_bytes(&self, global: &JSGlobalObject) -> JsResult<Option<Vec<u8>>> {
        let Some(object) = self.0.as_object() else {
            return Ok(None);
        };
        if let Ok(array) = JsTypedArray::from_object(object.clone()) {
            let buffer = array.buffer(global.cx());
            let buffer = global.lift(buffer)?;
            let Some(bytes) = buffer_bytes(global, buffer.as_object().as_ref())? else {
                return Ok(None);
            };
            let offset = array.byte_offset(global.cx());
            let offset = global.lift(offset)?;
            let length = array.byte_length(global.cx());
            let length = global.lift(length)?;
            return Ok(bytes.get(offset..offset + length).map(<[u8]>::to_vec));
        }
        if let Ok(view) = JsDataView::from_object(object.clone()) {
            let buffer = view.buffer(global.cx());
            let buffer = global.lift(buffer)?;
            let Some(bytes) = buffer_bytes(global, buffer.as_object().as_ref())? else {
                return Ok(None);
            };
            let offset = view.byte_offset(global.cx());
            let offset = global.lift(offset)? as usize;
            let length = view.byte_length(global.cx());
            let length = global.lift(length)? as usize;
            return Ok(bytes.get(offset..offset + length).map(<[u8]>::to_vec));
        }
        Ok(None)
    }
}

/// The bytes of an `ArrayBuffer` or a `SharedArrayBuffer`, `None` where it is detached.
pub(crate) fn buffer_bytes(
    global: &JSGlobalObject,
    buffer: Option<&JsObject>,
) -> JsResult<Option<Vec<u8>>> {
    let Some(buffer) = buffer else {
        return Ok(None);
    };
    if let Ok(buffer) = JsArrayBuffer::from_object(buffer.clone()) {
        return Ok(buffer.data().map(|bytes| bytes.to_vec()));
    }
    let Ok(shared) = JsSharedArrayBuffer::from_object(buffer.clone()) else {
        return Ok(None);
    };
    let view = JsUint8Array::from_shared_array_buffer(shared, global.cx());
    let view = global.lift(view)?;
    Ok(Some(view.iter(global.cx()).collect()))
}
