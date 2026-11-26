//! Name resolution for connects (`Bun__addrinfo_*` in Bun, a cache over getaddrinfo on a work pool): one
//! getaddrinfo per connect, on a thread of its own, without a cache. The result goes to the loop as loop.c's
//! `us_internal_dns_callback_threadsafe` does: on `dns_ready_head` under the loop's mutex, then a wakeup.

use core::ffi::{CStr, c_int};
use std::ffi::CString;
use std::net::SocketAddr;

use crate::*;

/// Addresses in getaddrinfo's order, or its `EAI_*` error.
pub(crate) type Resolved = Result<Vec<SocketAddr>, c_int>;

struct Send(*mut us_connecting_socket_t);
// SAFETY: the resolver thread only writes `resolved` and links the socket into its loop's list, under the mutex.
unsafe impl core::marker::Send for Send {}

/// # Safety
/// `c` is a live connecting socket.
pub(crate) unsafe fn resolved_of<'a>(c: *mut us_connecting_socket_t) -> Option<&'a Resolved> {
    // SAFETY: the caller's socket; `resolved` is null or a `Box<Resolved>` from `resolve_for`.
    unsafe { (*c).resolved.cast::<Resolved>().as_ref() }
}

pub(crate) fn getaddrinfo(host: &CStr, port: u16) -> Resolved {
    // SAFETY: an all-zero addrinfo is the empty hints of getaddrinfo(3).
    let mut hints: libc::addrinfo = unsafe { core::mem::zeroed() };
    hints.ai_family = libc::AF_UNSPEC;
    hints.ai_socktype = libc::SOCK_STREAM;
    hints.ai_flags = libc::AI_ADDRCONFIG;
    let service = CString::new(port.to_string()).expect("digits");
    let mut result: *mut libc::addrinfo = core::ptr::null_mut();
    // SAFETY: NUL-terminated strings, hints, and an out-pointer that getaddrinfo fills.
    let rc = unsafe { libc::getaddrinfo(host.as_ptr(), service.as_ptr(), &hints, &mut result) };
    if rc != 0 {
        return Err(rc);
    }
    let mut addresses = Vec::new();
    let mut entry = result;
    while !entry.is_null() {
        // SAFETY: a node of the list getaddrinfo returned.
        unsafe {
            if let Some(address) = sockaddr_to_std((*entry).ai_addr) {
                addresses.push(address);
            }
            entry = (*entry).ai_next;
        }
    }
    // SAFETY: the list getaddrinfo returned.
    unsafe { libc::freeaddrinfo(result) };
    Ok(addresses)
}

pub(crate) unsafe fn sockaddr_to_std(addr: *const libc::sockaddr) -> Option<SocketAddr> {
    // SAFETY: the caller passes a sockaddr of the family it says.
    unsafe {
        match c_int::from((*addr).sa_family) {
            libc::AF_INET => {
                let a = &*addr.cast::<libc::sockaddr_in>();
                Some(SocketAddr::from((u32::from_be(a.sin_addr.s_addr).to_be_bytes(), u16::from_be(a.sin_port))))
            }
            libc::AF_INET6 => {
                let a = &*addr.cast::<libc::sockaddr_in6>();
                Some(SocketAddr::V6(std::net::SocketAddrV6::new(
                    a.sin6_addr.s6_addr.into(),
                    u16::from_be(a.sin6_port),
                    a.sin6_flowinfo,
                    a.sin6_scope_id,
                )))
            }
            _ => None,
        }
    }
}

/// Resolves `host` for `c` on another thread; `us_internal_socket_after_resolve` runs on the loop's.
pub(crate) fn resolve_for(c: *mut us_connecting_socket_t, host: &CStr) {
    let host = host.to_owned();
    let c = Send(c);
    std::thread::Builder::new()
        .name("usockets dns".into())
        .spawn(move || {
            let c = c;
            // SAFETY: `c` is freed only after `us_internal_socket_after_resolve` ran for it, which this queues.
            unsafe {
                let port = (*c.0).port;
                let resolved = Box::into_raw(Box::new(getaddrinfo(&host, port)));
                let loop_ = (*c.0).loop_;
                let mutex = &raw mut (*loop_).data.mutex;
                Bun__lock(mutex);
                (*c.0).resolved = resolved.cast();
                (*c.0).next = (*loop_).data.dns_ready_head;
                (*loop_).data.dns_ready_head = c.0;
                Bun__unlock(mutex);
                crate::eventing::us_wakeup_loop(loop_);
            }
        })
        .expect("usockets: cannot start a thread for name resolution");
}

unsafe extern "C" {
    fn inet_aton(cp: *const core::ffi::c_char, inp: *mut libc::in_addr) -> c_int;
}

/// What `Bun__parseIpAddress` accepts: an IPv4 address (with inet_aton's shorthands) or an IPv6 one with a zone.
pub(crate) fn parse_ip(host: &CStr, port: u16) -> Option<SocketAddr> {
    let mut v4: libc::in_addr = libc::in_addr { s_addr: 0 };
    // SAFETY: a NUL-terminated string and an in_addr to write.
    if unsafe { inet_aton(host.as_ptr(), &mut v4) } != 0 {
        return Some(SocketAddr::from((u32::from_be(v4.s_addr).to_be_bytes(), port)));
    }
    let text = host.to_str().ok()?;
    let text = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')).unwrap_or(text);
    let (address, zone) = match text.split_once('%') {
        Some((address, zone)) => (address, Some(zone)),
        None => (text, None),
    };
    let address: std::net::Ipv6Addr = address.parse().ok()?;
    let scope_id = match zone {
        None => 0,
        Some(zone) => zone.parse::<u32>().ok().or_else(|| {
            let name = CString::new(zone).ok()?;
            // SAFETY: a NUL-terminated interface name.
            let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
            (index != 0).then_some(index)
        })?,
    };
    Some(SocketAddr::V6(std::net::SocketAddrV6::new(address, port, 0, scope_id)))
}
