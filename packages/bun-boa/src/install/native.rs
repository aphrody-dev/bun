//! The C and C++ symbols that `bun_install`'s graph links against and that no Rust in this build defines yet. Each
//! stops the process with its name and where it lives in Bun's build. Weak, so a real definition wins wherever one is
//! linked.

macro_rules! not_linked {
    ($($from:literal: $($name:ident)*;)*) => {
        $($(
            #[unsafe(no_mangle)]
            #[linkage = "weak"]
            extern "C" fn $name() -> ! {
                panic!(concat!(stringify!($name), " is ", $from, ", which the Boa build does not link"));
            }
        )*)*
    };
}

not_linked! {
    "uSockets' TLS (packages/bun-usockets/src/crypto/openssl.cpp)":
        us_ssl_ctx_from_options us_ssl_socket_verify_error_from_ssl us_ssl_take_error_reason us_socket_session_sink
        us_socket_set_session_sink us_socket_set_inline_reject us_ssl_identity_checked us_ssl_set_wrapper
        us_ssl_wrapper_from_verify us_internal_ssl_client_verify_defaults;
    "BoringSSL's X.509 and BIO (the server-identity check, TLS through an HTTPS proxy)":
        ASN1_STRING_to_UTF8 BIO_clear_retry_flags BIO_get_data BIO_get_new_index BIO_meth_new BIO_meth_set_ctrl
        BIO_meth_set_read BIO_meth_set_write BIO_new BIO_set_data BIO_set_init BIO_set_retry_read GENERAL_NAME_free
        OPENSSL_free SSL_CTX_up_ref SSL_clear_options SSL_enable_ocsp_stapling SSL_enable_signed_cert_timestamps
        SSL_get_peer_cert_chain SSL_set_bio SSL_set_options SSL_set_session_id_context SSL_set_verify X509V3_EXT_d2i
        X509V3_EXT_get X509V3_EXT_get_nid X509_NAME_ENTRY_get_data X509_NAME_get_entry X509_NAME_get_index_by_NID
        X509_get_ext X509_get_ext_by_NID X509_get_subject_name i2d_X509 sk_num sk_pop_free_ex sk_value;
    "UTS #46 ToASCII of the server-identity check (src/jsc/bindings/NodeURL.cpp)":
        Bun__idnaToASCII;
    "BoringSSL (TLS, HTTP/3's QUIC crypto)":
        SSL_CIPHER_get_protocol_id SSL_CTX_free SSL_CTX_set0_buffer_pool SSL_SESSION_free SSL_SESSION_from_bytes
        SSL_SESSION_get0_id_context SSL_do_handshake SSL_early_data_accepted SSL_free SSL_get0_alpn_selected
        SSL_get0_session_id_context SSL_get_SSL_CTX SSL_get_error SSL_get_ex_data
        SSL_get_peer_quic_transport_params SSL_get_shutdown SSL_get_verify_result SSL_in_init
        SSL_is_init_finished SSL_is_quic SSL_new SSL_pending SSL_process_quic_post_handshake
        SSL_provide_quic_data SSL_read SSL_renegotiate SSL_reset_early_data_reject SSL_set_accept_state
        SSL_set_alpn_protos SSL_set_connect_state SSL_set_custom_verify SSL_set_ex_data
        SSL_set_quic_early_data_context SSL_set_quic_method SSL_set_quic_transport_params
        SSL_set_quic_use_legacy_codepoint SSL_set_renegotiate_mode SSL_set_session SSL_set_tlsext_host_name
        SSL_shutdown SSL_write EVP_AEAD_CTX_aead EVP_AEAD_CTX_free EVP_AEAD_CTX_new EVP_AEAD_CTX_open_gather
        EVP_AEAD_CTX_seal_scatter EVP_aead_aes_128_gcm EVP_aead_aes_256_gcm EVP_aead_chacha20_poly1305
        AES_encrypt AES_set_encrypt_key CRYPTO_BUFFER_POOL_new CRYPTO_chacha_20 ERR_clear_error ERR_get_error
        HKDF_expand HKDF_extract;
    "bun_runtime's DNS cache (src/runtime/dns_jsc/dns.rs), which HTTP/3's connect reads":
        Bun__addrinfo_get Bun__addrinfo_freeRequest;
    "bun_runtime's TLS over a duplex stream (src/runtime/socket/UpgradedDuplex.rs)":
        UpgradedDuplex__close UpgradedDuplex__encode_and_write UpgradedDuplex__flush UpgradedDuplex__is_closed
        UpgradedDuplex__is_established UpgradedDuplex__is_shutdown UpgradedDuplex__pause_stream
        UpgradedDuplex__resume_stream UpgradedDuplex__set_timeout UpgradedDuplex__shutdown UpgradedDuplex__ssl
        UpgradedDuplex__ssl_error UpgradedDuplex__set_inline_reject;
}
