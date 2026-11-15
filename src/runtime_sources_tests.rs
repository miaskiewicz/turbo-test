use super::*;
#[test]
fn markers_expand_exactly_without_changing_unrelated_source() {
    assert_eq!(
        expand(
            "head /* __TT_A__ */ middle /* __TT_A__ */ tail",
            &[("/* __TT_A__ */", "value")]
        ),
        "head value middle value tail"
    );
    assert_eq!(expand("plain source", &[]), "plain source");
    assert_eq!(expand("/* __TT_UNKNOWN__ */", &[]), "/* __TT_UNKNOWN__ */");
    assert_eq!(
        expand("before /* __TT_UNFINISHED", &[]),
        "before /* __TT_UNFINISHED"
    );
}
#[test]
fn shared_payload_does_not_contain_backend_implementations() {
    let shared = shared_runtime_source();
    for marker in [
        "class __KeyObject",
        "class __X509Certificate",
        "class __HttpRequest",
        "const __asyncChildren",
        "function __cryptoPbkdf2",
        "const __enabledAsyncHooks",
        "const __vmContexts",
    ] {
        assert!(
            !shared.contains(marker),
            "backend implementation leaked: {marker}"
        );
    }
    assert!(shared.contains("function __cryptoCreateHmac"));
    assert!(shared.contains("class __NodeMessageChannel"));
    assert!(!shared.contains("/* __TT_"));
    let backend = backend_runtime_source();
    assert!(backend.contains("class __KeyObject"));
    assert!(backend.contains("function __cryptoPbkdf2"));
    assert!(backend.contains("__runtimeBackendInstalled = true"));
    assert!(!backend.contains("/* __TT_"));
}
