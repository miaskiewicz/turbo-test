//! Host-specific cleanup controls: subprocesses cannot retain worker resources after abort.
#![cfg(unix)]
use super::*;

fn shell(source: &str) -> Value {
    json!({"file":"/bin/sh", "args":["-c",source], "timeout":1000,"maxBuffer":1024,"killSignal":15})
}

#[test]
fn timeout_kills_child_group_and_closes_inherited_pipes() {
    let mut request = shell("sleep 60 & wait");
    request["timeout"] = json!(20);
    let start = Instant::now();
    let result = spawn(&request, Vec::new()).unwrap();
    assert_eq!(result["error"]["code"], "ETIMEDOUT");
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(result["stdout"], "");
    assert_eq!(result["stderr"], "");
}

#[test]
fn cancellation_reaps_children_without_a_javascript_result() {
    let request = shell("sleep 60 & wait");
    let start = Instant::now();
    let result = spawn_with_cancel(&request, Vec::new(), || {
        start.elapsed() >= Duration::from_millis(20)
    })
    .unwrap();
    assert_eq!(result["error"]["code"], "ECANCELED");
    assert_eq!(result["signal"], "SIGKILL");
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn max_buffer_bounds_captured_bytes_and_reports_overflow() {
    let mut request = shell("printf 12345678901234567890");
    request["maxBuffer"] = json!(8);
    let result = spawn(&request, Vec::new()).unwrap();
    assert_eq!(result["error"]["code"], "ENOBUFS");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(result["stdout"].as_str().unwrap())
        .unwrap();
    assert_eq!(bytes, b"12345678");
}
