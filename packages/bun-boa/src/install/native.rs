//! The C and C++ symbols that `bun_install`'s graph links against and that no Rust in this build defines yet. Each
//! stops the process with its name and where it lives in Bun's build. Weak, so a real definition wins wherever one is
//! linked. Every `bun install` reaches uSockets: the HTTP thread that `PackageManager::init` starts puts its socket
//! group on a uSockets loop (`us_socket_group_init`), and the main thread waits on one (`us_loop_run_bun_tick`).

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
    "uSockets (packages/bun-usockets) and uWebSockets (src/uws_sys/libuwsockets.cpp)":
        uws_get_loop us_wakeup_loop us_loop_run_bun_tick us_internal_enable_sweep_timer us_internal_disable_sweep_timer
        us_socket_group_init us_socket_group_deinit us_socket_group_close_all us_socket_group_connect
        us_socket_group_connect_unix us_socket_close us_socket_ext us_socket_flush us_socket_get_error
        us_socket_get_fd us_socket_get_native_handle us_socket_is_closed us_socket_is_established
        us_socket_is_shut_down us_socket_long_timeout us_socket_pause us_socket_queued_input us_socket_resume
        us_socket_sendfile_needs_more us_socket_shutdown us_socket_timeout us_socket_write
        us_connecting_socket_close us_connecting_socket_ext us_connecting_socket_get_error
        us_connecting_socket_get_native_handle us_connecting_socket_is_closed us_connecting_socket_is_shut_down
        us_connecting_socket_long_timeout us_connecting_socket_shutdown us_connecting_socket_timeout
        us_get_local_address_info us_get_remote_address_info us_ssl_ctx_from_options
        us_ssl_socket_verify_error_from_ssl us_ssl_take_error_reason us_create_udp_socket
        us_udp_packet_buffer_payload us_udp_packet_buffer_payload_length us_udp_socket_buffer_size
        us_udp_socket_close us_udp_socket_user us_quic_create_client_ssl_ctx us_quic_enable_offload
        us_quic_ignore_receive_errors us_quic_is_routable us_quic_is_server_authorized us_quic_local_address
        us_quic_parse_address us_quic_read_envelope us_quic_resolved_address us_quic_send us_quic_set_dontfrag
        us_quic_stop_reading;
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
    "Bun's posix_spawn (src/jsc/bindings/bun-spawn.cpp)":
        posix_spawn_bun;
    "bun_runtime's DNS cache (src/runtime/dns_jsc/dns.rs), which uSockets' connect reads":
        Bun__addrinfo_get Bun__addrinfo_freeRequest;
    "bun_runtime's TLS over a duplex stream (src/runtime/socket/UpgradedDuplex.rs)":
        UpgradedDuplex__close UpgradedDuplex__encode_and_write UpgradedDuplex__flush UpgradedDuplex__is_closed
        UpgradedDuplex__is_established UpgradedDuplex__is_shutdown UpgradedDuplex__pause_stream
        UpgradedDuplex__resume_stream UpgradedDuplex__set_timeout UpgradedDuplex__shutdown UpgradedDuplex__ssl
        UpgradedDuplex__ssl_error;
}
