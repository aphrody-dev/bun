//! TLS for client sockets (crypto/openssl.cpp's client half) on rustls with RustCrypto and the webpki roots.
//!
//! `SSL_CTX` and `SSL` are this module's `SslCtx` and `Ssl`, behind the BoringSSL entry points that `bun_http`
//! calls on them: SNI, ALPN, options, the verify result. A chain that does not verify does not fail the handshake
//! unless the socket asked for `us_socket_set_inline_reject` (or the context for `reject_unauthorized`): the error
//! goes to the handshake callback, as BoringSSL's with `SSL_VERIFY_NONE`, and the owner decides. The name check is
//! rustls's, and reports as openssl.cpp's identity check does (`ERR_TLS_CERT_ALTNAME_INVALID`).

use core::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use pki_types::pem::PemObject;
use pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct VerifyFailure {
    error: c_int,
    code: &'static CStr,
    reason: &'static CStr,
}

impl VerifyFailure {
    const fn of(error: c_int, code: &'static CStr, reason: &'static CStr) -> Self {
        Self { error, code, reason }
    }

    fn to_c(failure: Option<Self>) -> us_bun_verify_error_t {
        match failure {
            None => us_bun_verify_error_t { error: 0, code: ptr::null(), reason: ptr::null() },
            Some(f) => us_bun_verify_error_t { error: f.error, code: f.code.as_ptr(), reason: f.reason.as_ptr() },
        }
    }
}

/// rustls's verdict as the X509_V_ERR_* code, name and text that OpenSSL gives.
fn failure_of(error: &rustls::Error) -> VerifyFailure {
    use CertificateError::*;
    match error {
        rustls::Error::InvalidCertificate(cert) => match cert {
            NotValidForName | NotValidForNameContext { .. } => VerifyFailure::of(
                62,
                c"ERR_TLS_CERT_ALTNAME_INVALID",
                c"Hostname/IP does not match certificate's altnames",
            ),
            Expired | ExpiredContext { .. } => VerifyFailure::of(10, c"CERT_HAS_EXPIRED", c"certificate has expired"),
            NotValidYet | NotValidYetContext { .. } => {
                VerifyFailure::of(9, c"CERT_NOT_YET_VALID", c"certificate is not yet valid")
            }
            Revoked => VerifyFailure::of(23, c"CERT_REVOKED", c"certificate revoked"),
            BadSignature => VerifyFailure::of(7, c"CERT_SIGNATURE_FAILURE", c"certificate signature failure"),
            InvalidPurpose | InvalidPurposeContext { .. } => {
                VerifyFailure::of(26, c"INVALID_PURPOSE", c"unsupported certificate purpose")
            }
            _ => VerifyFailure::of(
                20,
                c"UNABLE_TO_GET_ISSUER_CERT_LOCALLY",
                c"unable to get local issuer certificate",
            ),
        },
        _ => VerifyFailure::of(1, c"UNSPECIFIED", c"unspecified certificate verification error"),
    }
}

/// The webpki check, whose failure is recorded for the handshake callback and fails the handshake only if `fatal`.
#[derive(Debug)]
struct Recording {
    inner: Arc<WebPkiServerVerifier>,
    failure: Arc<Mutex<Option<VerifyFailure>>>,
    fatal: bool,
}

impl ServerCertVerifier for Recording {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp, now) {
            Ok(verified) => Ok(verified),
            Err(error) => {
                *self.failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(failure_of(&error));
                if self.fatal { Err(error) } else { Ok(ServerCertVerified::assertion()) }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    static PROVIDER: std::sync::OnceLock<Arc<rustls::crypto::CryptoProvider>> = std::sync::OnceLock::new();
    PROVIDER.get_or_init(|| Arc::new(rustls_rustcrypto::provider())).clone()
}

/// `SSL_CTX`: the roots and their verifier, refcounted.
pub struct SslCtx {
    refs: AtomicU32,
    verifier: Arc<WebPkiServerVerifier>,
    reject_unauthorized: bool,
}

/// `SSL`: one connection's settings until its handshake starts, then its rustls connection.
pub struct Ssl {
    ctx: *mut SslCtx,
    host_name: Option<String>,
    connect_host: Option<String>,
    alpn: Vec<Vec<u8>>,
    alpn_selected: Vec<u8>,
    chain: Chain,
    options: u32,
    conn: Option<rustls::ClientConnection>,
    failure: Arc<Mutex<Option<VerifyFailure>>>,
    inline_reject: bool,
    handshake_done: bool,
    write_parked: bool,
    /// Ciphertext that the kernel did not take yet.
    out: Vec<u8>,
    shut_down: bool,
    session_sink: *mut c_void,
    session_on_free: Option<extern "C" fn(*mut c_void)>,
}

/// `create_bun_socket_error_t`.
const ERROR_NONE: c_int = 0;
const ERROR_LOAD_CA_FILE: c_int = 1;
const ERROR_INVALID_CA_FILE: c_int = 2;
const ERROR_INVALID_CA: c_int = 3;

fn add_pem(roots: &mut RootCertStore, pem: &[u8]) -> bool {
    let mut any = false;
    for cert in CertificateDer::pem_slice_iter(pem) {
        let Ok(cert) = cert else { return false };
        if roots.add(cert).is_err() {
            return false;
        }
        any = true;
    }
    any
}

/// The client context of `options`: webpki's roots plus `ca` and `ca_file_name`. Keys and certificates of the
/// client itself are not sent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_ssl_ctx_from_options(
    options: bun_uws_sys::BunSocketContextOptions,
    err: *mut c_int,
) -> *mut SslCtx {
    let fail = |code: c_int| {
        if !err.is_null() {
            // SAFETY: the caller's out-int (`create_bun_socket_error_t`, an int-sized enum).
            unsafe { *err = code };
        }
        ptr::null_mut()
    };
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // SAFETY: `ca` has `ca_count` NUL-terminated strings, `ca_file_name` is null or one.
    unsafe {
        for i in 0..options.ca_count as usize {
            let pem = CStr::from_ptr(*options.ca.add(i));
            if !add_pem(&mut roots, pem.to_bytes()) {
                return fail(ERROR_INVALID_CA);
            }
        }
        if !options.ca_file_name.is_null() {
            let path = CStr::from_ptr(options.ca_file_name);
            let Ok(pem) = std::fs::read(std::ffi::OsStr::from_encoded_bytes_unchecked(path.to_bytes())) else {
                return fail(ERROR_LOAD_CA_FILE);
            };
            if !add_pem(&mut roots, &pem) {
                return fail(ERROR_INVALID_CA_FILE);
            }
        }
    }
    let Ok(verifier) = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider()).build() else {
        return fail(ERROR_INVALID_CA);
    };
    if !err.is_null() {
        // SAFETY: as above.
        unsafe { *err = ERROR_NONE };
    }
    Box::into_raw(Box::new(SslCtx {
        refs: AtomicU32::new(1),
        verifier,
        reject_unauthorized: options.reject_unauthorized != 0,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_CTX_up_ref(ctx: *mut SslCtx) -> c_int {
    // SAFETY: the caller holds a ref.
    unsafe { (*ctx).refs.fetch_add(1, Ordering::Relaxed) };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_CTX_free(ctx: *mut SslCtx) {
    if ctx.is_null() {
        return;
    }
    // SAFETY: the caller gives back a ref; the last one frees the context.
    unsafe {
        if (*ctx).refs.fetch_sub(1, Ordering::AcqRel) == 1 {
            drop(Box::from_raw(ctx));
        }
    }
}

/// Buffer pools share certificate bytes between connections in BoringSSL; rustls has none.
#[unsafe(no_mangle)]
pub extern "C" fn CRYPTO_BUFFER_POOL_new() -> *mut c_void {
    static POOL: u8 = 0;
    (&raw const POOL).cast_mut().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn SSL_CTX_set0_buffer_pool(_ctx: *mut SslCtx, _pool: *mut c_void) {}

// ── the SSL entry points ───────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_get_SSL_CTX(ssl: *const Ssl) -> *mut SslCtx {
    // SAFETY: the caller's live SSL.
    unsafe { (*ssl).ctx }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_set_tlsext_host_name(ssl: *mut Ssl, name: *const c_char) -> c_int {
    // SAFETY: the caller's live SSL and NUL-terminated name.
    unsafe { (*ssl).host_name = Some(CStr::from_ptr(name).to_string_lossy().into_owned()) };
    1
}

/// `protos` in the wire format: each name after its length byte. 0 is success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_set_alpn_protos(ssl: *mut Ssl, protos: *const u8, len: usize) -> c_int {
    // SAFETY: the caller's live SSL and `len` bytes at `protos`.
    let mut wire = if len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(protos, len) } };
    let mut alpn = Vec::new();
    while let Some((&n, rest)) = wire.split_first() {
        let Some((name, rest)) = rest.split_at_checked(n as usize) else { return 1 };
        alpn.push(name.to_vec());
        wire = rest;
    }
    // SAFETY: as above.
    unsafe { (*ssl).alpn = alpn };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_get0_alpn_selected(ssl: *const Ssl, out_data: *mut *const u8, out_len: *mut c_uint) {
    // SAFETY: the caller's live SSL and out-pointers; the bytes live as long as the SSL.
    unsafe {
        let selected = &(*ssl).alpn_selected;
        *out_data = if selected.is_empty() { ptr::null() } else { selected.as_ptr() };
        *out_len = selected.len() as c_uint;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_set_options(ssl: *mut Ssl, options: u32) -> u32 {
    // SAFETY: the caller's live SSL. rustls has no legacy renegotiation to allow, so options are only kept.
    unsafe {
        (*ssl).options |= options;
        (*ssl).options
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_clear_options(ssl: *mut Ssl, options: u32) -> u32 {
    // SAFETY: as above.
    unsafe {
        (*ssl).options &= !options;
        (*ssl).options
    }
}

/// OCSP and SCTs are only asked for: nothing reads them.
#[unsafe(no_mangle)]
pub extern "C" fn SSL_enable_ocsp_stapling(_ssl: *mut Ssl) {}

#[unsafe(no_mangle)]
pub extern "C" fn SSL_enable_signed_cert_timestamps(_ssl: *mut Ssl) {}

/// An `X509`: the DER that the peer sent.
pub struct X509 {
    der: Vec<u8>,
}

/// `STACK_OF(X509)`, with `sk_num` and `sk_value`: the peer's chain, leaf first, once the handshake is done.
pub struct Chain(Vec<X509>);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_get_peer_cert_chain(ssl: *mut Ssl) -> *mut Chain {
    // SAFETY: the caller's live SSL.
    unsafe {
        if (&(*ssl).chain.0).is_empty() { ptr::null_mut() } else { &raw mut (*ssl).chain }
    }
}

/// Only peer chains are stacks in this build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sk_num(stack: *const Chain) -> usize {
    // SAFETY: a chain from `SSL_get_peer_cert_chain`.
    unsafe { (&(*stack).0).len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sk_value(stack: *const Chain, i: usize) -> *mut X509 {
    // SAFETY: as above.
    let chain = unsafe { &(*stack).0 };
    chain.get(i).map_or(ptr::null_mut(), |x| ptr::from_ref(x).cast_mut())
}

/// The DER of `x`: its length, and with a buffer at `*outp`, the bytes there and `*outp` past them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn i2d_X509(x: *const X509, outp: *mut *mut u8) -> c_int {
    // SAFETY: the caller's certificate, and null or room for its DER.
    unsafe {
        let der = &(*x).der;
        if !outp.is_null() && !(*outp).is_null() {
            ptr::copy_nonoverlapping(der.as_ptr(), *outp, der.len());
            *outp = (*outp).add(der.len());
        }
        der.len() as c_int
    }
}

/// openssl.cpp's: whether the verify step of this handshake checked the server's name. rustls's always does, so
/// `SSL_get_verify_result` already says whether it matched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_ssl_identity_checked(ssl: *mut Ssl) -> c_int {
    // SAFETY: the caller's live SSL.
    c_int::from(unsafe { (*ssl).handshake_done })
}

/// rustls resumes sessions from its own store; BoringSSL sessions never exist here.
#[unsafe(no_mangle)]
pub extern "C" fn SSL_set_session(_ssl: *mut Ssl, _session: *mut c_void) -> c_int {
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SSL_get_verify_result(ssl: *const Ssl) -> c_long {
    // SAFETY: the caller's live SSL.
    c_long::from(unsafe { verify_failure(ssl) }.map_or(0, |f| f.error))
}

unsafe fn verify_failure(ssl: *const Ssl) -> Option<VerifyFailure> {
    // SAFETY: the caller's live SSL.
    unsafe { *(*ssl).failure.lock().unwrap_or_else(|e| e.into_inner()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_ssl_socket_verify_error_from_ssl(ssl: *mut Ssl) -> us_bun_verify_error_t {
    // SAFETY: the caller's live SSL.
    VerifyFailure::to_c(unsafe { verify_failure(ssl) })
}

thread_local! {
    static LAST_ERROR: core::cell::RefCell<Option<String>> = const { core::cell::RefCell::new(None) };
}

/// The thread's last TLS failure, into `reason` (NUL-terminated): 1 if there was one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_ssl_take_error_reason(reason: *mut c_char, length: usize) -> c_int {
    let Some(text) = LAST_ERROR.with(|e| e.borrow_mut().take()) else { return 0 };
    if length == 0 {
        return 1;
    }
    let n = text.len().min(length - 1);
    // SAFETY: the caller's buffer of `length` bytes.
    unsafe {
        ptr::copy_nonoverlapping(text.as_ptr().cast::<c_char>(), reason, n);
        *reason.add(n) = 0;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_socket_set_session_sink(s: *mut us_socket_t, sink: *mut c_void, on_free: extern "C" fn(*mut c_void)) {
    // SAFETY: the caller's live TLS socket.
    unsafe {
        let ssl = (*s).ssl;
        if ssl.is_null() {
            on_free(sink);
            return;
        }
        if let Some(previous) = (*ssl).session_on_free.replace(on_free) {
            previous((*ssl).session_sink);
        }
        (*ssl).session_sink = sink;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_socket_session_sink(s: *mut us_socket_t) -> *mut c_void {
    // SAFETY: the caller's live socket.
    unsafe { if (*s).ssl.is_null() { ptr::null_mut() } else { (*(*s).ssl).session_sink } }
}

/// A chain that does not verify fails the handshake itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn us_socket_set_inline_reject(s: *mut us_socket_t) {
    // SAFETY: the caller's live socket.
    unsafe {
        if !(*s).ssl.is_null() {
            (*(*s).ssl).inline_reject = true;
        }
    }
}

// ── the socket side ────────────────────────────────────────────────────────

/// `us_internal_ssl_attach` for a client: the SSL takes a ref on `ctx`.
pub(crate) unsafe fn attach(s: *mut us_socket_t, ctx: *mut SslCtx, connect_host: Option<&CStr>) {
    // SAFETY: the caller's live socket and context.
    unsafe {
        SSL_CTX_up_ref(ctx);
        (*s).ssl = Box::into_raw(Box::new(Ssl {
            ctx,
            host_name: None,
            connect_host: connect_host.map(|h| h.to_string_lossy().into_owned()),
            alpn: Vec::new(),
            alpn_selected: Vec::new(),
            chain: Chain(Vec::new()),
            options: 0,
            conn: None,
            failure: Arc::new(Mutex::new(None)),
            inline_reject: false,
            handshake_done: false,
            write_parked: false,
            out: Vec::new(),
            shut_down: false,
            session_sink: ptr::null_mut(),
            session_on_free: None,
        }));
    }
}

/// `us_internal_ssl_detach`: after the socket's last callback.
pub(crate) unsafe fn free(s: *mut us_socket_t) {
    // SAFETY: the caller's socket, whose SSL nothing uses any more.
    unsafe {
        let ssl = core::mem::replace(&mut (*s).ssl, ptr::null_mut());
        if ssl.is_null() {
            return;
        }
        let ssl = Box::from_raw(ssl);
        if let Some(on_free) = ssl.session_on_free {
            on_free(ssl.session_sink);
        }
        SSL_CTX_free(ssl.ctx);
    }
}

pub(crate) unsafe fn native_handle(s: *mut us_socket_t) -> *mut c_void {
    // SAFETY: the caller's live socket.
    unsafe { (*s).ssl.cast() }
}

pub(crate) unsafe fn verify_error(s: *mut us_socket_t) -> us_bun_verify_error_t {
    // SAFETY: the caller's live TLS socket.
    unsafe {
        if (*s).is_closed {
            return VerifyFailure::to_c(None);
        }
        VerifyFailure::to_c(verify_failure((*s).ssl))
    }
}

pub(crate) unsafe fn is_shut_down(s: *const us_socket_t) -> bool {
    // SAFETY: the caller's live TLS socket.
    unsafe { (*(*s).ssl).shut_down }
}

/// Sends what rustls has for the peer; false if the kernel left some for the writable event.
unsafe fn flush(s: *mut us_socket_t) -> bool {
    // SAFETY: the caller's live TLS socket.
    unsafe {
        let ssl = &mut *(*s).ssl;
        if let Some(conn) = ssl.conn.as_mut() {
            while conn.wants_write() {
                if conn.write_tls(&mut ssl.out).is_err() {
                    break;
                }
            }
        }
        if ssl.out.is_empty() {
            return true;
        }
        let sent = crate::socket::raw_send(s, &ssl.out);
        ssl.out.drain(..sent);
        ssl.out.is_empty()
    }
}

fn record_error(error: &rustls::Error) {
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(error.to_string()));
}

/// The handshake failed: tell the owner, then close.
unsafe fn handshake_failed(s: *mut us_socket_t, error: Option<&rustls::Error>) {
    // SAFETY: the caller's live TLS socket.
    unsafe {
        if let Some(error) = error {
            record_error(error);
        }
        flush(s);
        let failure = verify_failure((*s).ssl);
        us_dispatch_handshake(s, 0, VerifyFailure::to_c(failure));
        if !(*s).is_closed {
            crate::socket::us_internal_socket_close_raw(s, LIBUS_SOCKET_CLOSE_CODE_CONNECTION_RESET, ptr::null_mut());
        }
    }
}

/// The TCP connect finished: the owner sets SNI and ALPN in on_open, then the ClientHello goes out.
pub(crate) unsafe fn on_open(s: *mut us_socket_t) {
    // SAFETY: the caller's live TLS socket, which on_open may close.
    unsafe {
        let s = us_dispatch_open(s, 1, ptr::null_mut(), 0);
        if s.is_null() || (*s).is_closed {
            return;
        }
        let ssl = &mut *(*s).ssl;
        let ctx = &*ssl.ctx;
        let name = ssl.host_name.clone().or_else(|| ssl.connect_host.clone()).unwrap_or_default();
        let name = name.trim_start_matches('[').trim_end_matches(']').to_owned();
        let Ok(server_name) = ServerName::try_from(name) else {
            handshake_failed(s, None);
            return;
        };
        let verifier = Arc::new(Recording {
            inner: ctx.verifier.clone(),
            failure: ssl.failure.clone(),
            fatal: ssl.inline_reject || ctx.reject_unauthorized,
        });
        let config = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map(|builder| {
                let mut config = builder.dangerous().with_custom_certificate_verifier(verifier).with_no_client_auth();
                config.alpn_protocols = ssl.alpn.clone();
                config
            });
        let conn = config.and_then(|config| rustls::ClientConnection::new(Arc::new(config), server_name));
        match conn {
            Ok(conn) => ssl.conn = Some(conn),
            Err(error) => {
                handshake_failed(s, Some(&error));
                return;
            }
        }
        if !flush(s) {
            (*s).last_write_failed = true;
            crate::socket::rearm_writable(s);
        }
    }
}

/// Ciphertext from the peer: the handshake goes on, and what it decrypts goes to on_data.
pub(crate) unsafe fn on_data(mut s: *mut us_socket_t, data: &[u8]) -> *mut us_socket_t {
    // SAFETY: the caller's live TLS socket; each callback may close it, after which it is only read.
    unsafe {
        let mut rest = data;
        let mut plaintext = [0u8; 16 * 1024];
        while !rest.is_empty() {
            let ssl = &mut *(*s).ssl;
            let Some(conn) = ssl.conn.as_mut() else { return s };
            if conn.read_tls(&mut rest).is_err() {
                break;
            }
            if let Err(error) = conn.process_new_packets() {
                if !ssl.handshake_done {
                    handshake_failed(s, Some(&error));
                } else {
                    record_error(&error);
                    flush(s);
                    crate::socket::us_internal_socket_close_raw(s, libc::ECONNRESET, ptr::null_mut());
                }
                return s;
            }
            if !ssl.handshake_done && !conn.is_handshaking() {
                ssl.handshake_done = true;
                ssl.alpn_selected = conn.alpn_protocol().map(<[u8]>::to_vec).unwrap_or_default();
                let peer = conn.peer_certificates().unwrap_or_default();
                ssl.chain = Chain(peer.iter().map(|der| X509 { der: der.to_vec() }).collect());
                flush(s);
                let failure = verify_failure((*s).ssl);
                us_dispatch_handshake(s, 1, VerifyFailure::to_c(failure));
                if (*s).is_closed {
                    return s;
                }
                if core::mem::take(&mut (*(*s).ssl).write_parked) {
                    s = us_dispatch_writable(s);
                    if s.is_null() || (*s).is_closed {
                        return s;
                    }
                }
            }
            let mut peer_closed = false;
            loop {
                let Some(conn) = (*(*s).ssl).conn.as_mut() else { return s };
                match conn.reader().read(&mut plaintext) {
                    Ok(0) => {
                        peer_closed = true;
                        break;
                    }
                    Ok(n) => {
                        s = us_dispatch_data(s, plaintext.as_mut_ptr().cast(), n as c_int);
                        if s.is_null() || (*s).is_closed {
                            return s;
                        }
                    }
                    Err(_) => break,
                }
            }
            flush(s);
            if peer_closed {
                // close_notify: the end of the stream.
                s = us_dispatch_end(s);
                if !s.is_null() && !(*s).is_closed {
                    crate::socket::us_internal_socket_close_raw(s, LIBUS_SOCKET_CLOSE_CODE_CLEAN_SHUTDOWN, ptr::null_mut());
                }
                return s;
            }
        }
        s
    }
}

/// The kernel has room: what rustls held back goes first, then the owner may write.
pub(crate) unsafe fn on_writable(s: *mut us_socket_t) -> *mut us_socket_t {
    // SAFETY: the caller's live TLS socket.
    unsafe {
        if !flush(s) {
            (*s).last_write_failed = true;
            return s;
        }
        let ssl = &*(*s).ssl;
        if !ssl.handshake_done {
            return s;
        }
        us_dispatch_writable(s)
    }
}

/// Plaintext from the owner: 0 until the handshake is done, else what rustls took.
pub(crate) unsafe fn write(s: *mut us_socket_t, data: &[u8]) -> c_int {
    // SAFETY: the caller's live TLS socket.
    unsafe {
        if (*s).is_closed || (*(*s).ssl).shut_down {
            return 0;
        }
        let ssl = &mut *(*s).ssl;
        if !ssl.handshake_done {
            ssl.write_parked = true;
            return 0;
        }
        if !ssl.out.is_empty() && !flush(s) {
            (*s).last_write_failed = true;
            crate::socket::rearm_writable(s);
            return 0;
        }
        let Some(conn) = (*(*s).ssl).conn.as_mut() else { return 0 };
        let written = conn.writer().write(data).unwrap_or(0);
        if !flush(s) || written < data.len() {
            (*s).last_write_failed = true;
            crate::socket::rearm_writable(s);
        }
        written as c_int
    }
}

/// close_notify, without waiting for the peer's.
pub(crate) unsafe fn send_close_notify(s: *mut us_socket_t) {
    // SAFETY: the caller's live TLS socket.
    unsafe {
        let ssl = &mut *(*s).ssl;
        if ssl.shut_down {
            return;
        }
        ssl.shut_down = true;
        if let Some(conn) = ssl.conn.as_mut() {
            conn.send_close_notify();
        }
        flush(s);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rustcrypto_provider_builds_a_config() {
        let roots = super::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        assert!(super::WebPkiServerVerifier::builder_with_provider(std::sync::Arc::new(roots), super::provider()).build().is_ok());
    }
}
