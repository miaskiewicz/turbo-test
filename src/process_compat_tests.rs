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

#[test]
fn process_errors_and_unusual_signals_preserve_their_identity() {
    for (kind, code) in [
        (std::io::ErrorKind::PermissionDenied, "EACCES"),
        (std::io::ErrorKind::InvalidInput, "EINVAL"),
        (std::io::ErrorKind::BrokenPipe, "EPIPE"),
        (std::io::ErrorKind::Other, "UNKNOWN"),
    ] {
        assert_eq!(error_code(&std::io::Error::from(kind)), code);
    }
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        signal_name(&ExitStatus::from_raw(2)).as_deref(),
        Some("SIGINT")
    );
    assert_eq!(
        signal_name(&ExitStatus::from_raw(3)).as_deref(),
        Some("SIG3")
    );
    assert!(signal_name(&ExitStatus::from_raw(0)).is_none());
}

#[test]
fn pipe_failures_and_panics_propagate_without_fabricated_output() {
    struct FailedRead;
    impl Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
    }
    let overflow = AtomicBool::new(false);
    assert_eq!(
        drain_pipe(&mut FailedRead, 8, &overflow)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
    let reader: Reader = std::thread::spawn(|| panic!("synthetic reader failure"));
    assert!(join_reader(reader)
        .unwrap_err()
        .to_string()
        .contains("reader panicked"));
}

#[test]
fn finish_child_reaps_on_wait_failure_and_reports_writer_failure() {
    fn reader() -> Reader {
        std::thread::spawn(|| Ok(Vec::new()))
    }
    let child = command(&shell("sleep 60")).spawn().unwrap();
    let pid = child.id();
    let writer = std::thread::spawn(|| Ok(()));
    let error = finish_child(
        ChildGuard(child),
        pid,
        Err(std::io::Error::other("synthetic wait failure")),
        reader(),
        reader(),
        writer,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "synthetic wait failure");

    let mut child = command(&shell("exit 0")).spawn().unwrap();
    let pid = child.id();
    let status = child.wait().unwrap();
    let writer = std::thread::spawn(|| Err(std::io::Error::from_raw_os_error(libc::EPIPE)));
    let result = finish_child(
        ChildGuard(child),
        pid,
        Ok((status, None)),
        reader(),
        reader(),
        writer,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result["error"]["code"], "EPIPE");
    assert_eq!(result["error"]["errno"], -libc::EPIPE);
}

#[test]
fn executable_discovery_rejects_nonfiles_and_files_without_execute_permission() {
    use std::os::unix::fs::PermissionsExt;
    let directory = std::env::temp_dir().join(format!(
        "turbo-process-executable-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    assert!(executable_path(directory.clone()).is_none());
    let file = directory.join("node");
    std::fs::write(&file, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(executable_path(file.clone()).is_none());
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
    let resolved = executable_path(file).unwrap();
    assert!(std::path::Path::new(&resolved).is_absolute());
    assert!(std::path::Path::new(&resolved).is_file());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dropping_live_child_guard_reaps_the_process() {
    let child = command(&shell("sleep 60")).spawn().unwrap();
    let pid = child.id() as libc::pid_t;
    drop(ChildGuard(child));
    let mut status = 0;
    // A second wait must report ECHILD: the guard already waited for this exact child.
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[test]
fn resistant_child_escalates_to_sigkill_after_grace_period() {
    use std::io::BufRead;
    let mut child = command(&shell(
        "trap '' TERM; printf 'ready\n'; while :; do sleep 1; done",
    ))
    .spawn()
    .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut ready = String::new();
    std::io::BufReader::new(stdout)
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "ready\n");
    let (status, reason) = abort_child(&mut child, 15, "ETIMEDOUT").unwrap();
    assert_eq!(signal_name(&status).as_deref(), Some("SIGKILL"));
    assert_eq!(reason, Some("ETIMEDOUT"));
}

#[test]
fn finish_child_reports_late_overflow_and_writer_panic() {
    fn reader() -> Reader {
        std::thread::spawn(|| Ok(Vec::new()))
    }
    let mut child = command(&shell("exit 0")).spawn().unwrap();
    let status = child.wait().unwrap();
    let pid = child.id();
    let result = finish_child(
        ChildGuard(child),
        pid,
        Ok((status, None)),
        reader(),
        reader(),
        std::thread::spawn(|| Ok(())),
        Arc::new(AtomicBool::new(true)),
    )
    .unwrap();
    assert_eq!(result["error"]["code"], "ENOBUFS");
    let mut child = command(&shell("exit 0")).spawn().unwrap();
    let status = child.wait().unwrap();
    let pid = child.id();
    let result = finish_child(
        ChildGuard(child),
        pid,
        Ok((status, None)),
        reader(),
        reader(),
        std::thread::spawn(|| panic!("synthetic writer panic")),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(result["error"]["message"]
        .as_str()
        .unwrap()
        .contains("writer panicked"));
    let executable = std::ffi::OsString::from("/bin/sh");
    assert_eq!(
        node_executable_from(Some(executable), None),
        executable_path("/bin/sh".into())
    );
}

fn with_native_spawn_scope(f: impl FnOnce(&mut v8::PinScope)) {
    crate::runner::init_v8();
    let isolate = &mut v8::Isolate::new(crate::runner::framework_snapshot().create_params());
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let name = v8::String::new(scope, "spawnNative").unwrap();
    let function = v8::Function::new(scope, native_spawn_sync).unwrap();
    context
        .global(scope)
        .set(scope, name.into(), function.into());
    f(scope);
}

fn native_script<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)?.run(scope)
}

#[test]
fn native_spawn_boundary_preserves_json_and_coercion_errors() {
    with_native_spawn_scope(|scope| {
        let result = native_script(
            scope,
            r#"(() => {
            try { spawnNative('not json', null); return false; }
            catch (error) { return error instanceof Error; }
        })()"#,
        )
        .unwrap();
        assert!(result.is_true());
        let result = native_script(
            scope,
            r#"(() => {
            const marker = new Error('conversion failed');
            try { spawnNative({toString() { throw marker; }}, null); return false; }
            catch (error) { return error === marker; }
        })()"#,
        )
        .unwrap();
        assert!(result.is_true());
        let result = native_script(scope, r#"JSON.parse(spawnNative(JSON.stringify({file:'/bin/sh',args:['-c','exit 0']}), null)).status === 0"#).unwrap();
        assert!(result.is_true());
    });
}

#[test]
fn native_spawn_termination_reaps_child_without_fabricating_a_result() {
    with_native_spawn_scope(|scope| {
        let marker = std::env::temp_dir().join(format!(
            "turbo-spawn-termination-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let request = json!({"file":"/bin/sh", "args":["-c", "printf '%s' \"$$\" > \"$PID_FILE\"; /bin/sleep 60"],
            "env":{"PID_FILE":marker.to_str().unwrap()}, "timeout":5000});
        let handle = scope.thread_safe_handle();
        let observed_marker = marker.clone();
        let terminator = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let Ok(text) = std::fs::read_to_string(&observed_marker) {
                    if let Ok(pid) = text.parse::<libc::pid_t>() {
                        handle.terminate_execution();
                        return pid;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "native child did not publish its PID"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let source = format!(
            "spawnNative({}, null)",
            serde_json::to_string(&request.to_string()).unwrap()
        );
        let result = native_script(scope, &source);
        let pid = terminator.join().unwrap();
        // A native callback can return undefined before V8 processes the pending
        // termination. Either representation must omit an ordinary spawn result.
        assert!(result.is_none_or(|value| value.is_undefined()));
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        scope.cancel_terminate_execution();
        assert_eq!(
            native_script(scope, "42").unwrap().integer_value(scope),
            Some(42)
        );
        std::fs::remove_file(marker).unwrap();
    });
}
