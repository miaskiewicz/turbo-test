use super::*;
#[test]
fn invalid_scope_and_unknown_rule_kind_are_not_silent_matches() {
    assert!(address(&json!("127.0.0.1%lo"), "ipv4").is_err());
    let failure = operation(&json!({"kind":"not-a-rule","address":"127.0.0.1","family":"ipv4"}));
    assert!(matches!(failure,Err(message) if message=="Invalid BlockList operation"));
}
