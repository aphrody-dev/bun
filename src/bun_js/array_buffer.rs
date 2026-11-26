//! The bytes of typed arrays, `DataView`s and their buffers, and the `Buffer`s Rust makes.

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

    /// The bytes of a view, as [`array_buffer_view_bytes`](Self::array_buffer_view_bytes), or of an `ArrayBuffer` or
    /// a `SharedArrayBuffer` itself: what node:fs takes as data or as a path.
    pub fn array_buffer_or_view_bytes(&self, global: &JSGlobalObject) -> JsResult<Option<Vec<u8>>> {
        if self.is_array_buffer_view() {
            return self.array_buffer_view_bytes(global);
        }
        buffer_bytes(global, self.0.as_object().as_ref())
    }

    /// How many bytes a typed array or a `DataView` views; `None` for anything else.
    pub fn array_buffer_view_byte_length(&self, global: &JSGlobalObject) -> JsResult<Option<usize>> {
        let Some(object) = self.0.as_object() else {
            return Ok(None);
        };
        if let Ok(array) = JsTypedArray::from_object(object.clone()) {
            let length = array.byte_length(global.cx());
            return global.lift(length).map(Some);
        }
        if let Ok(view) = JsDataView::from_object(object) {
            let length = view.byte_length(global.cx());
            return global.lift(length).map(|n| Some(n as usize));
        }
        Ok(None)
    }

    /// Whether this is a `Uint8Array` (a `Buffer` is one), a `DataView` or an `ArrayBuffer`: the buffers that
    /// node:fs takes a path in.
    pub fn is_uint8_array_data_view_or_array_buffer(&self) -> bool {
        self.0.as_object().is_some_and(|object| {
            JsUint8Array::from_object(object.clone()).is_ok()
                || JsDataView::from_object(object.clone()).is_ok()
                || JsArrayBuffer::from_object(object).is_ok()
        })
    }

    /// Copies `bytes` into the view at `offset`, as much as fits: how many were copied. A view of a detached or a
    /// shared buffer takes none.
    pub fn write_array_buffer_view(&self, global: &JSGlobalObject, offset: usize, bytes: &[u8]) -> JsResult<usize> {
        let Some(object) = self.0.as_object() else {
            return Ok(0);
        };
        let (buffer, view_offset, view_length) = if let Ok(array) = JsTypedArray::from_object(object.clone()) {
            let buffer = array.buffer(global.cx());
            let buffer = global.lift(buffer)?;
            let view_offset = array.byte_offset(global.cx());
            let view_offset = global.lift(view_offset)?;
            let view_length = array.byte_length(global.cx());
            (buffer, view_offset, global.lift(view_length)?)
        } else if let Ok(view) = JsDataView::from_object(object) {
            let buffer = view.buffer(global.cx());
            let buffer = global.lift(buffer)?;
            let view_offset = view.byte_offset(global.cx());
            let view_offset = global.lift(view_offset)? as usize;
            let view_length = view.byte_length(global.cx());
            (buffer, view_offset, global.lift(view_length)? as usize)
        } else {
            return Ok(0);
        };
        let Some(buffer) = buffer.as_object().and_then(|b| JsArrayBuffer::from_object(b).ok()) else {
            return Ok(0);
        };
        let Some(mut data) = buffer.data_mut() else {
            return Ok(0);
        };
        let start = view_offset + offset.min(view_length);
        let end = (view_offset + view_length).min(data.len());
        let n = bytes.len().min(end.saturating_sub(start));
        data[start..start + n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }

    /// A `Buffer` of `bytes`, as `bun_jsc`'s `create_buffer`: a `Uint8Array` whose prototype is
    /// `Buffer.prototype`, where the realm has `Buffer`; a plain `Uint8Array` otherwise.
    pub fn create_buffer(global: &JSGlobalObject, bytes: &[u8]) -> JsResult<JSValue> {
        let array = JsUint8Array::from_iter(bytes.iter().copied(), global.cx());
        let array: JsObject = global.lift(array)?.into();
        let buffer = global.to_js_value().get(global, "Buffer")?;
        if let Some(prototype) = buffer.map(|buffer| buffer.get(global, "prototype")).transpose()? {
            if let Some(prototype) = prototype.and_then(|prototype| prototype.0.as_object()) {
                array.set_prototype(Some(prototype));
            }
        }
        Ok(JSValue(array.into()))
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
