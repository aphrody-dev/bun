//! uSockets (packages/bun-usockets) in Rust, for the Boa build: the `us_*` C ABI that `bun_uws_sys` declares, with
//! the semantics of `loop.c`, `socket.c`, `context.cpp`, `bsd.cpp` and `eventing/epoll_kqueue.c`. Events go to the
//! `us_dispatch_*` exports of the binary that links this crate, as from the C library.
//!
//! What it has: the loop on epoll (wakeups, ready-poll dispatch, the timeout sweep, `Bun__internal_dispatch_ready_poll`
//! for `bun_io`'s polls), TCP and Unix-socket clients (connect, read, write, timeouts, shutdown, close), and name
//! resolution on a thread of its own. It has no listen sockets, UDP or QUIC (those entry points fail), and no kqueue.
#![cfg(any(target_os = "linux", target_os = "android"))]
#![allow(non_camel_case_types, clippy::missing_safety_doc)]

mod context;
mod dns;
mod eventing;
mod socket;
mod unsupported;

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::sync::atomic::AtomicU32;

pub const LIBUS_SOCKET_READABLE: c_int = libc::EPOLLIN;
pub const LIBUS_SOCKET_WRITABLE: c_int = libc::EPOLLOUT;

pub const LIBUS_RECV_BUFFER_LENGTH: usize = 524288;
pub const LIBUS_RECV_BUFFER_PADDING: usize = 32;
pub const LIBUS_TIMEOUT_GRANULARITY: i64 = 4;
pub const LIBUS_MAX_READY_POLLS: usize = 1024;

pub const LIBUS_SOCKET_CLOSE_CODE_CLEAN_SHUTDOWN: c_int = 0;
pub const LIBUS_SOCKET_CLOSE_CODE_CONNECTION_RESET: c_int = 1;
pub const LIBUS_SOCKET_CLOSE_CODE_FAST_SHUTDOWN: c_int = 2;

pub const LIBUS_SOCKET_ALLOW_HALF_OPEN: c_int = 2;

pub const LIBUS_QUEUED_INPUT_NONE: c_int = 0;
pub const LIBUS_QUEUED_INPUT_DATA: c_int = 1;
pub const LIBUS_QUEUED_INPUT_EOF: c_int = 2;
pub const LIBUS_QUEUED_INPUT_ERROR: c_int = 3;

/// `eof` hints of `us_internal_dispatch_ready_poll`.
pub const LIBUS_POLL_EOF: c_int = 1;
pub const LIBUS_POLL_HANGUP: c_int = 2;

pub(crate) const POLL_TYPE_SOCKET: u8 = 0;
pub(crate) const POLL_TYPE_SOCKET_SHUT_DOWN: u8 = 1;
pub(crate) const POLL_TYPE_SEMI_SOCKET: u8 = 2;
pub(crate) const POLL_TYPE_CALLBACK: u8 = 3;
pub(crate) const POLL_TYPE_POLLING_OUT: u8 = 8;
pub(crate) const POLL_TYPE_POLLING_IN: u8 = 16;
pub(crate) const POLL_TYPE_KIND_MASK: u8 = 0b111;
pub(crate) const POLL_TYPE_POLLING_MASK: u8 = 0b11000;

/// `struct us_poll_t`: what every entry of the loop's epoll set points at, unless `bun_io` tagged the pointer.
#[repr(C, align(16))]
pub struct us_poll_t {
    pub(crate) fd: c_int,
    pub(crate) poll_type: u8,
}

/// `struct us_internal_loop_data_t` (internal/loop_data.h), which `bun_uws_sys::InternalLoopData` mirrors.
#[repr(C)]
pub struct us_internal_loop_data_t {
    pub(crate) sweep_next_tick_ns: i64,
    pub(crate) sweep_timer_count: c_int,
    pub(crate) wakeup_async: *mut eventing::us_internal_async,
    pub(crate) head: *mut us_socket_group_t,
    pub(crate) quic_drivers: *mut c_void,
    pub(crate) quic_next_tick_us: i64,
    pub(crate) iterator: *mut us_socket_group_t,
    pub(crate) recv_buf: *mut u8,
    pub(crate) send_buf: *mut u8,
    pub(crate) ssl_data: *mut c_void,
    pub(crate) pre_cb: Option<LoopCb>,
    pub(crate) post_cb: Option<LoopCb>,
    pub(crate) closed_udp_head: *mut c_void,
    pub(crate) closed_head: *mut us_socket_t,
    pub(crate) low_prio_head: *mut us_socket_t,
    pub(crate) low_prio_iterator: *mut us_socket_t,
    pub(crate) low_prio_budget: c_int,
    pub(crate) dns_ready_head: *mut us_connecting_socket_t,
    pub(crate) closed_connecting_head: *mut us_connecting_socket_t,
    pub(crate) mutex: u32,
    pub(crate) parent_ptr: *mut c_void,
    pub(crate) parent_tag: c_char,
    pub(crate) iteration_nr: usize,
    pub(crate) jsc_vm: *mut c_void,
    pub(crate) tick_depth: c_int,
    pub(crate) num_unresumable_sockets: c_int,
    pub(crate) unresumable_sockets: *mut c_void,
}

#[repr(C, align(16))]
pub(crate) struct ReadyPolls(pub(crate) [libc::epoll_event; LIBUS_MAX_READY_POLLS]);

/// `struct us_loop_t` of eventing/epoll_kqueue.h, which `bun_uws_sys::Loop` mirrors.
#[repr(C, align(16))]
pub struct us_loop_t {
    pub(crate) data: us_internal_loop_data_t,
    pub(crate) num_polls: c_int,
    pub(crate) num_ready_polls: c_int,
    pub(crate) current_ready_poll: c_int,
    pub(crate) fd: c_int,
    pub(crate) bun_polls: c_uint,
    pub(crate) pending_wakeups: AtomicU32,
    pub(crate) ready_polls: ReadyPolls,
}

pub(crate) type LoopCb = unsafe extern "C" fn(*mut us_loop_t);

/// `struct us_socket_group_t` of libusockets.h, which `bun_uws_sys::SocketGroup` mirrors. Its owner embeds it.
#[repr(C)]
pub struct us_socket_group_t {
    pub(crate) loop_: *mut us_loop_t,
    pub(crate) vtable: *const c_void,
    pub(crate) ext: *mut c_void,
    pub(crate) head_sockets: *mut us_socket_t,
    pub(crate) head_connecting_sockets: *mut us_connecting_socket_t,
    pub(crate) head_listen_sockets: *mut c_void,
    pub(crate) iterator: *mut us_socket_t,
    pub(crate) prev: *mut us_socket_group_t,
    pub(crate) next: *mut us_socket_group_t,
    pub(crate) global_tick: u32,
    pub(crate) low_prio_count: u16,
    pub(crate) timestamp: u8,
    pub(crate) long_timestamp: u8,
    pub(crate) linked: u8,
    _padding: [u8; 7],
}

/// `struct us_socket_t`: a poll, its state, and the ext of `socket_ext_size` bytes after it.
#[repr(C, align(16))]
pub struct us_socket_t {
    pub(crate) p: us_poll_t,
    pub(crate) timeout: u8,
    pub(crate) long_timeout: u8,
    pub(crate) kind: u8,
    pub(crate) is_paused: bool,
    pub(crate) allow_half_open: bool,
    pub(crate) is_closed: bool,
    pub(crate) last_write_failed: bool,
    pub(crate) read_eof: bool,
    pub(crate) send_rejected: bool,
    pub(crate) unclassified_send_failures: u8,
    pub(crate) group: *mut us_socket_group_t,
    pub(crate) prev: *mut us_socket_t,
    pub(crate) next: *mut us_socket_t,
    pub(crate) connect_next: *mut us_socket_t,
    pub(crate) connect_state: *mut us_connecting_socket_t,
    /// What `alloc` was asked for, for `dealloc`.
    pub(crate) alloc_size: usize,
}

/// `struct us_connecting_socket_t`: a connect that waits for its name to resolve, then races up to four addresses.
#[repr(C, align(16))]
pub struct us_connecting_socket_t {
    pub(crate) group: *mut us_socket_group_t,
    pub(crate) loop_: *mut us_loop_t,
    pub(crate) next: *mut us_connecting_socket_t,
    pub(crate) connecting_head: *mut us_socket_t,
    pub(crate) next_pending: *mut us_connecting_socket_t,
    pub(crate) prev_pending: *mut us_connecting_socket_t,
    pub(crate) options: c_int,
    pub(crate) socket_ext_size: c_int,
    pub(crate) closed: bool,
    pub(crate) shutdown: bool,
    pub(crate) shutdown_read: bool,
    pub(crate) pending_resolve_callback: bool,
    pub(crate) error_is_dns: bool,
    pub(crate) timeout: u8,
    pub(crate) long_timeout: u8,
    pub(crate) kind: u8,
    pub(crate) port: u16,
    pub(crate) error: c_int,
    /// What the resolver thread found, written before it queues this on `dns_ready_head`.
    /// A `Box<dns::Resolved>`, erased: C sees this struct.
    pub(crate) resolved: *mut c_void,
    /// The addresses not tried yet, from `resolved`.
    pub(crate) next_address: usize,
    pub(crate) alloc_size: usize,
}

/// `struct us_bun_verify_error_t`.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct us_bun_verify_error_t {
    pub error: c_int,
    pub code: *const c_char,
    pub reason: *const c_char,
}

const _: () = {
    use core::mem::{align_of, offset_of, size_of};
    assert!(size_of::<us_internal_loop_data_t>() == size_of::<bun_uws_sys::InternalLoopData>());
    assert!(size_of::<us_loop_t>() == size_of::<bun_uws_sys::Loop>());
    assert!(align_of::<us_loop_t>() == align_of::<bun_uws_sys::Loop>());
    assert!(offset_of!(us_loop_t, num_polls) == offset_of!(bun_uws_sys::Loop, num_polls));
    assert!(offset_of!(us_internal_loop_data_t, iterator) == offset_of!(bun_uws_sys::InternalLoopData, iterator));
    assert!(offset_of!(us_internal_loop_data_t, jsc_vm) == offset_of!(bun_uws_sys::InternalLoopData, jsc_vm));
    assert!(size_of::<us_socket_group_t>() == size_of::<bun_uws_sys::SocketGroup>());
    assert!(offset_of!(us_socket_group_t, head_sockets) == offset_of!(bun_uws_sys::SocketGroup, head_sockets));
    assert!(offset_of!(us_socket_group_t, linked) == offset_of!(bun_uws_sys::SocketGroup, linked));
    assert!(size_of::<us_socket_t>() % 16 == 0 && size_of::<us_connecting_socket_t>() % 16 == 0);
};

// Event dispatch, defined by the binary (in Bun, src/runtime/socket/uws_dispatch.rs). The loop never reads a vtable.
unsafe extern "C" {
    pub(crate) fn us_dispatch_open(s: *mut us_socket_t, is_client: c_int, ip: *mut c_char, ip_length: c_int) -> *mut us_socket_t;
    pub(crate) fn us_dispatch_data(s: *mut us_socket_t, data: *mut c_char, length: c_int) -> *mut us_socket_t;
    pub(crate) fn us_dispatch_writable(s: *mut us_socket_t) -> *mut us_socket_t;
    pub(crate) fn us_dispatch_close(s: *mut us_socket_t, code: c_int, reason: *mut c_void) -> *mut us_socket_t;
    pub(crate) fn us_dispatch_timeout(s: *mut us_socket_t) -> *mut us_socket_t;
    pub(crate) fn us_dispatch_long_timeout(s: *mut us_socket_t) -> *mut us_socket_t;
    pub(crate) fn us_dispatch_end(s: *mut us_socket_t) -> *mut us_socket_t;
    pub(crate) fn us_dispatch_connect_error(s: *mut us_socket_t, code: c_int) -> *mut us_socket_t;
    pub(crate) fn us_dispatch_connecting_error(c: *mut us_connecting_socket_t, code: c_int) -> *mut us_connecting_socket_t;

    /// `bun_io`'s polls, whose pointers in the epoll set are tagged.
    pub(crate) fn Bun__internal_dispatch_ready_poll(loop_: *mut us_loop_t, poll: *mut c_void);

    /// `bun_threading`'s lock, on `us_internal_loop_data_t::mutex`.
    pub(crate) fn Bun__lock(lock: *mut u32);
    pub(crate) fn Bun__unlock(lock: *mut u32);
}

pub(crate) fn errno() -> c_int {
    // SAFETY: the calling thread's errno.
    unsafe { *libc::__errno_location() }
}

pub(crate) fn set_errno(value: c_int) {
    // SAFETY: as above.
    unsafe { *libc::__errno_location() = value };
}

/// `us_xcalloc`: zeroed, aligned to `LIBUS_EXT_ALIGNMENT`, or the process stops as out of memory.
pub(crate) fn xcalloc(size: usize) -> *mut u8 {
    let layout = core::alloc::Layout::from_size_align(size.max(16), 16).expect("allocation size");
    // SAFETY: the layout has a nonzero size.
    let p = unsafe { std::alloc::alloc_zeroed(layout) };
    if p.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    p
}

/// # Safety
/// `p` and `size` are those of an `xcalloc`.
pub(crate) unsafe fn xfree(p: *mut u8, size: usize) {
    let layout = core::alloc::Layout::from_size_align(size.max(16), 16).expect("allocation size");
    // SAFETY: the contract of this function.
    unsafe { std::alloc::dealloc(p, layout) };
}
