use super::*;

fn request() -> Value {
    json!({"file":"/bin/cat","args":[],"env":{},"stdio":["pipe","ignore","ignore"],"timeout":0,"maxBuffer":8,"killSignal":15})
}
fn exited_request() -> Value {
    let mut args = request();
    args["file"] = json!("/bin/sh");
    args["args"] = json!(["-c", "exit 0"]);
    args
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
    let args = exited_request();
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
    let args = exited_request();
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
    fd_probe_reserved_channel(parent.as_raw_fd(), child.as_raw_fd())
}
fn reserve_probe_endpoints() -> std::io::Result<(OwnedFd, OwnedFd)> {
    Ok((reserve_ipc_fd(0)?, reserve_ipc_fd(1)?))
}
fn fd_probe_reserved_channel(parent: i32, child: i32) -> i32 {
    // A CLOEXEC fd 3 stands in for Command's private spawn-error pipe.
    if unsafe { libc::dup2(parent, 3) } < 0 {
        return 24;
    }
    if unsafe { libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return 25;
    }
    let result = fd_probe_write(child);
    if result != 0 {
        return result;
    }
    let flags = unsafe { libc::fcntl(3, libc::F_GETFD) };
    if flags & libc::FD_CLOEXEC == 0 {
        27
    } else {
        0
    }
}
fn fd_probe_write(source: i32) -> i32 {
    if unsafe { libc::fcntl(source, libc::F_SETFD, 0) } < 0 {
        return 28;
    }
    let count = unsafe { libc::write(source, b"ready".as_ptr().cast(), 5) };
    if count == 5 {
        0
    } else {
        29
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
    let args = exited_request();
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

struct FailingIo(std::io::ErrorKind);
impl Write for FailingIo {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(self.0.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Read for FailingIo {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(self.0.into())
    }
}
impl AsRawFd for FailingIo {
    fn as_raw_fd(&self) -> i32 {
        -1
    }
}
fn queued(serial: u64) -> Pending {
    Pending {
        serial,
        bytes: b"bytes".to_vec(),
        written: 0,
    }
}

#[test]
fn errno_mapping_retry_policy_and_invalid_descriptors_are_explicit() {
    use std::io::ErrorKind::*;
    for (kind, code) in [
        (NotFound, "ENOENT"),
        (PermissionDenied, "EACCES"),
        (BrokenPipe, "EPIPE"),
        (InvalidInput, "EINVAL"),
        (Other, "EIO"),
    ] {
        assert_eq!(io_failure(kind.into()).code, code);
    }
    assert!(retry(&WouldBlock.into()));
    assert!(retry(&Interrupted.into()));
    assert!(!retry(&BrokenPipe.into()));
    assert!(nonblocking(-1).is_err());
    assert!(reserve_ipc_fd(-1).is_err());
    assert!(inherit_ipc_fd(-1).is_err());
    let (fd, _) = UnixStream::pair().unwrap();
    inherit_ipc_fd(fd.as_raw_fd()).unwrap();
    assert_eq!(
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    assert_eq!(
        pending(&json!({"data":"A".repeat(MAX_BYTES * 4 / 3 + 5)}))
            .err()
            .unwrap()
            .code,
        "ENOBUFS"
    );
}

#[test]
fn queued_write_errors_preserve_pending_bytes_and_retry_without_acknowledgement() {
    let mut queue = VecDeque::from([queued(7)]);
    let mut events = Vec::new();
    for kind in [
        std::io::ErrorKind::WouldBlock,
        std::io::ErrorKind::Interrupted,
    ] {
        drain_queue(1, &mut queue, &mut FailingIo(kind), "written", &mut events).unwrap();
        assert_eq!(queue_bytes(&queue), 5);
        assert!(events.is_empty());
    }
    assert!(drain_queue(
        1,
        &mut queue,
        &mut FailingIo(std::io::ErrorKind::BrokenPipe),
        "written",
        &mut events
    )
    .is_err());
    assert_eq!(queue.len(), 1);
}

#[test]
fn paused_output_does_not_read_and_failed_output_emits_one_terminal_error() {
    let mut output = Output {
        fd: 2,
        pipe: Box::new(FailingIo(std::io::ErrorKind::PermissionDenied)),
        paused: true,
    };
    let mut events = Vec::new();
    assert!(read_output(3, &mut output, &mut events));
    assert!(events.is_empty());
    output.paused = false;
    assert!(!read_output(3, &mut output, &mut events));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "streamError");
    assert_eq!(events[0]["error"]["code"], "EACCES");
}

#[test]
fn ipc_disconnect_and_protocol_errors_settle_all_pending_send_callbacks() {
    let (mut process, _) = socket_process();
    process.messages.push_back(queued(1));
    let mut events = Vec::new();
    process.disconnect(&mut events);
    assert_eq!(events[0]["kind"], "sendError");
    assert_eq!(events[0]["serial"], 1);
    assert_eq!(events[1]["kind"], "disconnect");
    let (mut process, _) = socket_process();
    process.messages.push_back(queued(2));
    events.clear();
    process.channel_error(failure("ERR_IPC_PROTOCOL", "bad frame"), &mut events);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["error"]["code"], "ERR_IPC_PROTOCOL");
    assert_eq!(events[0]["serial"], 2);
    assert!(process.messages.is_empty());
}

#[test]
fn ipc_partial_frame_bound_and_internal_dispatch_are_enforced() {
    let (mut process, mut peer) = socket_process();
    process.maximum = 4;
    peer.write_all(b"12345").unwrap();
    let mut events = Vec::new();
    process.read_ipc(&mut events);
    assert_eq!(events[0]["error"]["code"], "ENOBUFS");
    assert!(process.channel.is_none());
    assert_eq!(
        ipc_message_kind(&json!({"cmd":"NODE_TEST"})),
        "internalMessage"
    );
    assert_eq!(ipc_message_kind(&json!({"cmd":"application"})), "message");
}

#[test]
fn ipc_read_os_failure_is_reported_and_closes_the_channel() {
    let (mut process, _peer) = socket_process();
    let mut events = Vec::new();
    process.receive_ipc(
        Err(std::io::ErrorKind::PermissionDenied.into()),
        &[],
        &mut events,
    );
    assert_eq!(events[0]["error"]["code"], "EACCES");
    assert_eq!(events[1]["kind"], "disconnect");
    assert!(process.channel.is_none());
}

#[test]
fn externally_reaped_child_exit_query_reports_an_error_without_signalling() {
    let args = exited_request();
    let child = crate::process_compat::command(&args).spawn().unwrap();
    let mut process = Process::new(1, child, None, &args).unwrap();
    process.child.wait().unwrap();
    let mut events = Vec::new();
    process.check_exit(&mut events);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "error");
    assert_eq!(events[0]["error"]["code"], "EIO");
}

fn piped_process(args: &Value) -> Process {
    let mut command = crate::process_compat::command(args);
    configure_stdio(&mut command, args);
    Process::new(5, command.spawn().unwrap(), None, args).unwrap()
}
// Command's child can be reaped while a concurrent fork still holds a pre-exec
// alias of its stdin reader. Poll only terminal readiness, not writable capacity.
fn wait_for_stdin_reader_close(input: &std::process::ChildStdin) {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut fd = libc::pollfd {
        fd: input.as_raw_fd(),
        events: libc::POLLERR | libc::POLLHUP,
        revents: libc::POLLERR | libc::POLLHUP,
    };
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "inherited child stdin readers did not close"
        );
        let ready = unsafe {
            libc::poll(
                &mut fd,
                1,
                remaining.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if ready < 0 {
            assert_eq!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::Interrupted
            );
            continue;
        }
        if ready > 0 {
            assert_ne!(fd.revents & (libc::POLLERR | libc::POLLHUP), 0);
            return;
        }
    }
}
#[test]
fn broken_stdin_aborts_queued_writes_and_end_with_separate_callback_errors() {
    let mut process = piped_process(&exited_request());
    process.child.wait().unwrap();
    wait_for_stdin_reader_close(process.input.as_ref().unwrap());
    process.writes.push_back(queued(11));
    process.input_end = Some(12);
    let mut events = Vec::new();
    process.write_input(&mut events);
    assert!(process.input.is_none());
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["serial"], 11);
    assert_eq!(events[1]["serial"], 12);
    assert_eq!(events[0]["error"]["code"], "EPIPE");
    assert_eq!(events[1]["error"]["code"], "EPIPE");
    assert_eq!(
        process.queue_input(&json!({"data":""})).unwrap_err().code,
        "EPIPE"
    );
}

#[test]
fn broken_ipc_write_rejects_pending_send_and_disconnects() {
    let (mut process, _peer) = socket_process();
    // Disable this endpoint's writer: peer close may remain writable through inherited aliases.
    process
        .channel
        .as_ref()
        .unwrap()
        .shutdown(std::net::Shutdown::Write)
        .unwrap();
    process.messages.push_back(queued(9));
    let mut events = Vec::new();
    process.write_ipc(&mut events);
    assert_eq!(events[0]["kind"], "sendError");
    assert_eq!(events[0]["serial"], 9);
    assert_eq!(events[0]["error"]["code"], "EPIPE");
    assert!(process.channel.is_none());
}

#[test]
fn timeout_escalation_signals_only_the_owned_group_and_runs_once() {
    let mut args = request();
    args["file"] = json!("/bin/sleep");
    args["args"] = json!(["60"]);
    let mut process = piped_process(&args);
    process.terminating = Some(Instant::now() - Duration::from_secs(1));
    process.force_timeout();
    assert!(process.forced);
    process.force_timeout();
    assert!(process.child.wait().unwrap().signal() == Some(libc::SIGKILL));
}

#[test]
fn ref_toggle_control_updates_the_handle_refed_flag_without_closing_the_child() {
    reset();
    let mut args = request();
    args["file"] = json!("/bin/sleep");
    args["args"] = json!(["30"]);
    args["stdio"] = json!(["ignore", "ignore", "ignore"]);
    let record = operation("spawn", &args);
    let id = record["value"]["id"].as_u64().unwrap();
    assert!(operation("ref", &json!({"id":id,"enabled":false}))["value"].is_null());
    assert!(operation("ref", &json!({"id":id,"enabled":true}))["value"].is_null());
    // unknown field -> default true (acts as ref)
    assert!(operation("ref", &json!({"id":id}))["value"].is_null());
    assert_eq!(
        operation("ref", &json!({"id": id + 999, "enabled": false}))["error"]["code"],
        "ERR_CHILD_PROCESS_CLOSED"
    );
    reset();
}

#[test]
fn detached_and_unrefd_child_is_released_on_drop_without_a_sigkill_to_its_group() {
    reset();
    let mut args = request();
    args["file"] = json!("/bin/sh");
    args["args"] = json!(["-c", "sleep 2 && exit 0"]);
    args["stdio"] = json!(["ignore", "ignore", "ignore"]);
    args["detached"] = json!(true);
    let record = operation("spawn", &args);
    let id = record["value"]["id"].as_u64().unwrap();
    let pid = record["value"]["pid"].as_u64().unwrap() as i32;
    operation("ref", &json!({"id":id,"enabled":false}));
    reset();
    // The child must still be alive — Drop skipped SIGKILL to its group.
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    // Clean up so the test does not leak a surviving process group.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}

#[test]
fn active_child_without_pipes_waits_without_polling_invalid_descriptors() {
    reset();
    let mut args = request();
    args["file"] = json!("/bin/sleep");
    args["args"] = json!(["60"]);
    args["stdio"] = json!(["ignore", "ignore", "ignore"]);
    assert!(operation("spawn", &args).get("value").is_some());
    assert!(super::descriptors().is_empty());
    wait_for_io(Duration::ZERO);
    assert!(has_pending_io());
    reset();
}
