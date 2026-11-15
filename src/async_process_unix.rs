use base64::Engine;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_CHILDREN: usize = 32;
const MAX_BYTES: usize = 1_048_576;
const CHUNK: usize = 8192;
const BASE64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
}
fn failure(code: &'static str, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
    }
}
fn io_failure(error: std::io::Error) -> Failure {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::BrokenPipe => "EPIPE",
        std::io::ErrorKind::InvalidInput => "EINVAL",
        _ => "EIO",
    };
    failure(code, error.to_string())
}
fn error_value(error: &Failure) -> Value {
    json!({"code":error.code,"message":error.message})
}
fn retry(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    )
}
fn nonblocking(fd: i32) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io_failure(std::io::Error::last_os_error()));
    }
    Ok(())
}
fn event(id: u64, kind: &str, mut value: Value) -> Value {
    value["id"] = id.into();
    value["kind"] = kind.into();
    value
}
trait ReadFd: Read + AsRawFd {}
impl<T: Read + AsRawFd> ReadFd for T {}
struct Output {
    fd: u8,
    pipe: Box<dyn ReadFd>,
    paused: bool,
}
struct Pending {
    serial: u64,
    bytes: Vec<u8>,
    written: usize,
}
fn pending(args: &Value) -> Result<Pending> {
    let encoded = args["data"].as_str().unwrap_or_default();
    if encoded.len() > MAX_BYTES * 4 / 3 + 4 {
        return Err(failure("ENOBUFS", "Child process write limit exceeded"));
    }
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| failure("ERR_INVALID_ARG_VALUE", "Invalid process bytes"))?;
    Ok(Pending {
        serial: args["serial"].as_u64().unwrap_or(0),
        bytes,
        written: 0,
    })
}
fn queue_bytes(queue: &VecDeque<Pending>) -> usize {
    queue
        .iter()
        .map(|item| item.bytes.len() - item.written)
        .sum()
}
fn enqueue(queue: &mut VecDeque<Pending>, item: Pending, maximum: usize) -> Result<Value> {
    if queue_bytes(queue).saturating_add(item.bytes.len()) > maximum {
        return Err(failure("ENOBUFS", "Child process queue limit exceeded"));
    }
    queue.push_back(item);
    Ok(json!(true))
}
fn drain_queue(
    id: u64,
    queue: &mut VecDeque<Pending>,
    writer: &mut impl Write,
    kind: &str,
    events: &mut Vec<Value>,
) -> std::io::Result<()> {
    let Some(item) = queue.front_mut() else {
        return Ok(());
    };
    match writer.write(&item.bytes[item.written..]) {
        Ok(count) => item.written += count,
        Err(error) if retry(&error) => return Ok(()),
        Err(error) => return Err(error),
    }
    if item.written == item.bytes.len() {
        let serial = item.serial;
        queue.pop_front();
        events.push(event(id, kind, json!({"serial":serial})));
    }
    Ok(())
}
fn read_output(id: u64, output: &mut Output, events: &mut Vec<Value>) -> bool {
    if output.paused {
        return true;
    }
    let mut bytes = [0; CHUNK];
    match output.pipe.read(&mut bytes) {
        Ok(0) => {
            events.push(event(id, "end", json!({"fd":output.fd})));
            false
        }
        Ok(count) => {
            events.push(event(
                id,
                "data",
                json!({"fd":output.fd,"data":BASE64.encode(&bytes[..count])}),
            ));
            true
        }
        Err(error) if retry(&error) => true,
        Err(error) => {
            events.push(event(
                id,
                "streamError",
                json!({"fd":output.fd,"error":error_value(&io_failure(error))}),
            ));
            false
        }
    }
}
fn stdio(mode: &Value) -> Stdio {
    match mode.as_str() {
        Some("ignore") => Stdio::null(),
        Some("inherit") => Stdio::inherit(),
        _ => Stdio::piped(),
    }
}
fn configure_stdio(command: &mut Command, args: &Value) {
    command
        .stdin(stdio(&args["stdio"][0]))
        .stdout(stdio(&args["stdio"][1]))
        .stderr(stdio(&args["stdio"][2]));
}
fn reserve_ipc_fd(fd: i32) -> std::io::Result<OwnedFd> {
    // stdio may be closed in the host; reserve outside 0..=3 before Command remaps it.
    let reserved = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 4) };
    if reserved < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(reserved) })
}
fn waitid_nowait(pid: u32, info: &mut libc::siginfo_t) -> std::io::Result<()> {
    loop {
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
fn exited_unreaped(pid: u32) -> std::io::Result<Option<ExitStatus>> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    waitid_nowait(pid, &mut info)?;
    if unsafe { info.si_pid() } == 0 {
        return Ok(None);
    }
    let status = unsafe { info.si_status() };
    let raw = if info.si_code == libc::CLD_EXITED {
        status << 8
    } else {
        status
    };
    Ok(Some(ExitStatus::from_raw(raw)))
}

fn prepare_ipc(command: &mut Command, enabled: bool) -> Result<Option<UnixStream>> {
    if !enabled {
        return Ok(None);
    }
    let (parent_original, child_original) = UnixStream::pair().map_err(io_failure)?;
    let parent = UnixStream::from(reserve_ipc_fd(parent_original.as_raw_fd()).map_err(io_failure)?);
    let child = reserve_ipc_fd(child_original.as_raw_fd()).map_err(io_failure)?;
    drop((parent_original, child_original));
    parent.set_nonblocking(true).map_err(io_failure)?;
    command
        .env("NODE_CHANNEL_FD", child.as_raw_fd().to_string())
        .env("NODE_CHANNEL_SERIALIZATION_MODE", "json");
    unsafe {
        command.pre_exec(move || {
            // Never overwrite Command's internal error pipe, which may occupy fd 3.
            if libc::fcntl(child.as_raw_fd(), libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(Some(parent))
}
fn collect_outputs(child: &mut Child) -> Result<Vec<Output>> {
    let mut outputs = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        nonblocking(pipe.as_raw_fd())?;
        outputs.push(Output {
            fd: 1,
            pipe: Box::new(pipe),
            paused: false,
        });
    }
    if let Some(pipe) = child.stderr.take() {
        nonblocking(pipe.as_raw_fd())?;
        outputs.push(Output {
            fd: 2,
            pipe: Box::new(pipe),
            paused: false,
        });
    }
    Ok(outputs)
}
struct Process {
    id: u64,
    child: Child,
    input: Option<ChildStdin>,
    outputs: Vec<Output>,
    channel: Option<UnixStream>,
    writes: VecDeque<Pending>,
    messages: VecDeque<Pending>,
    ipc_bytes: Vec<u8>,
    ipc_eof: bool,
    disconnecting: bool,
    input_end: Option<u64>,
    status: Option<ExitStatus>,
    reaped: bool,
    owned: bool,
    started: Instant,
    timeout: u64,
    kill_signal: i32,
    terminating: Option<Instant>,
    forced: bool,
    maximum: usize,
}
impl Process {
    fn new(id: u64, child: Child, channel: Option<UnixStream>, args: &Value) -> Result<Self> {
        let mut process = Self {
            id,
            child,
            input: None,
            outputs: Vec::new(),
            channel,
            writes: VecDeque::new(),
            messages: VecDeque::new(),
            ipc_bytes: Vec::new(),
            ipc_eof: false,
            disconnecting: false,
            input_end: None,
            status: None,
            reaped: false,
            owned: true,
            started: Instant::now(),
            timeout: args["timeout"].as_u64().unwrap_or(0),
            kill_signal: args["killSignal"].as_i64().unwrap_or(15) as i32,
            terminating: None,
            forced: false,
            maximum: args["maxBuffer"]
                .as_u64()
                .unwrap_or(MAX_BYTES as u64)
                .min(MAX_BYTES as u64) as usize,
        };
        process.outputs = collect_outputs(&mut process.child)?;
        process.input = process.child.stdin.take();
        if let Some(input) = &process.input {
            nonblocking(input.as_raw_fd())?;
        }
        Ok(process)
    }
    fn poll(&mut self, events: &mut Vec<Value>) -> bool {
        self.check_timeout(events);
        self.write_input(events);
        let id = self.id;
        self.outputs
            .retain_mut(|pipe| read_output(id, pipe, events));
        self.write_ipc(events);
        self.read_ipc(events);
        self.check_exit(events);
        if self.outputs.is_empty() && self.channel.is_none() {
            if let Some(status) = self.status.as_ref() {
                events.push(event(self.id, "close", status_value(status)));
                return true;
            }
        }
        false
    }
    fn owns_child(&mut self) -> bool {
        if self.reaped || !self.owned {
            return false;
        }
        match exited_unreaped(self.child.id()) {
            Ok(_) => true,
            Err(error) => {
                self.owned = false;
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.reaped = true;
                }
                false
            }
        }
    }
    fn signal_with(
        &mut self,
        signal: i32,
        group: bool,
        mut send: impl FnMut(i32, i32) -> bool,
    ) -> bool {
        if (!group && self.status.is_some()) || !self.owns_child() {
            return false;
        }
        let pid = self.child.id() as i32;
        send(if group { -pid } else { pid }, signal)
    }
    fn kill(&mut self, signal: i32) -> bool {
        self.signal_with(signal, false, |pid, signal| unsafe {
            libc::kill(pid, signal) == 0
        })
    }
    fn kill_group(&mut self, signal: i32) -> bool {
        self.signal_with(signal, true, |pid, signal| unsafe {
            libc::kill(pid, signal) == 0
        })
    }
    fn check_timeout(&mut self, events: &mut Vec<Value>) {
        self.force_timeout();
        if self.status.is_some() {
            return;
        }
        if self.terminating.is_none()
            && self.timeout > 0
            && self.started.elapsed() >= Duration::from_millis(self.timeout)
        {
            self.kill_group(self.kill_signal);
            self.terminating = Some(Instant::now());
            events.push(event(self.id, "timeout", json!({})));
        }
    }
    fn force_timeout(&mut self) {
        if !self.forced
            && self
                .terminating
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(100))
        {
            self.kill_group(9);
            self.forced = true;
        }
    }
    fn write_input(&mut self, events: &mut Vec<Value>) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        if let Err(error) = drain_queue(self.id, &mut self.writes, input, "written", events) {
            self.abort_input(io_failure(error), events);
            return;
        }
        if self.writes.is_empty() {
            if let Some(serial) = self.input_end.take() {
                self.input.take();
                events.push(event(self.id, "inputEnd", json!({"serial":serial})));
            }
        }
    }
    fn abort_input(&mut self, error: Failure, events: &mut Vec<Value>) {
        self.input.take();
        for item in self.writes.drain(..) {
            events.push(event(
                self.id,
                "writeError",
                json!({"serial":item.serial,"error":error_value(&error)}),
            ));
        }
        if let Some(serial) = self.input_end.take() {
            events.push(event(
                self.id,
                "writeError",
                json!({"serial":serial,"error":error_value(&error)}),
            ));
        }
    }
    fn write_ipc(&mut self, events: &mut Vec<Value>) {
        let Some(channel) = self.channel.as_mut() else {
            return;
        };
        if let Err(error) = drain_queue(self.id, &mut self.messages, channel, "sent", events) {
            self.channel_error(io_failure(error), events);
            return;
        }
        if self.disconnecting && self.messages.is_empty() {
            self.disconnect(events);
        }
    }
    fn read_ipc(&mut self, events: &mut Vec<Value>) {
        let Some(channel) = self.channel.as_mut() else {
            return;
        };
        let mut bytes = [0; CHUNK];
        match channel.read(&mut bytes) {
            Ok(0) => self.ipc_eof = true,
            Ok(count) => self.ipc_bytes.extend_from_slice(&bytes[..count]),
            Err(error) if retry(&error) => {}
            Err(error) => {
                self.channel_error(io_failure(error), events);
                return;
            }
        }
        self.parse_messages(events);
        if self.ipc_bytes.len() > self.maximum {
            self.channel_error(failure("ENOBUFS", "IPC message limit exceeded"), events);
        }
        self.finish_ipc(events);
    }
    fn finish_ipc(&mut self, events: &mut Vec<Value>) {
        if !self.ipc_eof {
            return;
        }
        if self.ipc_bytes.is_empty() {
            self.disconnect(events);
            return;
        }
        if !self.ipc_bytes.contains(&b'\n') {
            self.channel_error(
                failure("ERR_IPC_PROTOCOL", "Truncated IPC JSON message"),
                events,
            );
        }
    }
    fn parse_messages(&mut self, events: &mut Vec<Value>) {
        for _ in 0..64 {
            let Some(end) = self.ipc_bytes.iter().position(|byte| *byte == b'\n') else {
                return;
            };
            if end > self.maximum {
                self.channel_error(failure("ENOBUFS", "IPC message limit exceeded"), events);
                return;
            }
            let line: Vec<_> = self.ipc_bytes.drain(..=end).collect();
            match serde_json::from_slice::<Value>(&line) {
                Ok(message) => self.dispatch_message(message, events),
                Err(error) => {
                    self.channel_error(failure("ERR_IPC_PROTOCOL", error.to_string()), events);
                    return;
                }
            }
        }
    }
    fn dispatch_message(&mut self, message: Value, events: &mut Vec<Value>) {
        if message["cmd"].as_str() == Some("NODE_HANDLE") {
            self.channel_error(
                failure(
                    "ERR_IPC_HANDLE_UNSUPPORTED",
                    "IPC handle transfer is not supported",
                ),
                events,
            );
            return;
        }
        events.push(event(
            self.id,
            ipc_message_kind(&message),
            json!({"message":message}),
        ));
    }
    fn disconnect(&mut self, events: &mut Vec<Value>) {
        if self.channel.take().is_some() {
            let error = failure("ERR_IPC_CHANNEL_CLOSED", "Channel closed");
            for item in self.messages.drain(..) {
                events.push(event(
                    self.id,
                    "sendError",
                    json!({"serial":item.serial,"error":error_value(&error)}),
                ));
            }
            events.push(event(self.id, "disconnect", json!({})));
        }
    }
    fn channel_error(&mut self, error: Failure, events: &mut Vec<Value>) {
        let notify = self.messages.is_empty();
        for item in self.messages.drain(..) {
            events.push(event(
                self.id,
                "sendError",
                json!({"serial":item.serial,"error":error_value(&error)}),
            ));
        }
        if notify {
            events.push(event(
                self.id,
                "error",
                json!({"error":error_value(&error)}),
            ));
        }
        self.ipc_bytes.clear();
        self.disconnect(events);
    }
    fn check_exit(&mut self, events: &mut Vec<Value>) {
        if self.status.is_some() {
            return;
        }
        match exited_unreaped(self.child.id()) {
            Ok(Some(status)) => {
                self.abort_input(failure("EPIPE", "Child process exited"), events);
                events.push(event(self.id, "exit", status_value(&status)));
                self.status = Some(status);
            }
            Ok(None) => {}
            Err(error) => events.push(event(
                self.id,
                "error",
                json!({"error":error_value(&io_failure(error))}),
            )),
        }
    }
    fn control(&mut self, name: &str, args: &Value) -> Result<Value> {
        match name {
            "write" => self.queue_input(args),
            "end" => {
                self.input_end = Some(args["serial"].as_u64().unwrap_or(0));
                Ok(Value::Null)
            }
            "closeInput" => {
                self.input.take();
                self.writes.clear();
                self.input_end = None;
                Ok(Value::Null)
            }
            "pause" | "resume" | "closeOutput" => self.output_control(name, args),
            _ => self.channel_control(name, args),
        }
    }
    fn output_control(&mut self, name: &str, args: &Value) -> Result<Value> {
        let fd = args["fd"].as_u64().unwrap_or(1) as u8;
        if name == "closeOutput" {
            self.outputs.retain(|output| output.fd != fd);
            return Ok(Value::Null);
        }
        for output in &mut self.outputs {
            if output.fd == fd {
                output.paused = name == "pause";
            }
        }
        Ok(Value::Null)
    }
    fn queue_input(&mut self, args: &Value) -> Result<Value> {
        if self.input.is_none() {
            return Err(failure("EPIPE", "Child stdin is closed"));
        }
        enqueue(&mut self.writes, pending(args)?, self.maximum)
    }
    fn channel_control(&mut self, name: &str, args: &Value) -> Result<Value> {
        match name {
            "kill" => Ok(json!(
                self.kill(args["signal"].as_i64().unwrap_or(15) as i32)
            )),
            "send" => self.queue_message(args),
            "disconnect" => {
                self.disconnecting = true;
                Ok(Value::Null)
            }
            _ => Err(failure(
                "ERR_INVALID_ARG_VALUE",
                "Unknown child process operation",
            )),
        }
    }
    fn queue_message(&mut self, args: &Value) -> Result<Value> {
        if self.channel.is_none() || self.disconnecting {
            return Err(failure("ERR_IPC_CHANNEL_CLOSED", "Channel closed"));
        }
        let mut bytes = serde_json::to_vec(&args["message"])
            .map_err(|error| failure("ERR_INVALID_ARG_VALUE", error.to_string()))?;
        bytes.push(b'\n');
        enqueue(
            &mut self.messages,
            Pending {
                serial: args["serial"].as_u64().unwrap_or(0),
                bytes,
                written: 0,
            },
            self.maximum,
        )
    }
    fn cleanup_with(&mut self, mut signal_group: impl FnMut(u32)) {
        // WNOWAIT retains this PID until cleanup; any ownership-query error fails closed.
        if !self.owns_child() {
            return;
        }
        signal_group(self.child.id());
        self.reaped = self.child.wait().is_ok();
        self.owned = false;
    }
    fn descriptors(&self) -> Vec<libc::pollfd> {
        let mut result: Vec<_> = self
            .outputs
            .iter()
            .filter(|output| !output.paused)
            .map(|output| descriptor(output.pipe.as_raw_fd(), false))
            .collect();
        if !self.writes.is_empty() {
            if let Some(input) = &self.input {
                result.push(descriptor(input.as_raw_fd(), true));
            }
        }
        if let Some(channel) = &self.channel {
            result.push(descriptor(channel.as_raw_fd(), !self.messages.is_empty()));
        }
        result
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.cleanup_with(|pid| unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        });
    }
}

fn descriptor(fd: i32, writing: bool) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN | if writing { libc::POLLOUT } else { 0 },
        revents: 0,
    }
}
fn ipc_message_kind(message: &Value) -> &'static str {
    if message["cmd"]
        .as_str()
        .is_some_and(|command| command.starts_with("NODE_"))
    {
        "internalMessage"
    } else {
        "message"
    }
}
fn status_value(status: &ExitStatus) -> Value {
    json!({"code":status.code(),"signal":crate::process_compat::signal_name(status)})
}
#[derive(Default)]
struct Table {
    children: HashMap<u64, Process>,
    pending: Vec<Value>,
}
impl Table {
    fn spawn(&mut self, args: &Value) -> Result<Value> {
        if self.children.len() >= MAX_CHILDREN || self.pending.len() >= MAX_CHILDREN * 2 {
            return Err(failure(
                "ERR_CHILD_PROCESS_LIMIT",
                "Too many active child processes",
            ));
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut command = crate::process_compat::command(args);
        configure_stdio(&mut command, args);
        let channel = prepare_ipc(&mut command, args["ipc"].as_bool().unwrap_or(false))?;
        match command.spawn() {
            Ok(child) => self.register(id, child, channel, args),
            Err(error) => {
                let code = -error.raw_os_error().unwrap_or(1);
                self.pending.push(event(
                    id,
                    "error",
                    json!({"error":error_value(&io_failure(error))}),
                ));
                self.pending
                    .push(event(id, "close", json!({"code":code,"signal":null})));
                Ok(json!({"id":id,"pid":null}))
            }
        }
    }
    fn register(
        &mut self,
        id: u64,
        child: Child,
        channel: Option<UnixStream>,
        args: &Value,
    ) -> Result<Value> {
        let pid = child.id();
        let process = Process::new(id, child, channel, args)?;
        self.children.insert(id, process);
        self.pending.push(event(id, "spawn", json!({})));
        Ok(json!({"id":id,"pid":pid}))
    }
    fn poll(&mut self) -> Value {
        let mut events = std::mem::take(&mut self.pending);
        self.children
            .retain(|_, process| !process.poll(&mut events));
        json!(events)
    }
    fn operation(&mut self, name: &str, args: &Value) -> Result<Value> {
        match name {
            "spawn" => self.spawn(args),
            "poll" => Ok(self.poll()),
            "reset" => {
                self.children.clear();
                self.pending.clear();
                Ok(Value::Null)
            }
            _ => self
                .children
                .get_mut(&args["id"].as_u64().unwrap_or(0))
                .ok_or_else(|| {
                    failure("ERR_CHILD_PROCESS_CLOSED", "Child process handle is closed")
                })?
                .control(name, args),
        }
    }
}
thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(Table::default());
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
}
pub fn operation(name: &str, args: &Value) -> Value {
    let result = TABLE.with(|table| {
        let mut table = table.borrow_mut();
        let result = table.operation(name, args);
        ACTIVE.set(!table.children.is_empty() || !table.pending.is_empty());
        result
    });
    match result {
        Ok(value) => json!({"value":value}),
        Err(error) => json!({"error":error_value(&error)}),
    }
}
pub fn reset() {
    TABLE.with(|table| {
        let mut table = table.borrow_mut();
        table.children.clear();
        table.pending.clear();
    });
    ACTIVE.set(false);
}
pub fn has_pending_io() -> bool {
    ACTIVE.get()
}
pub fn descriptors() -> Vec<libc::pollfd> {
    if !has_pending_io() {
        return Vec::new();
    }
    TABLE.with(|table| {
        table
            .borrow()
            .children
            .values()
            .flat_map(Process::descriptors)
            .collect()
    })
}
pub fn wait_for_io(timeout: Duration) {
    if !has_pending_io() {
        return;
    }
    let mut descriptors = descriptors();
    if descriptors.is_empty() {
        std::thread::sleep(timeout);
        return;
    }
    unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            timeout.as_millis().min(i32::MAX as u128) as i32,
        );
    }
}

#[cfg(test)]
#[path = "async_process_unix_tests.rs"]
mod tests;
