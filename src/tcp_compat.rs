//! Owned loopback TCP handles. No guest callbacks or V8 handles enter the transport.
use base64::Engine;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
}
type Result<T> = std::result::Result<T, Failure>;
fn fail(code: &'static str, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
    }
}
fn io_error(error: std::io::Error) -> Failure {
    use std::io::ErrorKind::*;
    let code = match error.kind() {
        AddrInUse => "EADDRINUSE",
        ConnectionRefused => "ECONNREFUSED",
        ConnectionReset => "ECONNRESET",
        BrokenPipe => "EPIPE",
        TimedOut => "ETIMEDOUT",
        _ => "ERR_TCP_IO",
    };
    fail(code, error.to_string())
}
fn error_value(error: Failure) -> Value {
    json!({"code":error.code,"message":error.message})
}
fn invalid() -> Failure {
    fail("ERR_INVALID_ARG_VALUE", "Invalid TCP arguments")
}
fn number(args: &Value, key: &str) -> Result<u64> {
    args[key].as_u64().ok_or_else(invalid)
}
fn flag(args: &Value, key: &str) -> Result<bool> {
    args[key].as_bool().ok_or_else(invalid)
}
fn socket_address(args: &Value) -> Result<SocketAddr> {
    let host = args["host"].as_str().ok_or_else(invalid)?;
    let host = if host == "localhost" {
        "127.0.0.1"
    } else {
        host
    };
    let ip: IpAddr = host.parse().map_err(|_| {
        fail(
            "ERR_TCP_UNSUPPORTED_HOST",
            "TCP supports literal loopback addresses and localhost",
        )
    })?;
    if !ip.is_loopback() {
        return Err(fail(
            "ERR_TCP_UNSUPPORTED_HOST",
            "TCP supports loopback addresses only",
        ));
    }
    let port = u16::try_from(number(args, "port")?).map_err(|_| invalid())?;
    Ok(SocketAddr::new(ip, port))
}
fn address_value(address: SocketAddr) -> Value {
    json!({"address":address.ip().to_string(),"port":address.port(),"family":if address.is_ipv4(){"IPv4"}else{"IPv6"}})
}
fn connection_address(socket: &TcpStream) -> Result<Value> {
    let local = socket.local_addr().map_err(io_error)?;
    let remote = socket.peer_addr().map_err(io_error)?;
    Ok(
        json!({"localAddress":local.ip().to_string(),"localPort":local.port(),"localFamily":if local.is_ipv4(){"IPv4"}else{"IPv6"},"remoteAddress":remote.ip().to_string(),"remotePort":remote.port(),"remoteFamily":if remote.is_ipv4(){"IPv4"}else{"IPv6"}}),
    )
}
fn decoded_write(args: &Value) -> Result<Vec<u8>> {
    let text = args["data"].as_str().ok_or_else(invalid)?;
    // Reject oversized encoded payloads before allocating the decoded byte buffer.
    if text.len() > (32usize * 1024 * 1024).div_ceil(3) * 4 {
        return Err(fail("ERR_OUT_OF_RANGE", "TCP write exceeds 32 MiB"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| invalid())?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(fail("ERR_OUT_OF_RANGE", "TCP write exceeds 32 MiB"));
    }
    Ok(bytes)
}
struct Listener {
    socket: TcpListener,
    referenced: bool,
}
struct Connection {
    socket: TcpStream,
    connecting: bool,
    read_closed: bool,
    write_closed: bool,
    paused: bool,
    referenced: bool,
    output: Vec<u8>,
    sent: usize,
    ack: bool,
    finishing: bool,
}
impl Connection {
    fn new(socket: TcpStream, connecting: bool) -> Result<Self> {
        socket.set_nonblocking(true).map_err(io_error)?;
        Ok(Self {
            socket,
            connecting,
            read_closed: false,
            write_closed: false,
            paused: false,
            referenced: true,
            output: Vec::new(),
            sent: 0,
            ack: false,
            finishing: false,
        })
    }
    fn connected(&mut self, id: u64, events: &mut Vec<Value>) -> Result<bool> {
        if !self.connecting {
            return Ok(true);
        }
        if let Some(error) = self.socket.take_error().map_err(io_error)? {
            return Err(io_error(error));
        }
        match self.socket.peer_addr() {
            Ok(_) => {
                self.connecting = false;
                events.push(
                    json!({"id":id,"kind":"connect","address":connection_address(&self.socket)?}),
                );
                Ok(true)
            }
            Err(error) => self.connect_failure(error),
        }
    }
    fn connect_failure(&self, peer_error: std::io::Error) -> Result<bool> {
        // The handshake can fail between SO_ERROR and getpeername. Preserve the
        // kernel's connection error instead of reporting that peer-state race.
        if let Some(error) = self.socket.take_error().map_err(io_error)? {
            return Err(io_error(error));
        }
        if pending(&peer_error) {
            return Ok(false);
        }
        Err(io_error(peer_error))
    }
    fn flush(&mut self, id: u64, events: &mut Vec<Value>) -> Result<()> {
        while self.sent < self.output.len() {
            match write_result(self.socket.write(&self.output[self.sent..]))? {
                Some(size) => self.sent += size,
                None => return Ok(()),
            }
        }
        self.output.clear();
        self.sent = 0;
        if self.ack {
            self.ack = false;
            events.push(json!({"id":id,"kind":"write"}));
        }
        self.finish(id, events)
    }
    fn finish(&mut self, id: u64, events: &mut Vec<Value>) -> Result<()> {
        if self.finishing && !self.write_closed {
            self.socket.shutdown(Shutdown::Write).map_err(io_error)?;
            self.write_closed = true;
            events.push(json!({"id":id,"kind":"finish"}));
        }
        Ok(())
    }
    fn read(&mut self, id: u64, events: &mut Vec<Value>) -> Result<()> {
        if self.paused || self.read_closed {
            return Ok(());
        }
        let mut bytes = [0u8; 16384];
        for _ in 0..16 {
            match io_result(self.socket.read(&mut bytes))? {
                Some(0) => { self.read_closed=true; events.push(json!({"id":id,"kind":"end"})); return Ok(()); }
                Some(size) => events.push(json!({"id":id,"kind":"data","data":base64::engine::general_purpose::STANDARD.encode(&bytes[..size])})),
                None => return Ok(()),
            }
        }
        Ok(())
    }
    fn poll(&mut self, id: u64, events: &mut Vec<Value>) -> Result<bool> {
        if !self.connected(id, events)? {
            return Ok(false);
        }
        self.flush(id, events)?;
        self.read(id, events)?;
        Ok(self.read_closed && self.write_closed)
    }
}
fn io_result<T>(result: std::io::Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if retryable(&error) => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}
fn write_result(result: std::io::Result<usize>) -> Result<Option<usize>> {
    match io_result(result)? {
        Some(0) => Err(fail("EPIPE", "TCP write returned zero")),
        value => Ok(value),
    }
}
fn retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    )
}
fn pending(error: &std::io::Error) -> bool {
    retryable(error) || error.raw_os_error() == Some(libc::ENOTCONN)
}
fn allocate_id(next: &mut u64, handles: usize) -> Result<u64> {
    if handles >= 4096 {
        return Err(fail("EMFILE", "TCP handle limit reached"));
    }
    *next = next
        .checked_add(1)
        .ok_or_else(|| fail("EMFILE", "TCP handle ids exhausted"))?;
    Ok(*next)
}
#[derive(Default)]
struct Transport {
    next: u64,
    listeners: HashMap<u64, Listener>,
    connections: HashMap<u64, Connection>,
}
impl Transport {
    fn id(&mut self) -> Result<u64> {
        allocate_id(
            &mut self.next,
            self.listeners.len() + self.connections.len(),
        )
    }

    fn connection(&mut self, args: &Value) -> Result<&mut Connection> {
        self.connections
            .get_mut(&number(args, "id")?)
            .ok_or_else(|| fail("ERR_SOCKET_CLOSED", "TCP socket is closed"))
    }
    fn listen(&mut self, args: &Value) -> Result<Value> {
        let address = socket_address(args)?;
        let socket = TcpListener::bind(address).map_err(io_error)?;
        socket.set_nonblocking(true).map_err(io_error)?;
        let address = address_value(socket.local_addr().map_err(io_error)?);
        let id = self.id()?;
        self.listeners.insert(
            id,
            Listener {
                socket,
                referenced: true,
            },
        );
        Ok(json!({"id":id,"address":address}))
    }
    fn connect(&mut self, args: &Value) -> Result<Value> {
        let address = socket_address(args)?;
        let connection = Connection::new(connect_socket(address)?, true)?;
        let id = self.id()?;
        self.connections.insert(id, connection);
        Ok(json!({"id":id}))
    }
    fn send(&mut self, args: &Value) -> Result<Value> {
        let bytes = decoded_write(args)?;
        let connection = self.connection(args)?;
        if connection.write_closed {
            return Err(fail("EPIPE", "TCP writable side is closed"));
        }
        if connection.ack {
            return Err(fail("ERR_TCP_WRITE_PENDING", "TCP write already pending"));
        }
        connection.output = bytes;
        connection.sent = 0;
        connection.ack = true;
        Ok(Value::Null)
    }
    fn end(&mut self, args: &Value) -> Result<Value> {
        self.connection(args)?.finishing = true;
        Ok(Value::Null)
    }
    fn pause(&mut self, args: &Value) -> Result<Value> {
        let paused = flag(args, "enabled")?;
        self.connection(args)?.paused = paused;
        Ok(Value::Null)
    }
    fn nodelay(&mut self, args: &Value) -> Result<Value> {
        let enabled = flag(args, "enabled")?;
        self.connection(args)?
            .socket
            .set_nodelay(enabled)
            .map_err(io_error)?;
        Ok(Value::Null)
    }
    fn keepalive(&mut self, args: &Value) -> Result<Value> {
        let enabled = flag(args, "enabled")?;
        let delay = u32::try_from(number(args, "delay")?).map_err(|_| invalid())?;
        set_keepalive(&self.connection(args)?.socket, enabled, delay)?;
        Ok(Value::Null)
    }
    fn reference(&mut self, args: &Value) -> Result<Value> {
        let id = number(args, "id")?;
        let enabled = flag(args, "enabled")?;
        if let Some(listener) = self.listeners.get_mut(&id) {
            listener.referenced = enabled;
            return Ok(Value::Null);
        }
        self.connection(args)?.referenced = enabled;
        Ok(Value::Null)
    }
    fn close(&mut self, args: &Value) -> Result<Value> {
        let id = number(args, "id")?;
        self.listeners.remove(&id);
        self.connections.remove(&id);
        Ok(Value::Null)
    }
    fn accept(&mut self, server: u64, events: &mut Vec<Value>) -> Result<()> {
        for _ in 0..64 {
            let result = io_result(self.listeners[&server].socket.accept())?;
            match result {
                Some((socket, _)) => self.accepted(server, socket, events)?,
                None => return Ok(()),
            }
        }
        Ok(())
    }
    fn accepted(&mut self, server: u64, socket: TcpStream, events: &mut Vec<Value>) -> Result<()> {
        let address = connection_address(&socket)?;
        let connection = Connection::new(socket, false)?;
        let id = self.id()?;
        self.connections.insert(id, connection);
        events.push(json!({"kind":"accept","server":server,"id":id,"address":address}));
        Ok(())
    }
    fn poll(&mut self, _: &Value) -> Result<Value> {
        let mut events = Vec::new();
        for server in self.listeners.keys().copied().collect::<Vec<_>>() {
            accept_completed(server, self.accept(server, &mut events), &mut events);
        }
        let ids = self.connections.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.poll_connection(id, &mut events);
        }
        Ok(Value::Array(events))
    }
    fn poll_connection(&mut self, id: u64, events: &mut Vec<Value>) {
        match self.connections.get_mut(&id).unwrap().poll(id, events) {
            Ok(false) => {}
            Ok(true) => {
                self.connections.remove(&id);
                events.push(json!({"id":id,"kind":"closed"}));
            }
            Err(error) => {
                self.connections.remove(&id);
                events.push(json!({"id":id,"kind":"error","error":error_value(error)}));
            }
        }
    }
    fn referenced(&self) -> bool {
        self.listeners.values().any(|h| h.referenced)
            || self.connections.values().any(|h| h.referenced)
    }
    fn clear(&mut self) -> Result<Value> {
        self.listeners.clear();
        self.connections.clear();
        Ok(Value::Null)
    }
}
fn accept_completed(server: u64, result: Result<()>, events: &mut Vec<Value>) {
    if let Err(error) = result {
        events.push(json!({"kind":"serverError","server":server,"error":error_value(error)}));
    }
}
thread_local! {
    static TRANSPORT: RefCell<Transport> = RefCell::new(Transport::default());
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static REFERENCED: Cell<bool> = const { Cell::new(false) };
}
type Operation = fn(&mut Transport, &Value) -> Result<Value>;
const OPERATIONS: &[(&str, Operation)] = &[
    ("listen", Transport::listen),
    ("connect", Transport::connect),
    ("send", Transport::send),
    ("end", Transport::end),
    ("pause", Transport::pause),
    ("setNoDelay", Transport::nodelay),
    ("setKeepAlive", Transport::keepalive),
    ("ref", Transport::reference),
    ("close", Transport::close),
    ("poll", Transport::poll),
    ("reset", |table, _| table.clear()),
];
fn operation(name: &str, args: &Value) -> Result<Value> {
    let (_, handler) = OPERATIONS
        .iter()
        .find(|(key, _)| *key == name)
        .ok_or_else(|| fail("ERR_TCP_UNSUPPORTED_OPERATION", "Unsupported TCP operation"))?;
    TRANSPORT.with(|table| {
        let mut table = table.borrow_mut();
        let result = handler(&mut table, args);
        ACTIVE.set(!table.listeners.is_empty() || !table.connections.is_empty());
        REFERENCED.set(table.referenced());
        result
    })
}
pub fn has_pending_io() -> bool {
    ACTIVE.get()
}
pub fn has_referenced_io() -> bool {
    REFERENCED.get()
}
pub fn reset() {
    if has_pending_io() {
        let _ = operation("reset", &Value::Null);
    }
}
pub fn native_tcp(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let encoded = args.get(1).to_rust_string_lossy(scope);
    let value = response(&name, &encoded);
    rv.set(v8::String::new(scope, &value.to_string()).unwrap().into());
}
fn response(name: &str, encoded: &str) -> Value {
    let result = serde_json::from_str(encoded)
        .map_err(|_| invalid())
        .and_then(|args| operation(name, &args));
    match result {
        Ok(value) => json!({"value":value}),
        Err(error) => json!({"error":error_value(error)}),
    }
}
#[cfg(unix)]
pub fn descriptors() -> Vec<libc::pollfd> {
    use std::os::fd::AsRawFd;
    if !has_pending_io() {
        return Vec::new();
    }
    TRANSPORT.with(|table| {
        let table = table.borrow();
        let mut descriptors = Vec::new();
        for listener in table.listeners.values() {
            descriptors.push(libc::pollfd {
                fd: listener.socket.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        for connection in table.connections.values() {
            descriptors.push(libc::pollfd {
                fd: connection.socket.as_raw_fd(),
                events: interests(connection),
                revents: 0,
            });
        }
        descriptors
    })
}
#[cfg(unix)]
fn interests(connection: &Connection) -> i16 {
    let mut flags = 0;
    if !connection.read_closed && !connection.paused {
        flags |= libc::POLLIN;
    }
    if connection.connecting || connection.sent < connection.output.len() {
        flags |= libc::POLLOUT;
    }
    flags
}
#[cfg(unix)]
pub fn wait_for_io(timeout: Duration) {
    let mut descriptors = descriptors();
    if descriptors.is_empty() {
        return;
    }
    // The table owns every fd throughout this call; no guest code executes while polling.
    unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            timeout.as_millis().min(10) as i32,
        );
    }
}
#[cfg(not(unix))]
pub fn wait_for_io(timeout: Duration) {
    if has_pending_io() {
        std::thread::sleep(timeout.min(Duration::from_millis(10)));
    }
}

#[cfg(unix)]
fn socket_option(
    socket: &TcpStream,
    level: libc::c_int,
    name: libc::c_int,
    value: libc::c_int,
) -> Result<()> {
    use std::os::fd::AsRawFd;
    // The descriptor and integer option remain owned and live throughout setsockopt.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            level,
            name,
            (&value as *const libc::c_int).cast(),
            std::mem::size_of_val(&value) as _,
        )
    };
    syscall(result).map_err(io_error)
}
#[cfg(unix)]
fn set_keepalive(socket: &TcpStream, enabled: bool, delay: u32) -> Result<()> {
    socket_option(
        socket,
        libc::SOL_SOCKET,
        libc::SO_KEEPALIVE,
        i32::from(enabled),
    )?;
    if enabled && delay > 0 {
        keepalive_delay(socket, (delay / 1000).max(1) as i32)?;
    }
    Ok(())
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn keepalive_delay(socket: &TcpStream, seconds: i32) -> Result<()> {
    socket_option(socket, libc::IPPROTO_TCP, libc::TCP_KEEPALIVE, seconds)
}
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios"))))]
fn keepalive_delay(socket: &TcpStream, seconds: i32) -> Result<()> {
    socket_option(socket, libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, seconds)
}
#[cfg(not(unix))]
fn set_keepalive(_: &TcpStream, _: bool, _: u32) -> Result<()> {
    Err(fail(
        "ERR_TCP_UNSUPPORTED_OPTION",
        "TCP keepalive is unsupported on this target",
    ))
}

#[cfg(unix)]
fn connect_socket(address: SocketAddr) -> Result<TcpStream> {
    use std::os::fd::AsRawFd;
    let socket = connecting_socket(address)?;
    let result = syscall(raw_connect(socket.as_raw_fd(), address));
    complete_connect(socket, result)
}
#[cfg(unix)]
fn complete_connect(socket: TcpStream, result: std::io::Result<()>) -> Result<TcpStream> {
    match result {
        Ok(()) => Ok(socket),
        Err(error) if error.raw_os_error() == Some(libc::EINPROGRESS) || retryable(&error) => {
            Ok(socket)
        }
        Err(error) => Err(io_error(error)),
    }
}
#[cfg(unix)]
fn syscall(result: libc::c_int) -> std::io::Result<()> {
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
#[cfg(unix)]
fn cloexec(fd: libc::c_int) -> Result<()> {
    // Owned TCP descriptors must not leak through child-process exec.
    syscall(unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) }).map_err(io_error)
}
#[cfg(unix)]
fn owned_socket(family: libc::c_int) -> Result<TcpStream> {
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::socket(family, libc::SOCK_STREAM, 0) };
    syscall(fd).map_err(io_error)?;
    // Ownership transfers immediately so configuration failures close the descriptor.
    Ok(unsafe { TcpStream::from_raw_fd(fd) })
}
#[cfg(unix)]
fn connecting_socket(address: SocketAddr) -> Result<TcpStream> {
    use std::os::fd::AsRawFd;
    let family = if address.is_ipv4() {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };
    let socket = owned_socket(family)?;
    cloexec(socket.as_raw_fd())?;
    socket.set_nonblocking(true).map_err(io_error)?;
    Ok(socket)
}

#[cfg(unix)]
fn raw_connect(fd: libc::c_int, address: SocketAddr) -> libc::c_int {
    match address {
        SocketAddr::V4(address) => {
            // Zero initializes the platform padding; sockaddr lengths are supplied explicitly.
            let mut raw: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            raw.sin_family = libc::AF_INET as _;
            raw.sin_port = address.port().to_be();
            raw.sin_addr.s_addr = u32::from_ne_bytes(address.ip().octets());
            #[cfg(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "dragonfly"
            ))]
            {
                raw.sin_len = std::mem::size_of_val(&raw) as _;
            }
            unsafe {
                libc::connect(
                    fd,
                    (&raw as *const libc::sockaddr_in).cast(),
                    std::mem::size_of_val(&raw) as _,
                )
            }
        }
        SocketAddr::V6(address) => {
            let mut raw: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
            raw.sin6_family = libc::AF_INET6 as _;
            raw.sin6_port = address.port().to_be();
            raw.sin6_addr.s6_addr = address.ip().octets();
            raw.sin6_scope_id = address.scope_id();
            #[cfg(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "dragonfly"
            ))]
            {
                raw.sin6_len = std::mem::size_of_val(&raw) as _;
            }
            unsafe {
                libc::connect(
                    fd,
                    (&raw as *const libc::sockaddr_in6).cast(),
                    std::mem::size_of_val(&raw) as _,
                )
            }
        }
    }
}
#[cfg(not(unix))]
fn connect_socket(address: SocketAddr) -> Result<TcpStream> {
    TcpStream::connect_timeout(&address, Duration::from_millis(250)).map_err(io_error)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn args(id: u64) -> Value {
        json!({"id":id})
    }
    fn pair(table: &mut Transport, host: &str) -> (u64, u64, u64) {
        let listening = table.listen(&json!({"host":host,"port":0})).unwrap();
        let server = listening["id"].as_u64().unwrap();
        let client = table
            .connect(&json!({"host":host,"port":listening["address"]["port"]}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        let events = until(table, |events| {
            events.iter().any(|e| e["kind"] == "accept")
                && events.iter().any(|e| e["kind"] == "connect")
        });
        let peer = events.iter().find(|e| e["kind"] == "accept").unwrap()["id"]
            .as_u64()
            .unwrap();
        (server, client, peer)
    }
    fn until(table: &mut Transport, ready: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut events = Vec::new();
        loop {
            events.extend(
                table
                    .poll(&Value::Null)
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .iter()
                    .cloned(),
            );
            if ready(&events) {
                return events;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "TCP events: {events:?}"
            );
            std::thread::yield_now();
        }
    }
    fn write(table: &mut Transport, id: u64, bytes: &[u8]) {
        table
            .send(&json!({"id":id,"data":base64::engine::general_purpose::STANDARD.encode(bytes)}))
            .unwrap();
    }
    fn data(events: &[Value], id: u64) -> Vec<u8> {
        events
            .iter()
            .filter(|e| e["id"] == id && e["kind"] == "data")
            .flat_map(|e| {
                base64::engine::general_purpose::STANDARD
                    .decode(e["data"].as_str().unwrap())
                    .unwrap()
            })
            .collect()
    }
    #[test]
    fn native_bytes_options_pause_half_close_and_addresses() {
        for host in ["127.0.0.1", "::1"] {
            let mut table = Transport::default();
            let (server, client, peer) = pair(&mut table, host);
            assert!(table.referenced());
            use std::os::fd::AsRawFd;
            let flags = unsafe {
                libc::fcntl(table.connections[&client].socket.as_raw_fd(), libc::F_GETFD)
            };
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
            for enabled in [false, true] {
                table
                    .keepalive(&json!({"id":client,"enabled":enabled,"delay":2000}))
                    .unwrap();
                let mut value: libc::c_int = -1;
                let mut length = std::mem::size_of_val(&value) as libc::socklen_t;
                let result = unsafe {
                    libc::getsockopt(
                        table.connections[&client].socket.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_KEEPALIVE,
                        (&mut value as *mut libc::c_int).cast(),
                        &mut length,
                    )
                };
                assert_eq!(result, 0);
                assert_eq!(value != 0, enabled);
            }
            for enabled in [false, true] {
                table
                    .nodelay(&json!({"id":client,"enabled":enabled}))
                    .unwrap();
                assert_eq!(
                    table.connections[&client].socket.nodelay().unwrap(),
                    enabled
                );
            }
            table.pause(&json!({"id":peer,"enabled":true})).unwrap();
            write(&mut table, client, &[0, 255, 128, 1]);
            let events = until(&mut table, |events| {
                events
                    .iter()
                    .any(|e| e["id"] == client && e["kind"] == "write")
            });
            assert!(data(&events, peer).is_empty());
            table.pause(&json!({"id":peer,"enabled":false})).unwrap();
            let events = until(&mut table, |events| !data(events, peer).is_empty());
            assert_eq!(data(&events, peer), [0, 255, 128, 1]);
            table.end(&args(client)).unwrap();
            until(&mut table, |events| {
                events.iter().any(|e| e["id"] == peer && e["kind"] == "end")
            });
            assert!(!table.connections[&peer].write_closed);
            write(&mut table, peer, b"after EOF");
            table.end(&args(peer)).unwrap();
            let events = until(&mut table, |events| {
                events
                    .iter()
                    .any(|e| e["id"] == client && e["kind"] == "closed")
            });
            assert_eq!(data(&events, client), b"after EOF");
            assert!(table.connections.is_empty());
            table.close(&args(server)).unwrap();
            assert!(table.listeners.is_empty());
        }
    }
    #[test]
    fn reference_state_and_reset_own_handles_without_reusing_ids() {
        reset();
        assert!(!has_pending_io());
        assert!(!has_referenced_io());
        assert!(descriptors().is_empty());
        wait_for_io(Duration::from_secs(30));
        let server = operation("listen", &json!({"host":"localhost","port":0})).unwrap();
        let id = server["id"].as_u64().unwrap();
        assert!(has_pending_io());
        assert!(has_referenced_io());
        assert_eq!(descriptors().len(), 1);
        operation("ref", &json!({"id":id,"enabled":false})).unwrap();
        assert!(!has_referenced_io());
        operation("ref", &json!({"id":id,"enabled":true})).unwrap();
        assert!(has_referenced_io());
        wait_for_io(Duration::ZERO);
        reset();
        assert!(!has_pending_io());
        assert!(!has_referenced_io());
        assert!(descriptors().is_empty());
        let address = SocketAddr::new(
            "127.0.0.1".parse().unwrap(),
            server["address"]["port"].as_u64().unwrap() as u16,
        );
        let rebound = TcpListener::bind(address).unwrap();
        drop(rebound);
        let next = operation("listen", &json!({"host":"127.0.0.1","port":0})).unwrap()["id"]
            .as_u64()
            .unwrap();
        assert!(next > id);
        assert_eq!(
            operation("setNoDelay", &json!({"id":id,"enabled":true}))
                .unwrap_err()
                .code,
            "ERR_SOCKET_CLOSED"
        );
        reset();
    }
    #[test]
    fn refused_connections_emit_failure_and_release_native_handle() {
        let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reserved.local_addr().unwrap();
        drop(reserved);
        let mut table = Transport::default();
        let id = table
            .connect(&json!({"host":"127.0.0.1","port":address.port()}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        let events = until(&mut table, |events| {
            events.iter().any(|e| e["kind"] == "error")
        });
        assert_eq!(events[0]["id"], id);
        assert_eq!(events[0]["error"]["code"], "ECONNREFUSED", "{events:?}");
        assert!(table.connections.is_empty());
    }
    #[test]
    fn peer_failure_prefers_authoritative_socket_error_after_connect_state_changes() {
        use std::os::fd::AsRawFd;
        let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reserved.local_addr().unwrap();
        drop(reserved);
        let socket = connect_socket(address).unwrap();
        let mut descriptor = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut descriptor, 1, 1000) }, 1);
        // On Darwin this is EINVAL once the refused handshake changes the peer state.
        // SO_ERROR still owns ECONNREFUSED, and must take precedence over getpeername.
        let peer_error = socket.peer_addr().unwrap_err();
        let connection = Connection::new(socket, true).unwrap();
        assert_eq!(
            connection.connect_failure(peer_error).unwrap_err().code,
            "ECONNREFUSED"
        );
        let socket = connecting_socket(address).unwrap();
        let peer_error = socket.peer_addr().unwrap_err();
        let connection = Connection::new(socket, true).unwrap();
        assert!(!connection.connect_failure(peer_error).unwrap());
        assert_eq!(
            connection
                .connect_failure(std::io::Error::from(std::io::ErrorKind::InvalidInput))
                .unwrap_err()
                .code,
            "ERR_TCP_IO"
        );
    }
    #[test]
    fn malformed_inputs_unknown_handles_and_bounds_are_errors() {
        let mut table = Transport::default();
        for value in [
            json!({}),
            json!({"host":12,"port":0}),
            json!({"host":"127.0.0.1","port":-1}),
            json!({"host":"127.0.0.1","port":65536}),
        ] {
            assert_eq!(
                table.listen(&value).unwrap_err().code,
                "ERR_INVALID_ARG_VALUE"
            );
        }
        for host in ["example.invalid", "192.0.2.1"] {
            assert_eq!(
                table
                    .connect(&json!({"host":host,"port":1}))
                    .unwrap_err()
                    .code,
                "ERR_TCP_UNSUPPORTED_HOST"
            );
        }
        assert_eq!(
            table.send(&json!({"id":0,"data":"!"})).unwrap_err().code,
            "ERR_INVALID_ARG_VALUE"
        );
        assert_eq!(
            table.send(&args(0)).unwrap_err().code,
            "ERR_INVALID_ARG_VALUE"
        );
        for name in ["send", "end", "pause", "setNoDelay", "ref"] {
            let value = json!({"id":0,"data":"","enabled":true});
            assert_eq!(
                operation(name, &value).unwrap_err().code,
                "ERR_SOCKET_CLOSED"
            );
        }
        assert_eq!(
            table.pause(&json!({"id":0,"enabled":1})).unwrap_err().code,
            "ERR_INVALID_ARG_VALUE"
        );
        assert_eq!(
            table.close(&json!({})).unwrap_err().code,
            "ERR_INVALID_ARG_VALUE"
        );
        table.close(&args(0)).unwrap();
        assert_eq!(
            operation("bogus", &Value::Null).unwrap_err().code,
            "ERR_TCP_UNSUPPORTED_OPERATION"
        );
        table.next = u64::MAX;
        assert_eq!(table.id().unwrap_err().code, "EMFILE");
    }
    #[test]
    fn pending_write_and_ended_write_are_rejected_without_silencing_errors() {
        let mut table = Transport::default();
        let (_, client, _) = pair(&mut table, "127.0.0.1");
        write(&mut table, client, b"queued");
        assert_eq!(
            table
                .send(&json!({"id":client,"data":""}))
                .unwrap_err()
                .code,
            "ERR_TCP_WRITE_PENDING"
        );
        table.connections.get_mut(&client).unwrap().write_closed = true;
        assert_eq!(
            table
                .send(&json!({"id":client,"data":""}))
                .unwrap_err()
                .code,
            "EPIPE"
        );
        let huge = base64::engine::general_purpose::STANDARD.encode(vec![0; 32 * 1024 * 1024 + 1]);
        assert_eq!(
            table
                .send(&json!({"id":client,"data":huge}))
                .unwrap_err()
                .code,
            "ERR_OUT_OF_RANGE"
        );
        table
            .reference(&json!({"id":client,"enabled":false}))
            .unwrap();
        assert!(!table.connections[&client].referenced);
        table
            .reference(&json!({"id":client,"enabled":true}))
            .unwrap();
        assert!(table.connections[&client].referenced);
    }
    #[test]
    fn encoded_write_bound_precedes_decoding_and_accepts_exact_byte_limit() {
        let encoded_limit = (32usize * 1024 * 1024).div_ceil(3) * 4;
        // Malformed input would be INVALID_ARG_VALUE if decoding ran first.
        let oversized = "!".repeat(encoded_limit + 1);
        assert_eq!(
            decoded_write(&json!({"data":oversized})).unwrap_err().code,
            "ERR_OUT_OF_RANGE"
        );
        assert_eq!(
            decoded_write(&json!({"data":"!!!!"})).unwrap_err().code,
            "ERR_INVALID_ARG_VALUE"
        );
        let limit = vec![0; 32 * 1024 * 1024];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&limit);
        assert_eq!(decoded_write(&json!({"data":encoded})).unwrap(), limit);
    }
    #[test]
    fn socket_interests_cover_connect_write_pause_and_eof() {
        let mut table = Transport::default();
        let (_, client, _) = pair(&mut table, "127.0.0.1");
        let connection = table.connections.get_mut(&client).unwrap();
        assert_eq!(interests(connection), libc::POLLIN);
        connection.connecting = true;
        assert_eq!(interests(connection), libc::POLLIN | libc::POLLOUT);
        connection.connecting = false;
        connection.output = vec![1];
        assert_eq!(interests(connection), libc::POLLIN | libc::POLLOUT);
        connection.paused = true;
        assert_eq!(interests(connection), libc::POLLOUT);
        connection.sent = 1;
        assert_eq!(interests(connection), 0);
        connection.paused = false;
        connection.read_closed = true;
        assert_eq!(interests(connection), 0);
    }
    #[test]
    fn io_completion_preserves_partial_progress_retries_eof_and_failures() {
        use std::io::ErrorKind::*;
        assert_eq!(write_result(Ok(7)).unwrap(), Some(7));
        assert_eq!(write_result(Ok(0)).unwrap_err().code, "EPIPE");
        for kind in [WouldBlock, Interrupted] {
            assert_eq!(write_result(Err(std::io::Error::from(kind))).unwrap(), None);
            assert_eq!(
                io_result::<usize>(Err(std::io::Error::from(kind))).unwrap(),
                None
            );
        }
        assert_eq!(
            write_result(Err(std::io::Error::from(BrokenPipe)))
                .unwrap_err()
                .code,
            "EPIPE"
        );
        assert_eq!(
            io_result::<usize>(Err(std::io::Error::from(ConnectionReset)))
                .unwrap_err()
                .code,
            "ECONNRESET"
        );
        assert_eq!(io_result(Ok(0)).unwrap(), Some(0));
        assert_eq!(
            io_result(Ok(b"bytes".to_vec())).unwrap(),
            Some(b"bytes".to_vec())
        );
    }
    #[test]
    fn capacity_rejection_does_not_consume_or_wrap_handle_ids() {
        let mut next = 9;
        assert_eq!(allocate_id(&mut next, 4095).unwrap(), 10);
        for count in [4096, usize::MAX] {
            assert_eq!(allocate_id(&mut next, count).unwrap_err().code, "EMFILE");
            assert_eq!(next, 10);
        }
        next = u64::MAX;
        assert_eq!(allocate_id(&mut next, 0).unwrap_err().code, "EMFILE");
        assert_eq!(next, u64::MAX);
    }
    #[test]
    fn accept_budget_and_error_events_are_bounded_and_preserve_listener_identity() {
        let mut table = Transport::default();
        let id = table.listen(&json!({"host":"127.0.0.1","port":0})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let address = table.listeners[&id].socket.local_addr().unwrap();
        let clients: Vec<_> = (0..64)
            .map(|_| TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap())
            .collect();
        let mut events = Vec::new();
        table.accept(id, &mut events).unwrap();
        assert_eq!(events.len(), 64);
        assert_eq!(table.connections.len(), 64);
        assert!(events
            .iter()
            .all(|event| event["kind"] == "accept" && event["server"] == id));
        table.accept(id, &mut events).unwrap();
        assert_eq!(events.len(), 64);
        accept_completed(id, Err(fail("EMFILE", "accept handle limit")), &mut events);
        assert_eq!(events[64]["server"], id);
        assert_eq!(events[64]["kind"], "serverError");
        assert_eq!(events[64]["error"]["code"], "EMFILE");
        accept_completed(id, Ok(()), &mut events);
        assert_eq!(events.len(), 65);
        assert!(table.listeners.contains_key(&id));
        drop(clients);
    }
    #[test]
    fn native_response_reports_invalid_json_and_operation_errors() {
        assert_eq!(
            response("poll", "{")["error"]["code"],
            "ERR_INVALID_ARG_VALUE"
        );
        assert_eq!(
            response("missing", "{}")["error"]["code"],
            "ERR_TCP_UNSUPPORTED_OPERATION"
        );
        assert_eq!(
            response("listen", r#"{"host":"127.0.0.1","port":-1}"#)["error"]["code"],
            "ERR_INVALID_ARG_VALUE"
        );
        assert_eq!(response("close", r#"{"id":0}"#), json!({"value":null}));
    }
    #[test]
    fn actual_syscall_errors_and_connect_result_ownership_are_preserved() {
        assert_eq!(owned_socket(-1).unwrap_err().code, "ERR_TCP_IO");
        assert_eq!(cloexec(-1).unwrap_err().code, "ERR_TCP_IO");
        let socket = owned_socket(libc::AF_INET).unwrap();
        assert_eq!(
            socket_option(&socket, libc::SOL_SOCKET, -1, 0)
                .unwrap_err()
                .code,
            "ERR_TCP_IO"
        );
        drop(complete_connect(socket, Ok(())).unwrap());
        for error in [
            std::io::Error::from_raw_os_error(libc::EINPROGRESS),
            std::io::Error::from(std::io::ErrorKind::WouldBlock),
        ] {
            let socket = owned_socket(libc::AF_INET).unwrap();
            drop(complete_connect(socket, Err(error)).unwrap());
        }
        let socket = owned_socket(libc::AF_INET).unwrap();
        assert_eq!(
            complete_connect(
                socket,
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
            )
            .unwrap_err()
            .code,
            "ECONNREFUSED"
        );
    }
    #[test]
    fn io_error_mapping_and_retry_classification_remain_distinct() {
        use std::io::ErrorKind::*;
        for (kind, code) in [
            (AddrInUse, "EADDRINUSE"),
            (ConnectionRefused, "ECONNREFUSED"),
            (ConnectionReset, "ECONNRESET"),
            (BrokenPipe, "EPIPE"),
            (TimedOut, "ETIMEDOUT"),
            (Other, "ERR_TCP_IO"),
        ] {
            assert_eq!(io_error(std::io::Error::from(kind)).code, code);
        }
        assert!(retryable(&std::io::Error::from(WouldBlock)));
        assert!(retryable(&std::io::Error::from(Interrupted)));
        assert!(!retryable(&std::io::Error::from(BrokenPipe)));
        assert!(pending(&std::io::Error::from_raw_os_error(libc::ENOTCONN)));
        assert!(!pending(&std::io::Error::from(ConnectionRefused)));
    }
}
