//! Socket event dispatch (src/runtime/socket/uws_dispatch.rs) for the sockets of this build: the HTTP client
//! thread's, and those with a vtable on their group. uSockets (`bun_usockets_rs`) calls these `us_dispatch_*`.

use core::ffi::{c_int, c_void};

use bun_uws_sys::socket::NewSocketHandler;
use bun_uws_sys::socket_group::VTable;
use bun_uws_sys::vtable::{self, Handler as VHandler};
use bun_uws_sys::{ConnectingSocket, SocketKind, us_bun_verify_error_t, us_socket_t};

type HttpH<const SSL: bool> = bun_http::http_context::Handler<SSL>;

/// src/runtime/socket/uws_handlers.rs's `HTTPClient`.
struct HTTPClient<const SSL: bool>;

impl<const SSL: bool> VHandler for HTTPClient<SSL> {
    type Ext = ();
    const HAS_EXT: bool = false;

    const HAS_ON_OPEN: bool = true;
    const HAS_ON_DATA: bool = true;
    const HAS_ON_WRITABLE: bool = true;
    const HAS_ON_CLOSE: bool = true;
    const HAS_ON_TIMEOUT: bool = true;
    const HAS_ON_LONG_TIMEOUT: bool = true;
    const HAS_ON_END: bool = true;
    const HAS_ON_CONNECT_ERROR: bool = true;
    const HAS_ON_CONNECTING_ERROR: bool = true;
    const HAS_ON_HANDSHAKE: bool = true;

    fn on_open_no_ext(s: *mut us_socket_t, _is_client: bool, _ip: &[u8]) {
        HttpH::<SSL>::on_open(NewSocketHandler::<SSL>::from(s));
    }
    fn on_data_no_ext(s: *mut us_socket_t, data: &[u8]) {
        HttpH::<SSL>::on_data(NewSocketHandler::<SSL>::from(s), data);
    }
    fn on_writable_no_ext(s: *mut us_socket_t) {
        HttpH::<SSL>::on_writable(NewSocketHandler::<SSL>::from(s));
    }
    fn on_close_no_ext(s: *mut us_socket_t, code: i32, reason: Option<*mut c_void>) {
        HttpH::<SSL>::on_close(NewSocketHandler::<SSL>::from(s), code, reason);
    }
    fn on_timeout_no_ext(s: *mut us_socket_t) {
        HttpH::<SSL>::on_timeout(NewSocketHandler::<SSL>::from(s));
    }
    fn on_long_timeout_no_ext(s: *mut us_socket_t) {
        HttpH::<SSL>::on_long_timeout(NewSocketHandler::<SSL>::from(s));
    }
    fn on_end_no_ext(s: *mut us_socket_t) {
        HttpH::<SSL>::on_end(NewSocketHandler::<SSL>::from(s));
    }
    fn on_connect_error_no_ext(s: *mut us_socket_t, code: i32) {
        HttpH::<SSL>::on_connect_error(NewSocketHandler::<SSL>::from(s), code);
    }
    fn on_connecting_error(cs: *mut ConnectingSocket, code: i32) {
        HttpH::<SSL>::on_connect_error(NewSocketHandler::<SSL>::from_connecting(cs), code);
    }
    fn on_handshake_no_ext(s: *mut us_socket_t, ok: bool, err: us_bun_verify_error_t) {
        HttpH::<SSL>::on_handshake(NewSocketHandler::<SSL>::from(s), ok as i32, err);
    }
}

fn table(kind: SocketKind, group_vtable: impl FnOnce() -> Option<&'static VTable>) -> &'static VTable {
    match kind {
        SocketKind::HttpClient => vtable::make::<HTTPClient<false>>(),
        SocketKind::HttpClientTls => vtable::make::<HTTPClient<true>>(),
        SocketKind::Dynamic => group_vtable().expect("group vtable"),
        other => panic!("a {other:?} socket: only bun_runtime makes those"),
    }
}

fn vt(s: *mut us_socket_t) -> &'static VTable {
    let s = us_socket_t::opaque_mut(s);
    let kind = s.kind();
    table(kind, || s.raw_group().vtable.get())
}

fn vtc(c: *mut ConnectingSocket) -> &'static VTable {
    let c = ConnectingSocket::opaque_mut(c);
    let kind = c.kind();
    // SAFETY: a connecting socket with a kind has a group.
    table(kind, || unsafe { (*c.raw_group()).vtable.get() })
}

macro_rules! us_dispatch_shims {
    ($(
        fn $name:ident($recv:ident: *mut $Recv:ty $(, $a:ident: $t:ty)* $(,)?) -> $ret:ty
            = $lookup:ident.$field:ident($($call:expr),* $(,)?) or $default:expr;
    )*) => {$(
        /// # Safety
        /// uSockets passes a live socket, and buffers that are valid for the call.
        #[unsafe(no_mangle)]
        #[allow(clippy::unused_unit)]
        unsafe extern "C" fn $name($recv: *mut $Recv $(, $a: $t)*) -> $ret {
            match $lookup($recv).$field {
                // SAFETY: the vtable's callback for this kind, whose signature this shim has.
                Some(f) => unsafe { f($($call),*) },
                None => $default,
            }
        }
    )*};
}

us_dispatch_shims! {
    fn us_dispatch_open(s: *mut us_socket_t, is_client: c_int, ip: *mut u8, ip_len: c_int) -> *mut us_socket_t
        = vt.on_open(s, is_client, ip, ip_len) or s;
    fn us_dispatch_data(s: *mut us_socket_t, data: *mut u8, len: c_int) -> *mut us_socket_t
        = vt.on_data(s, data, len) or s;
    fn us_dispatch_fd(s: *mut us_socket_t, fd: c_int) -> *mut us_socket_t
        = vt.on_fd(s, fd) or s;
    fn us_dispatch_writable(s: *mut us_socket_t) -> *mut us_socket_t
        = vt.on_writable(s) or s;
    fn us_dispatch_close(s: *mut us_socket_t, code: c_int, reason: *mut c_void) -> *mut us_socket_t
        = vt.on_close(s, code, reason) or s;
    fn us_dispatch_timeout(s: *mut us_socket_t) -> *mut us_socket_t
        = vt.on_timeout(s) or s;
    fn us_dispatch_long_timeout(s: *mut us_socket_t) -> *mut us_socket_t
        = vt.on_long_timeout(s) or s;
    fn us_dispatch_end(s: *mut us_socket_t) -> *mut us_socket_t
        = vt.on_end(s) or s;
    fn us_dispatch_connect_error(s: *mut us_socket_t, code: c_int) -> *mut us_socket_t
        = vt.on_connect_error(s, code) or s;
    fn us_dispatch_connecting_error(c: *mut ConnectingSocket, code: c_int) -> *mut ConnectingSocket
        = vtc.on_connecting_error(c, code) or c;
    fn us_dispatch_handshake(s: *mut us_socket_t, ok: c_int, err: us_bun_verify_error_t) -> ()
        = vt.on_handshake(s, ok, err, core::ptr::null_mut()) or ();
}
