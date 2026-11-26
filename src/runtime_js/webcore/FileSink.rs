//! `Bun.file().writer()`, `Bun.stdout.writer()`: src/runtime/webcore/FileSink.rs on bun_js, and the source of a
//! stream that reads a descriptor (`Bun.stdin.stream()`, `Bun.file(fd).stream()`).
//!
//! The writes are synchronous: a sink buffers until `flush()` or `end()`, then writes all of it, waiting where a
//! non-blocking descriptor is full. A read waits for its chunk the same way.

use boa_gc::{Finalize, Trace};
use bun_js::class::{Attributes, ClassBuilder, ClassRef};
use bun_js::{JSGlobalObject, JSValue, JsClass, JsResult};
use bun_paths::Path;
use bun_sys::{Fd, O};

use super::store::{self, PathOrFileDescriptor, mkdirp_parent};
use super::write_file::write_all;
use super::{Blob, throw_system_error};
use crate::sys_js::SystemError;
use crate::web::streams::internals;
use crate::web::streams::readable_stream;
use crate::web::streams::readable_stream_default_controller::{self as controller, SourceAlgorithms};

#[derive(Trace, Finalize)]
pub struct FileSink {
    #[unsafe_ignore_trace]
    fd: Option<Fd>,
    /// The file that the sink opened, which `end()` closes.
    #[unsafe_ignore_trace]
    owned: Option<bun_sys::File>,
    #[unsafe_ignore_trace]
    buffer: Vec<u8>,
    ended: bool,
}

fn sink_of(value: &JSValue) -> ClassRef<FileSink> {
    value.as_class_ref::<FileSink>().expect("a FileSink")
}

/// `blob.writer(options)`: a path is opened for writing now, truncated, its directory made if missing.
pub fn writer(global: &JSGlobalObject, blob: &Blob, options: &JSValue) -> JsResult<JSValue> {
    let Some(file) = blob.file() else {
        return Err(global.throw_type_error(format_args!("Cannot write to a Blob backed by bytes, which are always read-only")));
    };
    // `highWaterMark` is read, as Bun does, but writes wait for `flush()` and `end()` here.
    internals::member(global, options, "highWaterMark")?;
    let (fd, owned) = match &file.pathlike {
        PathOrFileDescriptor::Fd(fd) => (*fd, None),
        PathOrFileDescriptor::Path(path) => {
            let path_z = Path::from(path.as_slice());
            let flags = O::WRONLY | O::CREAT | O::TRUNC | O::CLOEXEC;
            let opened = bun_sys::File::open(&path_z, flags, 0o664).or_else(|err| {
                if err.get_errno() == bun_sys::E::ENOENT && mkdirp_parent(path).is_ok() {
                    bun_sys::File::open(&path_z, flags, 0o664)
                } else {
                    Err(err)
                }
            });
            match opened {
                Ok(file) => (file.fd(), Some(file)),
                Err(err) => return Err(throw_system_error(global, &SystemError::from(err.with_path(path)))),
            }
        }
    };
    bun_js::class::to_js(
        global,
        FileSink {
            fd: Some(fd),
            owned,
            buffer: Vec::new(),
            ended: false,
        },
    )
}

/// Writes what is buffered. Returns how many bytes.
fn flush(global: &JSGlobalObject, sink: &JSValue) -> JsResult<usize> {
    let (fd, bytes) = {
        let s = sink_of(sink);
        let mut s = s.borrow_mut(global)?;
        (s.fd, std::mem::take(&mut s.buffer))
    };
    let Some(fd) = fd else { return Ok(0) };
    if bytes.is_empty() {
        return Ok(0);
    }
    write_all(fd, &bytes).map_err(|err| throw_system_error(global, &SystemError::from(err)))
}

fn end(global: &JSGlobalObject, sink: &JSValue) -> JsResult<JSValue> {
    if sink_of(sink).borrow(global)?.ended {
        return Ok(JSValue::js_number(0.0));
    }
    let flushed = flush(global, sink)?;
    let owned = {
        let s = sink_of(sink);
        let mut s = s.borrow_mut(global)?;
        s.ended = true;
        s.fd = None;
        s.owned.take()
    };
    drop(owned);
    Ok(JSValue::js_number(flushed as f64))
}

impl JsClass for FileSink {
    const NAME: &'static str = "FileSink";

    fn define(class: &mut ClassBuilder<'_, Self>) -> JsResult<()> {
        let attributes = Attributes::DEFAULT;
        class.method("write", 1, attributes, |this, global, frame| {
            let chunk = frame.argument(0);
            let bytes = match crate::engine::buffer_source_bytes(global, &chunk) {
                Some(bytes) => bytes,
                None => chunk.to_utf8(global)?.into_bytes(),
            };
            let mut s = this.borrow_mut(global)?;
            if s.ended {
                return Ok(JSValue::js_boolean(true));
            }
            s.buffer.extend_from_slice(&bytes);
            Ok(JSValue::js_number(bytes.len() as f64))
        })?;
        class.method("flush", 0, attributes, |this, global, _| {
            if this.borrow(global)?.ended {
                return Ok(JSValue::js_boolean(true));
            }
            Ok(JSValue::js_number(flush(global, &this.to_js())? as f64))
        })?;
        class.method("end", 0, attributes, |this, global, _| end(global, &this.to_js()))?;
        class.method("close", 0, attributes, |this, global, _| end(global, &this.to_js()))?;
        class.method("start", 1, attributes, |_, _, _| Ok(JSValue::UNDEFINED))?;
        class.method("ref", 0, attributes, |_, _, _| Ok(JSValue::UNDEFINED))?;
        class.method("unref", 0, attributes, |_, _, _| Ok(JSValue::UNDEFINED))?;
        Ok(())
    }
}

/// What a stream of a descriptor reads from.
#[derive(Trace, Finalize)]
pub struct FileSource {
    #[unsafe_ignore_trace]
    fd: Fd,
    done: bool,
}

impl JsClass for FileSource {
    const NAME: &'static str = "FileSource";

    fn define(_: &mut ClassBuilder<'_, Self>) -> JsResult<()> {
        Ok(())
    }
}

/// The stream of what `fd` gives, a chunk each time a reader asks, until its end.
pub fn stream_of_fd(global: &JSGlobalObject, fd: Fd) -> JsResult<JSValue> {
    let source = bun_js::class::to_js(global, FileSource { fd, done: false })?;
    readable_stream::create(global, SourceAlgorithms::File { source }, 0.0)
}

const READ_CHUNK: usize = 64 * 1024;

/// The pull algorithm of a descriptor's stream: one read.
pub fn source_pull(global: &JSGlobalObject, stream_controller: &JSValue, source: &JSValue) -> JsResult<JSValue> {
    let (fd, done) = {
        let s = source.as_class_ref::<FileSource>().expect("a FileSource");
        let s = s.borrow(global)?;
        (s.fd, s.done)
    };
    if done {
        return Ok(internals::resolved(global, JSValue::UNDEFINED));
    }
    let mut chunk = vec![0u8; READ_CHUNK];
    let read = loop {
        match bun_sys::read(fd, &mut chunk) {
            Err(err) if err.get_errno() == bun_sys::E::EAGAIN => store::wait_for(fd, libc::POLLIN),
            other => break other,
        }
    };
    match read {
        Ok(0) => {
            source.as_class_ref::<FileSource>().expect("a FileSource").borrow_mut(global)?.done = true;
            controller::close(global, stream_controller)?;
        }
        Ok(len) => {
            let bytes = crate::engine::uint8_array(global, &chunk[..len])?;
            controller::enqueue(global, stream_controller, bytes)?;
        }
        Err(err) => {
            let error = SystemError::from(err).to_error_instance(global);
            return Ok(internals::rejected(global, error));
        }
    }
    Ok(internals::resolved(global, JSValue::UNDEFINED))
}
