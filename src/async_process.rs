//! Context-owned asynchronous child processes. Unix uses nonblocking pipes and Node JSON IPC.
use serde_json::{json, Value};
use std::time::Duration;

#[cfg(unix)]
#[path = "async_process_unix.rs"]
mod platform;

#[cfg(unix)]
pub fn has_pending_io() -> bool {
    platform::has_pending_io()
}
#[cfg(not(unix))]
pub fn has_pending_io() -> bool {
    false
}
#[cfg(unix)]
pub fn descriptors() -> Vec<libc::pollfd> {
    platform::descriptors()
}
#[cfg(unix)]
pub fn wait_for_io(timeout: Duration) {
    platform::wait_for_io(timeout);
}
#[cfg(not(unix))]
pub fn wait_for_io(_timeout: Duration) {}
#[cfg(unix)]
pub fn reset() {
    platform::reset();
}
#[cfg(not(unix))]
pub fn reset() {}

#[cfg(unix)]
fn operation(name: &str, args: &Value) -> Value {
    platform::operation(name, args)
}
#[cfg(not(unix))]
fn operation(name: &str, _args: &Value) -> Value {
    match name {
        "poll" => json!({"value": []}),
        "reset" => json!({"value": null}),
        _ => {
            json!({"error":{"code":"ERR_ASYNC_PROCESS_UNSUPPORTED_PLATFORM","message":"Asynchronous child pipes and Node IPC currently require Unix"}})
        }
    }
}

pub fn native_async_process(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let Some(encoded) = args.get(1).to_string(scope) else {
        return;
    };
    let result = if encoded.length() > 2_097_152 {
        json!({"error":{"code":"ENOBUFS","message":"Child process request limit exceeded"}})
    } else {
        parse_operation(&name, &encoded.to_rust_string_lossy(scope))
    };
    if let Some(text) = v8::String::new(scope, &result.to_string()) {
        rv.set(text.into());
    }
}

fn parse_operation(name: &str, encoded: &str) -> Value {
    match serde_json::from_str::<Value>(encoded) {
        Ok(args) => operation(name, &args),
        Err(error) => json!({"error":{"code":"ERR_INVALID_ARG_VALUE","message":error.to_string()}}),
    }
}
