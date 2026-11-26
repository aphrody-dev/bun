//! Symbols that `bun_bundler` links against and that Bun's JSC side defines: JS plugins (`JSBundlerPlugin__*`),
//! JSC bytecode (`--bytecode`) and the dev server's barrel hooks. `bun build` here has none of them, so they are
//! never reached; each one aborts with its name if a path to it ever opens.

macro_rules! unreachable_without_jsc {
    ($($name:ident)*) => {
        $(
            #[unsafe(no_mangle)]
            extern "C" fn $name() -> ! {
                eprintln!(concat!("bun-boa: ", stringify!($name), " needs JavaScriptCore"));
                std::process::abort()
            }
        )*
    };
}

unreachable_without_jsc! {
    JSBundlerPlugin__anyMatches
    JSBundlerPlugin__callOnBeforeParsePlugins
    JSBundlerPlugin__hasOnBeforeParsePlugins
    __bun_dispatch__DevServerHandle__Bake__barrel_needed_exports
    __bun_dispatch__DevServerHandle__Bake__log_for_resolution_failures
    __bun_dispatch__DevServerHandle__Bake__register_barrel_with_deferrals
    __bun_jsc_bytecode_link_encoder_add_internal_module
    __bun_jsc_bytecode_link_encoder_add_internal_module_from_source
    __bun_jsc_bytecode_link_encoder_add_module
    __bun_jsc_bytecode_link_encoder_destroy
    __bun_jsc_bytecode_link_encoder_finish
    __bun_jsc_bytecode_link_encoder_new
    __bun_jsc_encoder_string_table_new
    __bun_jsc_encoder_string_table_slot
    __bun_jsc_encoder_string_table_take
    __bun_jsc_generate_cached_bytecode
    __bun_jsc_generate_internal_module_bytecode
    __bun_jsc_generate_internal_module_bytecode_from_source
    __bun_jsc_host_builtins
    __bun_jsc_wtf_string_hash
}
