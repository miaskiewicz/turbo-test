use super::*;

fn request() -> Value {
    json!({"file":"/bin/cat","args":[],"env":{},"stdio":["pipe","ignore","ignore"],"timeout":0,"maxBuffer":8,"killSignal":15})
}
fn socket_process() -> (Process, UnixStream) {
    let args = request();
    let child = crate::process_compat::command(&args).spawn().unwrap();
    let (host, peer) = UnixStream::pair().unwrap();
    host.set_nonblocking(true).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let process = Process::new(42, child, Some(host), &json!({"maxBuffer":1024})).unwrap();
    (process, peer)
}

#[test]
fn reset_kills_and_reaps_only_the_owned_process_and_clears_activity() {
    reset();
    let value = operation("spawn", &request());
    let pid = value["value"]["pid"].as_u64().unwrap() as i32;
    assert!(has_pending_io());
    reset();
    assert!(!has_pending_io());
    assert!(descriptors().is_empty());
    wait_for_io(Duration::ZERO);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn stdin_queue_limits_and_invalid_controls_fail_without_leaking_children() {
    reset();
    let record = operation("spawn", &request());
    let id = record["value"]["id"].as_u64().unwrap();
    assert_eq!(
        operation(
            "write",
            &json!({"id":id,"serial":1,"data":BASE64.encode(b"123456789")})
        )["error"]["code"],
        "ENOBUFS"
    );
    assert_eq!(
        operation("write", &json!({"id":id,"serial":1,"data":"%%%"}))["error"]["code"],
        "ERR_INVALID_ARG_VALUE"
    );
    assert_eq!(
        operation("unknown", &json!({"id":id}))["error"]["code"],
        "ERR_INVALID_ARG_VALUE"
    );
    reset();
    assert_eq!(
        operation("write", &json!({"id":id,"serial":1,"data":""}))["error"]["code"],
        "ERR_CHILD_PROCESS_CLOSED"
    );
}

#[test]
fn node_json_ipc_frames_and_send_callbacks_use_real_socket_bytes() {
    let (mut process, mut peer) = socket_process();
    let mut events = Vec::new();
    peer.write_all(b"{\"value\":1}\n2\n").unwrap();
    process.read_ipc(&mut events);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["message"]["value"], 1);
    assert_eq!(events[1]["message"], 2);
    assert!(process
        .queue_message(&json!({"message":{"hello":"world"},"serial":7}))
        .is_ok());
    process.write_ipc(&mut events);
    let mut bytes = [0; 18];
    peer.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"{\"hello\":\"world\"}\n");
    assert_eq!(events.last().unwrap()["kind"], "sent");
    assert_eq!(events.last().unwrap()["serial"], 7);
    peer.write_all(b"not-json\n").unwrap();
    process.read_ipc(&mut events);
    assert!(process.channel.is_none());
    assert!(events
        .iter()
        .any(|event| event["error"]["code"] == "ERR_IPC_PROTOCOL"));
}

#[test]
fn truncated_ipc_eof_disconnects_instead_of_spinning_on_a_partial_message() {
    let (mut process, mut peer) = socket_process();
    peer.write_all(b"{").unwrap();
    peer.shutdown(std::net::Shutdown::Both).unwrap();
    let mut events = Vec::new();
    process.read_ipc(&mut events);
    process.read_ipc(&mut events);
    assert!(process.channel.is_none());
    assert!(events.iter().any(|event| event["kind"] == "disconnect"));
    assert!(process.queue_message(&json!({"message":1})).is_err());
}

#[test]
fn asynchronous_spawn_failures_have_a_bounded_pending_event_queue() {
    reset();
    let mut args = request();
    args["file"] = json!("/turbo-test-nonexistent-process-371");
    for _ in 0..MAX_CHILDREN {
        assert!(operation("spawn", &args).get("value").is_some());
    }
    assert_eq!(
        operation("spawn", &args)["error"]["code"],
        "ERR_CHILD_PROCESS_LIMIT"
    );
    assert_eq!(
        operation("poll", &json!({}))["value"]
            .as_array()
            .unwrap()
            .len(),
        MAX_CHILDREN * 2
    );
    assert!(!has_pending_io());
    reset();
}
