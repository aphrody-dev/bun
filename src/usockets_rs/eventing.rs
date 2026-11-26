//! The loop: eventing/epoll_kqueue.c and the loop half of loop.c.

use core::ffi::{c_int, c_uint, c_void};
use core::ptr;
use core::sync::atomic::Ordering;

use crate::*;

/// `struct us_internal_callback_t` of the loop's wakeup: an eventfd, edge-triggered.
#[repr(C, align(16))]
pub struct us_internal_async {
    p: us_poll_t,
    loop_: *mut us_loop_t,
    cb: Option<LoopCb>,
}

const UNSET_BITS_49_UNTIL_64: u64 = 0x0000_FFFF_FFFF_FFFF;
/// The kernel's `sigset_t` (`_NSIG / 8`), not glibc's.
const KERNEL_SIGSET_SIZE: usize = 8;
const LIBUS_TIMEOUT_GRANULARITY_NS: i64 = LIBUS_TIMEOUT_GRANULARITY * 1_000_000_000;

pub(crate) fn us_internal_monotonic_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a timespec to write.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

// ── polls ──────────────────────────────────────────────────────────────────

pub(crate) unsafe fn us_poll_events(p: *const us_poll_t) -> c_int {
    // SAFETY: the caller passes a live poll.
    let t = unsafe { (*p).poll_type };
    (if t & POLL_TYPE_POLLING_IN != 0 { LIBUS_SOCKET_READABLE } else { 0 })
        | (if t & POLL_TYPE_POLLING_OUT != 0 { LIBUS_SOCKET_WRITABLE } else { 0 })
}

pub(crate) unsafe fn us_internal_poll_type(p: *const us_poll_t) -> u8 {
    // SAFETY: the caller passes a live poll.
    unsafe { (*p).poll_type & POLL_TYPE_KIND_MASK }
}

pub(crate) unsafe fn us_internal_poll_set_type(p: *mut us_poll_t, poll_type: u8) {
    // SAFETY: the caller passes a live poll.
    unsafe { (*p).poll_type = poll_type | ((*p).poll_type & POLL_TYPE_POLLING_MASK) };
}

fn polling_bits(events: c_int) -> u8 {
    (if events & LIBUS_SOCKET_READABLE != 0 { POLL_TYPE_POLLING_IN } else { 0 })
        | (if events & LIBUS_SOCKET_WRITABLE != 0 { POLL_TYPE_POLLING_OUT } else { 0 })
}

fn epoll_events(events: c_int) -> u32 {
    let mut e = events as u32;
    if events & (LIBUS_SOCKET_READABLE | LIBUS_SOCKET_WRITABLE) == 0 {
        // Neither direction: EPOLLHUP and EPOLLERR still report, which is what closes a half-open socket.
        e |= (libc::EPOLLHUP | libc::EPOLLERR) as u32;
    }
    e
}

fn epoll_ctl(epfd: c_int, op: c_int, fd: c_int, event: &mut libc::epoll_event) -> c_int {
    loop {
        // SAFETY: `event` is an epoll_event to read.
        let rc = unsafe { libc::epoll_ctl(epfd, op, fd, event) };
        if rc == -1 && errno() == libc::EINTR {
            continue;
        }
        return rc;
    }
}

/// Counts in `num_polls` unless `fallthrough`.
pub(crate) unsafe fn us_create_poll(loop_: *mut us_loop_t, fallthrough: bool, size: usize) -> *mut u8 {
    if !fallthrough {
        // SAFETY: the caller passes a live loop.
        unsafe { (*loop_).num_polls += 1 };
    }
    xcalloc(size)
}

pub(crate) unsafe fn us_poll_free(p: *mut u8, size: usize, loop_: *mut us_loop_t) {
    // SAFETY: the caller passes what `us_create_poll` gave it, and its loop.
    unsafe {
        (*loop_).num_polls -= 1;
        xfree(p, size);
    }
}

pub(crate) unsafe fn us_poll_start_rc(p: *mut us_poll_t, loop_: *mut us_loop_t, events: c_int) -> c_int {
    // SAFETY: the caller passes a live poll and its loop.
    unsafe {
        (*p).poll_type = us_internal_poll_type(p) | polling_bits(events);
        let mut event = libc::epoll_event { events: epoll_events(events), u64: p as u64 };
        epoll_ctl((*loop_).fd, libc::EPOLL_CTL_ADD, (*p).fd, &mut event)
    }
}

pub(crate) unsafe fn us_poll_change(p: *mut us_poll_t, loop_: *mut us_loop_t, events: c_int) -> c_int {
    // SAFETY: the caller passes a live poll and its loop.
    unsafe {
        let old_events = us_poll_events(p);
        if old_events == events {
            return 0;
        }
        (*p).poll_type = us_internal_poll_type(p) | polling_bits(events);
        let mut event = libc::epoll_event { events: epoll_events(events), u64: p as u64 };
        let mut rc = epoll_ctl((*loop_).fd, libc::EPOLL_CTL_MOD, (*p).fd, &mut event);
        // A paused socket that hung up left epoll; this is its resume coming back.
        if rc == -1 && errno() == libc::ENOENT {
            rc = us_poll_start_rc(p, loop_, events);
        }
        rc
    }
}

pub(crate) unsafe fn us_poll_stop(p: *mut us_poll_t, loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live poll and its loop.
    unsafe {
        let mut event = libc::epoll_event { events: 0, u64: 0 };
        epoll_ctl((*loop_).fd, libc::EPOLL_CTL_DEL, (*p).fd, &mut event);
        us_internal_loop_update_pending_ready_polls(loop_, p.cast(), ptr::null_mut());
    }
}

/// Points what is left of this iteration's ready polls at `new_poll` instead of `old_poll` (null: skip it).
pub(crate) unsafe fn us_internal_loop_update_pending_ready_polls(loop_: *mut us_loop_t, old_poll: *mut c_void, new_poll: *mut c_void) {
    // SAFETY: the caller passes a live loop.
    unsafe {
        let start = (*loop_).current_ready_poll.max(0) as usize;
        let end = (*loop_).num_ready_polls.max(0) as usize;
        for i in start..end {
            if (*loop_).ready_polls.0[i].u64 == old_poll as u64 {
                (*loop_).ready_polls.0[i].u64 = new_poll as u64;
                break;
            }
        }
    }
}

// ── the loop ───────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_create_loop(
    _hint: *mut c_void,
    wakeup_cb: Option<LoopCb>,
    pre_cb: Option<LoopCb>,
    post_cb: Option<LoopCb>,
    ext_size: c_uint,
) -> *mut us_loop_t {
    let size = core::mem::size_of::<us_loop_t>() + ext_size as usize;
    let loop_ = xcalloc(size).cast::<us_loop_t>();
    // SAFETY: `loop_` is zeroed memory for a loop, which zero is a valid state of.
    unsafe {
        (*loop_).fd = libc::epoll_create1(libc::EPOLL_CLOEXEC);
        if (*loop_).fd == -1 {
            xfree(loop_.cast(), size);
            return ptr::null_mut();
        }
        let data = &mut (*loop_).data;
        data.sweep_next_tick_ns = -1;
        data.recv_buf = xcalloc(LIBUS_RECV_BUFFER_LENGTH + LIBUS_RECV_BUFFER_PADDING * 2);
        data.pre_cb = pre_cb;
        data.post_cb = post_cb;
        let efd = libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC);
        if efd == -1 {
            xfree(data.recv_buf, LIBUS_RECV_BUFFER_LENGTH + LIBUS_RECV_BUFFER_PADDING * 2);
            libc::close((*loop_).fd);
            xfree(loop_.cast(), size);
            return ptr::null_mut();
        }
        let a = us_create_poll(loop_, true, core::mem::size_of::<us_internal_async>()).cast::<us_internal_async>();
        (*a).p.fd = efd;
        (*a).p.poll_type = POLL_TYPE_CALLBACK;
        (*a).loop_ = loop_;
        (*a).cb = wakeup_cb;
        (*a).p.poll_type |= POLL_TYPE_POLLING_IN;
        let mut event = libc::epoll_event { events: (libc::EPOLLIN | libc::EPOLLET) as u32, u64: a as u64 };
        epoll_ctl((*loop_).fd, libc::EPOLL_CTL_ADD, efd, &mut event);
        data.wakeup_async = a;
    }
    loop_
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_loop_free(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a loop from `us_create_loop` that nothing uses any more.
    unsafe {
        us_internal_free_closed_sockets(loop_);
        let a = (*loop_).data.wakeup_async;
        libc::close((*a).p.fd);
        xfree(a.cast(), core::mem::size_of::<us_internal_async>());
        xfree((*loop_).data.recv_buf, LIBUS_RECV_BUFFER_LENGTH + LIBUS_RECV_BUFFER_PADDING * 2);
        libc::close((*loop_).fd);
        xfree(loop_.cast(), core::mem::size_of::<us_loop_t>());
    }
}

thread_local! {
    static THREAD_LOOP: core::cell::Cell<*mut us_loop_t> = const { core::cell::Cell::new(ptr::null_mut()) };
}

unsafe extern "C" fn noop(_: *mut us_loop_t) {}

/// The thread's loop, made on first use (libuwsockets.cpp's `uWS::Loop::get()`; uWS's own pre/post hooks have no
/// users here).
#[unsafe(no_mangle)]
pub extern "C" fn uws_get_loop() -> *mut us_loop_t {
    THREAD_LOOP.with(|cell| {
        if cell.get().is_null() {
            // SAFETY: no hint, no ext.
            let loop_ = unsafe { us_create_loop(ptr::null_mut(), Some(noop), Some(noop), Some(noop), 0) };
            assert!(!loop_.is_null(), "uws_get_loop: epoll_create1 or eventfd failed (errno {})", errno());
            cell.set(loop_);
        }
        cell.get()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_wakeup_loop(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop; any thread may.
    unsafe {
        (*loop_).pending_wakeups.fetch_add(1, Ordering::Release);
        let fd = (*(*loop_).data.wakeup_async).p.fd;
        loop {
            let val: u64 = 1;
            if libc::write(fd, (&raw const val).cast(), 8) >= 0 {
                return;
            }
            match errno() {
                libc::EINTR => continue,
                libc::EAGAIN => {
                    let mut drained: u64 = 0;
                    if libc::read(fd, (&raw mut drained).cast(), 8) > 0 || matches!(errno(), libc::EAGAIN | libc::EINTR) {
                        continue;
                    }
                    return;
                }
                _ => return,
            }
        }
    }
}

// ── the sweep ──────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_internal_enable_sweep_timer(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop.
    unsafe {
        (*loop_).data.sweep_timer_count += 1;
        if (*loop_).data.sweep_timer_count == 1 {
            (*loop_).data.sweep_next_tick_ns = us_internal_monotonic_ns() as i64 + LIBUS_TIMEOUT_GRANULARITY_NS;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_internal_disable_sweep_timer(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop.
    unsafe {
        (*loop_).data.sweep_timer_count -= 1;
        if (*loop_).data.sweep_timer_count == 0 {
            (*loop_).data.sweep_next_tick_ns = -1;
        }
    }
}

unsafe fn us_internal_sweep_timeout_ns(loop_: *mut us_loop_t) -> i64 {
    // SAFETY: the caller passes a live loop.
    let next = unsafe { (*loop_).data.sweep_next_tick_ns };
    if next < 0 {
        return -1;
    }
    (next - us_internal_monotonic_ns() as i64).max(0)
}

unsafe fn us_internal_sweep_if_due(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop.
    unsafe {
        if (*loop_).data.sweep_next_tick_ns < 0 {
            return;
        }
        let now = us_internal_monotonic_ns() as i64;
        if now < (*loop_).data.sweep_next_tick_ns {
            return;
        }
        // Re-armed first: a timeout handler may unlink the last socket and disarm it.
        (*loop_).data.sweep_next_tick_ns = now + LIBUS_TIMEOUT_GRANULARITY_NS;
        us_internal_timer_sweep(loop_);
    }
}

/// Emits the timeouts that are due: each group's tick advances, and a socket whose stamp it reaches times out.
unsafe fn us_internal_timer_sweep(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop; handlers may unlink sockets and groups, which the iterators follow.
    unsafe {
        let data = &raw mut (*loop_).data;
        (*data).iterator = (*data).head;
        while !(*data).iterator.is_null() {
            let group = (*data).iterator;
            (*group).global_tick = (*group).global_tick.wrapping_add(1);
            let short_ticks = ((*group).global_tick % 240) as u8;
            let long_ticks = (((*group).global_tick / 15) % 240) as u8;
            (*group).timestamp = short_ticks;
            (*group).long_timestamp = long_ticks;

            let mut s = (*group).head_sockets;
            let mut group_survived = true;
            while !s.is_null() {
                if short_ticks != (*s).timeout && long_ticks != (*s).long_timeout {
                    s = (*s).next;
                    continue;
                }
                (*group).iterator = s;
                if short_ticks == (*s).timeout {
                    (*s).timeout = 255;
                    us_dispatch_timeout(s);
                }
                if (*data).iterator != group {
                    group_survived = false;
                    break;
                }
                if (*group).iterator == s && long_ticks == (*s).long_timeout {
                    (*s).long_timeout = 255;
                    us_dispatch_long_timeout(s);
                }
                if (*data).iterator != group {
                    group_survived = false;
                    break;
                }
                s = if s == (*group).iterator { (*s).next } else { (*group).iterator };
            }
            if group_survived {
                (*group).iterator = ptr::null_mut();
                (*data).iterator = (*group).next;
            }
        }
    }
}

// ── pre, post, and freeing what closed ─────────────────────────────────────

unsafe fn us_internal_handle_dns_results(loop_: *mut us_loop_t) -> bool {
    // SAFETY: the caller passes a live loop; the list is shared with resolver threads under the loop's mutex.
    unsafe {
        let mutex = &raw mut (*loop_).data.mutex;
        Bun__lock(mutex);
        let mut c = (*loop_).data.dns_ready_head;
        (*loop_).data.dns_ready_head = ptr::null_mut();
        Bun__unlock(mutex);
        let any = !c.is_null();
        while !c.is_null() {
            let next = (*c).next;
            crate::context::us_internal_socket_after_resolve(c);
            c = next;
        }
        any
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_internal_free_closed_sockets(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop; what is on its closed lists is freed nowhere else.
    unsafe {
        let mut s = (*loop_).data.closed_head;
        while !s.is_null() {
            let next = (*s).next;
            let size = (*s).alloc_size;
            us_poll_free(s.cast(), size, loop_);
            s = next;
        }
        (*loop_).data.closed_head = ptr::null_mut();

        let mut c = (*loop_).data.closed_connecting_head;
        while !c.is_null() {
            let next = (*c).next;
            crate::context::us_connecting_socket_dealloc(c);
            c = next;
        }
        (*loop_).data.closed_connecting_head = ptr::null_mut();
    }
}

unsafe fn us_internal_loop_pre(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop.
    unsafe {
        (*loop_).data.iteration_nr = (*loop_).data.iteration_nr.wrapping_add(1);
        us_internal_handle_dns_results(loop_);
        if let Some(pre) = (*loop_).data.pre_cb {
            pre(loop_);
        }
    }
}

unsafe fn us_internal_loop_post(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop.
    unsafe {
        us_internal_handle_dns_results(loop_);
        // A nested tick must not free what the outer dispatch may still read.
        if (*loop_).data.tick_depth <= 1 {
            us_internal_free_closed_sockets(loop_);
        }
        if let Some(post) = (*loop_).data.post_cb {
            post(loop_);
        }
    }
}

// ── waiting and dispatching ────────────────────────────────────────────────

/// epoll_pwait2 (ns timeouts), or epoll_pwait with milliseconds rounded up where the kernel lacks it. EINTR waits
/// for the rest of the time.
unsafe fn bun_epoll_pwait2(loop_: *mut us_loop_t, timeout: Option<libc::timespec>) -> c_int {
    static HAS_EPOLL_PWAIT2: core::sync::atomic::AtomicI8 = core::sync::atomic::AtomicI8::new(-1);
    let deadline = timeout
        .filter(|t| t.tv_sec != 0 || t.tv_nsec != 0)
        .map(|t| us_internal_monotonic_ns() + t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64);
    let mut remaining = timeout;
    // SAFETY: the caller passes a live loop; the kernel writes at most LIBUS_MAX_READY_POLLS events.
    unsafe {
        let events = (*loop_).ready_polls.0.as_mut_ptr();
        loop {
            let ret = if HAS_EPOLL_PWAIT2.load(Ordering::Relaxed) != 0 {
                let mut mask: libc::sigset_t = core::mem::zeroed();
                libc::sigemptyset(&mut mask);
                let ts = remaining.as_ref().map_or(ptr::null(), |t| t as *const libc::timespec);
                let r = libc::syscall(
                    libc::SYS_epoll_pwait2,
                    (*loop_).fd,
                    events,
                    LIBUS_MAX_READY_POLLS as c_int,
                    ts,
                    &mask as *const libc::sigset_t,
                    KERNEL_SIGSET_SIZE,
                ) as c_int;
                if r == -1 && matches!(errno(), libc::ENOSYS | libc::EPERM | libc::EOPNOTSUPP | libc::EACCES | libc::EFAULT) {
                    HAS_EPOLL_PWAIT2.store(0, Ordering::Relaxed);
                    continue;
                }
                r
            } else {
                let ms = remaining.map_or(-1, |t| {
                    let ns = t.tv_sec as i128 * 1_000_000_000 + t.tv_nsec as i128;
                    ((ns.max(0) + 999_999) / 1_000_000).min(i32::MAX as i128) as c_int
                });
                libc::epoll_wait((*loop_).fd, events, LIBUS_MAX_READY_POLLS as c_int, ms)
            };
            if ret != -1 || errno() != libc::EINTR {
                return ret.max(0);
            }
            if let Some(deadline) = deadline {
                let now = us_internal_monotonic_ns();
                if now >= deadline {
                    return 0;
                }
                let left = deadline - now;
                remaining = Some(libc::timespec { tv_sec: (left / 1_000_000_000) as _, tv_nsec: (left % 1_000_000_000) as _ });
            }
        }
    }
}

unsafe fn us_internal_dispatch_ready_polls(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes a live loop whose ready polls were just filled; stopping a poll scrubs it from them.
    unsafe {
        (*loop_).current_ready_poll = 0;
        while (*loop_).current_ready_poll < (*loop_).num_ready_polls {
            let ready = (*loop_).ready_polls.0[(*loop_).current_ready_poll as usize];
            let poll = ready.u64;
            if poll != 0 {
                if poll & UNSET_BITS_49_UNTIL_64 != poll {
                    Bun__internal_dispatch_ready_poll(loop_, poll as *mut c_void);
                } else {
                    let p = poll as *mut us_poll_t;
                    let mut events = ready.events as c_int;
                    let error = c_int::from(events & libc::EPOLLERR != 0);
                    let eof = if events & libc::EPOLLHUP != 0 { LIBUS_POLL_HANGUP } else { 0 };
                    events &= us_poll_events(p);
                    if events != 0 || error != 0 || eof != 0 {
                        us_internal_dispatch_ready_poll(p, error, eof, events);
                    }
                }
            }
            (*loop_).current_ready_poll += 1;
        }
    }
}

unsafe fn us_internal_dispatch_ready_poll(p: *mut us_poll_t, error: c_int, eof: c_int, events: c_int) {
    // SAFETY: the caller passes a live poll of this crate.
    unsafe {
        match us_internal_poll_type(p) {
            POLL_TYPE_CALLBACK => {
                let a = p.cast::<us_internal_async>();
                if let Some(cb) = (*a).cb {
                    cb((*a).loop_);
                }
            }
            POLL_TYPE_SEMI_SOCKET => {
                if us_poll_events(p) & LIBUS_SOCKET_WRITABLE != 0 {
                    let s = p.cast::<us_socket_t>();
                    let mut connect_error = 0;
                    if error != 0 || eof != 0 {
                        connect_error = crate::socket::us_socket_get_error(s);
                        if connect_error == 0 {
                            connect_error = libc::ECONNRESET;
                        }
                    }
                    crate::context::us_internal_socket_after_open(s, connect_error);
                } else {
                    crate::listen::on_readable(p.cast());
                }
            }
            _ => crate::socket::dispatch_socket(p.cast(), error, eof, events),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_loop_run_bun_tick(loop_: *mut us_loop_t, timeout: *const libc::timespec, _now_ns: u64) {
    // SAFETY: the caller passes its thread's live loop, and null or a timespec.
    unsafe {
        if (*loop_).num_polls == 0 {
            return;
        }
        (*loop_).data.tick_depth += 1;
        us_internal_loop_pre(loop_);

        let mut timeout = timeout.as_ref().copied();
        let sweep_ns = us_internal_sweep_timeout_ns(loop_);
        if sweep_ns >= 0 {
            let sweep = libc::timespec { tv_sec: (sweep_ns / 1_000_000_000) as _, tv_nsec: (sweep_ns % 1_000_000_000) as _ };
            let sooner = timeout.is_none_or(|t| (t.tv_sec, t.tv_nsec) > (sweep.tv_sec, sweep.tv_nsec));
            if sooner {
                timeout = Some(sweep);
            }
        }
        (*loop_).pending_wakeups.swap(0, Ordering::Acquire);

        (*loop_).num_ready_polls = bun_epoll_pwait2(loop_, timeout);
        (*loop_).current_ready_poll = 0;
        us_internal_dispatch_ready_polls(loop_);
        // A full batch may have left more ready: take them without waiting.
        let mut drain_count = 48;
        while (*loop_).num_ready_polls == LIBUS_MAX_READY_POLLS as c_int && drain_count > 1 && (*loop_).num_polls > 0 {
            drain_count -= 1;
            (*loop_).num_ready_polls = bun_epoll_pwait2(loop_, Some(libc::timespec { tv_sec: 0, tv_nsec: 0 }));
            us_internal_dispatch_ready_polls(loop_);
        }
        us_internal_sweep_if_due(loop_);

        us_internal_loop_post(loop_);
        (*loop_).data.tick_depth -= 1;
    }
}

/// `us_loop_run`: ticks until nothing keeps the loop alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_loop_run(loop_: *mut us_loop_t) {
    // SAFETY: the caller passes its thread's live loop.
    unsafe {
        while (*loop_).num_polls > 0 {
            us_loop_run_bun_tick(loop_, ptr::null(), 0);
        }
    }
}

pub(crate) unsafe fn loop_link_group(loop_: *mut us_loop_t, group: *mut us_socket_group_t) {
    // SAFETY: the caller passes a live loop and an unlinked group.
    unsafe {
        (*group).next = (*loop_).data.head;
        (*group).prev = ptr::null_mut();
        if !(*loop_).data.head.is_null() {
            (*(*loop_).data.head).prev = group;
        }
        (*loop_).data.head = group;
    }
}

pub(crate) unsafe fn loop_unlink_group(loop_: *mut us_loop_t, group: *mut us_socket_group_t) {
    // SAFETY: the caller passes a live loop and a group linked into it.
    unsafe {
        if group == (*loop_).data.iterator {
            (*loop_).data.iterator = (*group).next;
        }
        if (*loop_).data.head == group {
            (*loop_).data.head = (*group).next;
            if !(*loop_).data.head.is_null() {
                (*(*loop_).data.head).prev = ptr::null_mut();
            }
        } else {
            (*(*group).prev).next = (*group).next;
            if !(*group).next.is_null() {
                (*(*group).next).prev = (*group).prev;
            }
        }
    }
}

/// Closes every socket of every group on the loop; 1 if there was any.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_loop_close_all_groups(loop_: *mut us_loop_t) -> c_int {
    // SAFETY: the caller passes a live loop; closing can unlink groups, so `next` is re-read when it was.
    unsafe {
        let mut g = (*loop_).data.head;
        let mut any = 0;
        while !g.is_null() {
            let mut next = (*g).next;
            if !(*g).head_sockets.is_null() || !(*g).head_connecting_sockets.is_null() {
                crate::context::group_close_all(g, false);
                any = 1;
            }
            if !next.is_null() && (*next).linked == 0 {
                next = (*loop_).data.head;
            }
            g = next;
        }
        any
    }
}
