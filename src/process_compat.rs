//! Real synchronous child processes with concurrent pipe draining and bounded cleanup.
use base64::Engine;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

type Reader = JoinHandle<std::io::Result<Vec<u8>>>;

fn error_code(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        std::io::ErrorKind::BrokenPipe => "EPIPE",
        _ => "UNKNOWN",
    }
}

fn error_json(code: &str, message: &str, errno: Option<i32>) -> Value {
    json!({ "code": code, "message": message, "errno": errno })
}

fn spawn_error(error: &std::io::Error) -> Value {
    json!({ "pid": 0, "status": null, "signal": null, "stdout": null, "stderr": null,
        "error": error_json(error_code(error), &error.to_string(), error.raw_os_error().map(|n| -n)) })
}

fn command(request: &Value) -> Command {
    let mut command = Command::new(request["file"].as_str().unwrap_or_default());
    if let Some(arguments) = request["args"].as_array() {
        command.args(arguments.iter().map(|arg| arg.as_str().unwrap_or_default()));
    }
    if let Some(cwd) = request["cwd"].as_str() {
        command.current_dir(cwd);
    }
    configure_environment(&mut command, &request["env"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);
    command
}

fn configure_environment(command: &mut Command, environment: &Value) {
    if let Some(environment) = environment.as_object() {
        command.env_clear();
        for (key, value) in environment {
            command.env(key, value.as_str().unwrap_or_default());
        }
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}
#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate(child: &mut Child, signal: i32) {
    // Each child owns a new process group; kill descendants too so inherited pipes close.
    unsafe {
        libc::kill(-(child.id() as i32), signal);
    }
}
#[cfg(not(unix))]
fn terminate(child: &mut Child, _signal: i32) {
    let _ = child.kill();
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            terminate(&mut self.0, 9);
            let _ = self.0.wait();
        }
    }
}

fn read_pipe(
    mut pipe: impl Read + Send + 'static,
    maximum: usize,
    overflow: Arc<AtomicBool>,
    finished: Arc<AtomicUsize>,
) -> Reader {
    std::thread::spawn(move || {
        let result = drain_pipe(&mut pipe, maximum, &overflow);
        finished.fetch_add(1, Ordering::Release);
        result
    })
}

fn drain_pipe(
    pipe: &mut impl Read,
    maximum: usize,
    overflow: &AtomicBool,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let count = pipe.read(&mut chunk)?;
        if count == 0 {
            return Ok(bytes);
        }
        let available = maximum.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..count.min(available)]);
        if count > available {
            overflow.store(true, Ordering::Relaxed);
        }
    }
}

fn write_input(
    mut pipe: impl Write + Send + 'static,
    input: Vec<u8>,
    finished: Arc<AtomicUsize>,
) -> JoinHandle<std::io::Result<()>> {
    std::thread::spawn(move || {
        let result = pipe.write_all(&input);
        finished.fetch_add(1, Ordering::Release);
        result
    })
}

fn abort_reason(start: Instant, timeout: u64, overflow: &AtomicBool) -> Option<&'static str> {
    if overflow.load(Ordering::Relaxed) {
        return Some("ENOBUFS");
    }
    if timeout > 0 && start.elapsed() >= Duration::from_millis(timeout) {
        return Some("ETIMEDOUT");
    }
    None
}

fn wait_child(
    child: &mut Child,
    timeout: u64,
    overflow: &AtomicBool,
    signal: i32,
    finished: &AtomicUsize,
    cancelled: &impl Fn() -> bool,
) -> std::io::Result<(ExitStatus, Option<&'static str>)> {
    let start = Instant::now();
    loop {
        if cancelled() {
            return abort_child(child, 9, "ECANCELED");
        }
        if let Some(reason) = abort_reason(start, timeout, overflow) {
            return abort_child(child, signal, reason);
        }
        if finished.load(Ordering::Acquire) == 3 {
            if let Some(status) = child.try_wait()? {
                return Ok((status, None));
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn abort_child(
    child: &mut Child,
    signal: i32,
    reason: &'static str,
) -> std::io::Result<(ExitStatus, Option<&'static str>)> {
    terminate(child, signal);
    let deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            terminate(child, 9);
            return Ok((status, Some(reason)));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    terminate(child, 9);
    Ok((child.wait()?, Some(reason)))
}

fn join_reader(reader: Reader) -> std::io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| std::io::Error::other("child pipe reader panicked"))?
}

#[cfg(unix)]
fn signal_name(status: &ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt;
    status.signal().map(|signal| match signal {
        9 => "SIGKILL".to_owned(),
        15 => "SIGTERM".to_owned(),
        2 => "SIGINT".to_owned(),
        other => format!("SIG{other}"),
    })
}
#[cfg(not(unix))]
fn signal_name(_status: &ExitStatus) -> Option<String> {
    None
}

fn completed(
    pid: u32,
    status: ExitStatus,
    error: Option<Value>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
) -> Value {
    let encoding = base64::engine::general_purpose::STANDARD;
    json!({ "pid": pid, "status": status.code(), "signal": signal_name(&status),
        "stdout": encoding.encode(stdout), "stderr": encoding.encode(stderr),
        "error": error })
}

pub fn spawn(request: &Value, input: Vec<u8>) -> std::io::Result<Value> {
    spawn_with_cancel(request, input, || false)
}

fn spawn_with_cancel(
    request: &Value,
    input: Vec<u8>,
    cancelled: impl Fn() -> bool,
) -> std::io::Result<Value> {
    let child = match command(request).spawn() {
        Ok(child) => child,
        Err(error) => return Ok(spawn_error(&error)),
    };
    let mut child = ChildGuard(child);
    let maximum = request["maxBuffer"].as_u64().unwrap_or(1_048_576) as usize;
    let overflow = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicUsize::new(0));
    let stdout = read_pipe(
        child.0.stdout.take().unwrap(),
        maximum,
        overflow.clone(),
        finished.clone(),
    );
    let stderr = read_pipe(
        child.0.stderr.take().unwrap(),
        maximum,
        overflow.clone(),
        finished.clone(),
    );
    let writer = write_input(child.0.stdin.take().unwrap(), input, finished.clone());
    let pid = child.0.id();
    let result = wait_child(
        &mut child.0,
        request["timeout"].as_u64().unwrap_or(0),
        &overflow,
        request["killSignal"].as_i64().unwrap_or(15) as i32,
        &finished,
        &cancelled,
    );
    finish_child(child, pid, result, stdout, stderr, writer, overflow)
}

fn finish_child(
    mut child: ChildGuard,
    pid: u32,
    result: std::io::Result<(ExitStatus, Option<&'static str>)>,
    stdout: Reader,
    stderr: Reader,
    writer: JoinHandle<std::io::Result<()>>,
    overflow: Arc<AtomicBool>,
) -> std::io::Result<Value> {
    if result.is_err() {
        terminate(&mut child.0, 9);
        let _ = child.0.wait();
    }
    let stdout = join_reader(stdout);
    let stderr = join_reader(stderr);
    let write_result = writer
        .join()
        .unwrap_or_else(|_| Err(std::io::Error::other("child input writer panicked")));
    let stdout = stdout?;
    let stderr = stderr?;
    let (status, mut reason) = result?;
    if overflow.load(Ordering::Relaxed) && reason.is_none() {
        reason = Some("ENOBUFS");
    }
    let error = reason
        .map(|code| error_json(code, &format!("spawnSync {code}"), None))
        .or_else(|| {
            write_result.err().map(|error| {
                error_json(
                    error_code(&error),
                    &error.to_string(),
                    error.raw_os_error().map(|n| -n),
                )
            })
        });
    Ok(completed(pid, status, error, stdout, stderr))
}

fn input_bytes(value: v8::Local<v8::Value>) -> Vec<u8> {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) else {
        return Vec::new();
    };
    let mut bytes = vec![0; view.byte_length()];
    view.copy_contents(&mut bytes);
    bytes
}

pub fn native_spawn_sync(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(request) = args.get(0).to_string(scope) else {
        return;
    };
    let isolate = scope.thread_safe_handle();
    let result = serde_json::from_str::<Value>(&request.to_rust_string_lossy(scope))
        .map_err(|error| error.to_string())
        .and_then(|request| {
            spawn_with_cancel(&request, input_bytes(args.get(1)), || {
                isolate.is_execution_terminating()
            })
            .map_err(|error| error.to_string())
        });
    if isolate.is_execution_terminating() {
        return;
    }
    match result {
        Ok(result) => {
            let text = v8::String::new(scope, &result.to_string()).unwrap();
            rv.set(text.into());
        }
        Err(error) => {
            let text = v8::String::new(scope, &error).unwrap();
            let error = v8::Exception::error(scope, text);
            scope.throw_exception(error);
        }
    }
}

pub fn node_executable() -> Option<String> {
    if let Some(path) = std::env::var_os("TURBO_NODE_EXECUTABLE") {
        if let Some(path) = executable_path(std::path::PathBuf::from(path)) {
            return Some(path);
        }
    }
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        let name = if cfg!(windows) { "node.exe" } else { "node" };
        executable_path(directory.join(name))
    })
}

fn executable_path(path: std::path::PathBuf) -> Option<String> {
    if !is_executable(&path) {
        return None;
    }
    crate::canon::canonicalize(path)
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
#[path = "process_compat_tests.rs"]
mod tests;
