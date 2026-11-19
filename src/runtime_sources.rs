//! Physically separate shared startup code from backend-only JS implementations.
fn expand(source: &str, fragments: &[(&str, &str)]) -> String {
    let mut output = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("/* __TT_") {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(" */") else {
            break;
        };
        let end = end + 3;
        let replacement = fragments.iter().find(|(marker, _)| *marker == &rest[..end]);
        output.push_str(replacement.map_or(&rest[..end], |(_, source)| *source));
        rest = &rest[end..];
    }
    output.push_str(rest);
    output
}

pub(crate) fn shared_runtime_source() -> String {
    let buffer_limits = format!(
        "MAX_LENGTH: {}, MAX_STRING_LENGTH: {}",
        v8::Uint8Array::MAX_LENGTH,
        v8::String::MAX_LENGTH
    );
    expand(
        include_str!("runtime.js"),
        &[
            ("/* __TT_NODE_BUFFER_LIMITS__ */", &buffer_limits),
            (
                "/* __TT_RUNTIME_SHARED_SLOTS__ */",
                include_str!("runtime_shared_slots.js"),
            ),
            (
                "/* __TT_NODE_BUFFER_BYTE_LENGTH__ */",
                include_str!("node_buffer_byte_length.js"),
            ),
            (
                "/* __TT_NODE_BUFFER_OPS__ */",
                include_str!("node_buffer_ops.js"),
            ),
            (
                "/* __TT_NODE_PORTS__ */",
                include_str!("node_message_ports.js"),
            ),
            (
                "/* __TT_NODE_CRYPTO_HASHES__ */",
                include_str!("node_crypto_hashes.js"),
            ),
            ("/* __TT_NODE_RANDOM__ */", include_str!("node_random.js")),
            (
                "/* __TT_NODE_WARNINGS__ */",
                include_str!("node_warnings.js"),
            ),
            ("/* __TT_NODE_STREAMS__ */", include_str!("node_streams.js")),
        ],
    )
}

pub(crate) fn backend_runtime_source() -> String {
    expand(
        include_str!("runtime_backend.js"),
        &[
            ("/* __TT_NODE_KEYS__ */", include_str!("node_keys.js")),
            ("/* __TT_NODE_AES__ */", include_str!("node_aes.js")),
            ("/* __TT_NODE_X509__ */", include_str!("node_x509.js")),
            (
                "/* __TT_NODE_CRYPTO_KDF__ */",
                include_str!("node_crypto_kdf.js"),
            ),
            ("/* __TT_NODE_HTTP__ */", include_str!("node_http.js")),
            (
                "/* __TT_NODE_BLOCKLIST__ */",
                include_str!("node_blocklist.js"),
            ),
            ("/* __TT_NODE_TCP__ */", include_str!("node_tcp.js")),
            (
                "/* __TT_NODE_ZLIB_CONSTANTS__ */",
                include_str!("node_zlib_constants.js"),
            ),
            (
                "/* __TT_NODE_ASYNC_CONTEXT__ */",
                include_str!("node_async_context.js"),
            ),
            (
                "/* __TT_NODE_ASYNC_HOOKS__ */",
                include_str!("node_async_hooks.js"),
            ),
            ("/* __TT_NODE_VM__ */", include_str!("node_vm.js")),
            (
                "/* __TT_NODE_PROCESSES__ */",
                include_str!("node_processes.js"),
            ),
            (
                "/* __TT_NODE_ASYNC_PROCESSES__ */",
                include_str!("node_async_processes.js"),
            ),
        ],
    )
}

pub(crate) fn dom_runtime_source() -> &'static str {
    include_str!("runtime_dom.js")
}

#[cfg(test)]
#[path = "runtime_sources_tests.rs"]
mod tests;
