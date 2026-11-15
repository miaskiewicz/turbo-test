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

#[test]
fn complete_ipc_frames_respect_the_limit_before_json_deserialization() {
    let (mut process, mut peer) = socket_process();
    process.maximum = 16;
    peer.write_all(b"{\"text\":\"this complete frame exceeds sixteen bytes\"}\n")
        .unwrap();
    let mut events = Vec::new();
    process.read_ipc(&mut events);
    assert!(!events.iter().any(|event| event["kind"] == "message"));
    assert!(events
        .iter()
        .any(|event| event["error"]["code"] == "ENOBUFS"));
    assert!(process.channel.is_none());
}

#[test]
fn cleanup_signals_only_a_pid_that_is_still_an_owned_unreaped_child() {
    let mut args = request();
    args["file"] = json!("/bin/true");
    let child = crate::process_compat::command(&args).spawn().unwrap();
    let mut process = Process::new(1, child, None, &args).unwrap();
    let _ = process.child.wait().unwrap();
    let mut signals = 0;
    process.cleanup_with(|_| signals += 1);
    process.cleanup_with(|_| signals += 1);
    assert_eq!(signals, 0);
    assert!(process.reaped);
}

#[test]
fn exit_observation_retains_the_pid_until_owned_group_cleanup_and_reaping() {
    let mut args = request();
    args["file"] = json!("/bin/true");
    let child = crate::process_compat::command(&args).spawn().unwrap();
    let mut process = Process::new(1, child, None, &args).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while process.status.is_none() && Instant::now() < deadline {
        process.check_exit(&mut events);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(process.status.is_some());
    assert!(exited_unreaped(process.child.id()).unwrap().is_some());
    let mut signals = 0;
    process.cleanup_with(|_| signals += 1);
    process.cleanup_with(|_| signals += 1);
    assert_eq!(signals, 1);
    assert!(process.reaped);
}

struct ProbePid(libc::pid_t);
impl Drop for ProbePid {
    fn drop(&mut self) {
        if self.0 > 0 && exited_unreaped(self.0 as u32).is_ok() {
            unsafe {
                libc::kill(self.0, libc::SIGKILL);
                libc::waitpid(self.0, std::ptr::null_mut(), 0);
            }
        }
    }
}

#[test]
fn ipc_descriptor_reservation_survives_closed_standard_fds_in_an_isolated_fork() {
    let (mut parent, child) = UnixStream::pair().unwrap();
    parent
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0);
    if pid == 0 {
        unsafe {
            libc::_exit(fd_probe_code(parent.as_raw_fd(), child.as_raw_fd()));
        }
    }

    let mut guard = ProbePid(pid);
    let mut bytes = [0; 5];
    parent.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"ready");
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    guard.0 = 0;
    assert_eq!(status, 0);
}

fn fd_probe_code(parent_source: i32, child_source: i32) -> i32 {
    // Only descriptor syscalls and stack-only OwnedFd construction run before _exit.
    unsafe {
        libc::close(0);
        libc::close(1);
        libc::close(2);
    }
    if unsafe { libc::dup2(parent_source, 0) } < 0 {
        return 20;
    }
    if unsafe { libc::dup2(child_source, 1) } < 0 {
        return 21;
    }
    let (parent, child) = match reserve_probe_endpoints() {
        Ok(pair) => pair,
        Err(_) => return 22,
    };
    if parent.as_raw_fd() < 4 || child.as_raw_fd() < 4 {
        return 23;
    }
    unsafe {
        libc::close(0);
        libc::close(1);
    }
    fd_probe_write(child.as_raw_fd())
}
fn reserve_probe_endpoints() -> std::io::Result<(OwnedFd, OwnedFd)> {
    Ok((reserve_ipc_fd(0)?, reserve_ipc_fd(1)?))
}
fn fd_probe_write(source: i32) -> i32 {
    if unsafe { libc::dup2(source, 3) } < 0 {
        return 24;
    }
    if unsafe { libc::fcntl(3, libc::F_SETFD, 0) } < 0 {
        return 25;
    }
    let count = unsafe { libc::write(3, b"ready".as_ptr().cast(), 5) };
    if count == 5 {
        0
    } else {
        26
    }
}

#[test]
fn exited_leader_keeps_ownership_until_inherited_pipe_holders_are_terminated() {
    let mut args = request();
    args["file"] = json!("/bin/sh");
    args["args"] = json!(["-c", "printf ready; /bin/sleep 60 & exit 0"]);
    let child = crate::process_compat::command(&args).spawn().unwrap();
    let mut process = Process::new(1, child, None, &args).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while process.status.is_none() && Instant::now() < deadline {
        process.check_exit(&mut events);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(process.status.is_some());
    let index = process
        .outputs
        .iter()
        .position(|output| output.fd == 1)
        .unwrap();
    let mut pipe = process.outputs.remove(index).pipe;
    process.cleanup_with(|pid| unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    });
    assert!(pipe_reaches_eof(&mut *pipe));
}

fn pipe_reaches_eof(pipe: &mut dyn ReadFd) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bytes = [0; 32];
    while Instant::now() < deadline {
        match pipe.read(&mut bytes) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(error) if retry(&error) => std::thread::sleep(Duration::from_millis(1)),
            Err(_) => return false,
        }
    }
    false
}

#[test]
fn peer_handle_transfer_reports_an_explicit_unsupported_capability() {
    let (mut process, mut peer) = socket_process();
    peer.write_all(b"{\"cmd\":\"NODE_HANDLE\"}\n").unwrap();
    let mut events = Vec::new();
    process.read_ipc(&mut events);
    assert!(process.channel.is_none());
    assert!(events
        .iter()
        .any(|event| event["error"]["code"] == "ERR_IPC_HANDLE_UNSUPPORTED"));
    assert!(!events.iter().any(|event| event["kind"] == "message"));
}

#[test]
fn all_signal_paths_fail_closed_after_an_external_reap_without_touching_a_pid() {
    let mut args = request();
    args["file"] = json!("/bin/true");
    let child = crate::process_compat::command(&args).spawn().unwrap();
    let mut process = Process::new(1, child, None, &args).unwrap();
    process.child.wait().unwrap();
    let mut signals = Vec::new();
    assert!(!process.signal_with(15, false, |pid, signal| {
        signals.push((pid, signal));
        true
    }));
    assert!(!process.signal_with(9, true, |pid, signal| {
        signals.push((pid, signal));
        true
    }));
    process.cleanup_with(|pid| signals.push((-(pid as i32), 9)));
    assert!(signals.is_empty());
}

#[test]
fn public_kill_targets_the_leader_while_owned_timeout_cleanup_targets_its_group() {
    let args = request();
    let child = crate::process_compat::command(&args).spawn().unwrap();
    let mut process = Process::new(1, child, None, &args).unwrap();
    let pid = process.child.id() as i32;
    let mut signals = Vec::new();
    assert!(process.signal_with(15, false, |target, signal| {
        signals.push((target, signal));
        true
    }));
    assert!(process.signal_with(9, true, |target, signal| {
        signals.push((target, signal));
        true
    }));
    assert_eq!(signals, [(pid, 15), (-pid, 9)]);
}
