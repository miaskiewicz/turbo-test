//! Nonblocking loopback HTTP transport. No V8 handles or callbacks cross native threads.
use base64::Engine;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

#[derive(Debug)]
struct Failure {
    code: String,
    message: String,
}
type Result<T> = std::result::Result<T, Failure>;
fn fail(code: &str, message: &str) -> Failure {
    Failure {
        code: code.into(),
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
        _ => "ERR_HTTP_IO",
    };
    fail(code, &error.to_string())
}
fn error_value(error: &Failure) -> Value {
    json!({"code":error.code,"message":error.message})
}
fn address(host: &str, port: u16) -> Result<SocketAddr> {
    let host = if host == "localhost" {
        "127.0.0.1"
    } else {
        host
    };
    let ip: IpAddr = host.trim_matches(['[', ']']).parse().map_err(|_| {
        fail(
            "ERR_HTTP_UNSUPPORTED_HOST",
            "Only literal loopback addresses and localhost are supported",
        )
    })?;
    if !ip.is_loopback() {
        return Err(fail(
            "ERR_HTTP_UNSUPPORTED_HOST",
            "Only loopback HTTP is supported",
        ));
    }
    Ok(SocketAddr::new(ip, port))
}
fn encoded(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
fn decoded(value: &Value) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value.as_str().unwrap_or(""))
        .map_err(|_| fail("ERR_INVALID_ARG_VALUE", "Invalid byte encoding"))
}
#[derive(Clone, Copy)]
enum Phase {
    Headers,
    Fixed(usize),
    ChunkHead,
    ChunkBody(usize),
    ChunkCrLf,
    Trailers,
    Eof,
    Done,
}
struct Decoder {
    input: Vec<u8>,
    phase: Phase,
    request: bool,
    head: bool,
    body: usize,
}
impl Decoder {
    fn new(request: bool, head: bool) -> Self {
        Self {
            input: Vec::new(),
            phase: Phase::Headers,
            request,
            head,
            body: 0,
        }
    }
    fn feed(&mut self, id: u64, data: &[u8], events: &mut Vec<Value>) -> Result<()> {
        self.input.extend_from_slice(data);
        if self.input.len() > 65536 {
            return Err(fail("HPE_HEADER_OVERFLOW", "HTTP framing exceeds 64 KiB"));
        }
        while self.step(id, events)? {}
        Ok(())
    }
    fn step(&mut self, id: u64, events: &mut Vec<Value>) -> Result<bool> {
        match self.phase {
            Phase::Headers => self.headers(id, events),
            Phase::Fixed(left) => self.fixed(id, left, events),
            Phase::ChunkHead => self.chunk_head(),
            Phase::ChunkBody(left) => self.chunk_body(id, left, events),
            Phase::ChunkCrLf => self.chunk_crlf(),
            Phase::Trailers => self.trailers(id, events),
            Phase::Eof => self.until_eof(id, events),
            Phase::Done => Ok(false),
        }
    }
    fn headers(&mut self, id: u64, events: &mut Vec<Value>) -> Result<bool> {
        let Some(end) = separator(&self.input, b"\r\n\r\n") else {
            if self.input.len() > 65536 {
                return Err(fail("HPE_HEADER_OVERFLOW", "HTTP headers exceed 64 KiB"));
            }
            return Ok(false);
        };
        let header: String = self.input.drain(..end + 4).map(char::from).collect();
        let (line, headers) = parse_headers(&header)?;
        self.phase = body_phase(&line, &headers, self.request, self.head)?;
        events.push(json!({"kind":"headers","id":id,"line":line,"headers":headers}));
        if matches!(self.phase, Phase::Fixed(0)) {
            self.end(id, events);
        }
        Ok(true)
    }
    fn data(&mut self, id: u64, count: usize, events: &mut Vec<Value>) -> Result<()> {
        self.body += count;
        if self.body > 8 * 1024 * 1024 {
            return Err(fail("ERR_HTTP_BODY_TOO_LARGE", "HTTP body exceeds 8 MiB"));
        }
        let bytes: Vec<u8> = self.input.drain(..count).collect();
        if !bytes.is_empty() {
            events.push(json!({"kind":"data","id":id,"data":encoded(&bytes)}));
        }
        Ok(())
    }
    fn end(&mut self, id: u64, events: &mut Vec<Value>) {
        self.phase = Phase::Done;
        events.push(json!({"kind":"end","id":id}));
    }
    fn fixed(&mut self, id: u64, left: usize, events: &mut Vec<Value>) -> Result<bool> {
        if self.input.is_empty() {
            return Ok(false);
        }
        let count = left.min(self.input.len());
        self.data(id, count, events)?;
        self.phase = Phase::Fixed(left - count);
        if left == count {
            self.end(id, events);
        }
        Ok(true)
    }
    fn chunk_head(&mut self) -> Result<bool> {
        let Some(end) = separator(&self.input, b"\r\n") else {
            return Ok(false);
        };
        let bytes: Vec<u8> = self.input.drain(..end + 2).collect();
        let text = std::str::from_utf8(&bytes[..end])
            .map_err(|_| fail("HPE_INVALID_CHUNK_SIZE", "Invalid chunk size"))?;
        let size = usize::from_str_radix(text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| fail("HPE_INVALID_CHUNK_SIZE", "Invalid chunk size"))?;
        self.phase = if size == 0 {
            Phase::Trailers
        } else {
            Phase::ChunkBody(size)
        };
        Ok(true)
    }
    fn chunk_body(&mut self, id: u64, left: usize, events: &mut Vec<Value>) -> Result<bool> {
        if self.input.is_empty() {
            return Ok(false);
        }
        let count = left.min(self.input.len());
        self.data(id, count, events)?;
        self.phase = if left == count {
            Phase::ChunkCrLf
        } else {
            Phase::ChunkBody(left - count)
        };
        Ok(true)
    }
    fn chunk_crlf(&mut self) -> Result<bool> {
        if self.input.len() < 2 {
            return Ok(false);
        }
        if self.input[..2] != *b"\r\n" {
            return Err(fail("HPE_INVALID_CHUNK_SIZE", "Invalid chunk terminator"));
        }
        self.input.drain(..2);
        self.phase = Phase::ChunkHead;
        Ok(true)
    }
    fn trailers(&mut self, id: u64, events: &mut Vec<Value>) -> Result<bool> {
        let count = if self.input.starts_with(b"\r\n") {
            2
        } else {
            let Some(end) = separator(&self.input, b"\r\n\r\n") else {
                return Ok(false);
            };
            end + 4
        };
        self.input.drain(..count);
        self.end(id, events);
        Ok(true)
    }
    fn until_eof(&mut self, id: u64, events: &mut Vec<Value>) -> Result<bool> {
        if self.input.is_empty() {
            return Ok(false);
        }
        self.data(id, self.input.len(), events)?;
        Ok(false)
    }
    fn eof(&mut self, id: u64, events: &mut Vec<Value>) -> Result<()> {
        if matches!(self.phase, Phase::Eof) {
            self.end(id, events);
        }
        if !matches!(self.phase, Phase::Done) {
            return Err(fail(
                "ECONNRESET",
                "Socket closed before HTTP message completed",
            ));
        }
        Ok(())
    }
}
fn separator(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).position(|part| part == needle)
}
fn parse_headers(text: &str) -> Result<(String, Vec<(String, String)>)> {
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or("").to_owned();
    let minimum = if first.starts_with("HTTP/") { 2 } else { 3 };
    if first.split(' ').count() < minimum {
        return Err(fail("HPE_INVALID_CONSTANT", "Invalid HTTP start line"));
    }
    let mut headers = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((key, value)) = line.split_once(':') else {
            return Err(fail("HPE_INVALID_HEADER_TOKEN", "Invalid HTTP header"));
        };
        if !valid_header_name(key) {
            return Err(fail("HPE_INVALID_HEADER_TOKEN", "Invalid HTTP header name"));
        }
        headers.push((key.to_owned(), value.trim().to_owned()));
    }
    Ok((first, headers))
}
fn valid_header_name(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
fn body_phase(
    line: &str,
    headers: &[(String, String)],
    request: bool,
    head: bool,
) -> Result<Phase> {
    let transfer = header(headers, "transfer-encoding");
    let length = header(headers, "content-length");
    validate_framing(headers, transfer, length)?;
    validate_message_line(line, request)?;
    if no_body(line, request, head) {
        return Ok(Phase::Fixed(0));
    }
    if let Some(value) = transfer {
        return transfer_phase(value);
    }
    if let Some(value) = length {
        return content_length_phase(value);
    }

    Ok(if request { Phase::Fixed(0) } else { Phase::Eof })
}
fn validate_framing(
    headers: &[(String, String)],
    transfer: Option<&str>,
    length: Option<&str>,
) -> Result<()> {
    if transfer.is_some() && length.is_some() {
        return Err(fail(
            "HPE_UNEXPECTED_CONTENT_LENGTH",
            "Conflicting HTTP body framing",
        ));
    }
    if headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .count()
        > 1
    {
        return Err(fail(
            "HPE_UNEXPECTED_CONTENT_LENGTH",
            "Duplicate content length",
        ));
    }
    if headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding"))
        .count()
        > 1
    {
        return Err(fail(
            "HPE_INVALID_TRANSFER_ENCODING",
            "Duplicate transfer encoding",
        ));
    }
    Ok(())
}
fn header<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str())
}
fn content_length_phase(value: &str) -> Result<Phase> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(fail("HPE_INVALID_CONTENT_LENGTH", "Invalid content length"));
    }
    value
        .parse()
        .map(Phase::Fixed)
        .map_err(|_| fail("HPE_INVALID_CONTENT_LENGTH", "Invalid content length"))
}
fn valid_version(value: &str) -> bool {
    matches!(value, "HTTP/1.0" | "HTTP/1.1")
}
fn valid_request_line(parts: &[&str]) -> bool {
    parts.len() == 3
        && valid_header_name(parts[0])
        && !parts[1].is_empty()
        && valid_version(parts[2])
}
fn valid_response_line(parts: &[&str]) -> bool {
    parts.len() >= 2
        && valid_version(parts[0])
        && parts[1].len() == 3
        && parts[1].bytes().all(|byte| byte.is_ascii_digit())
}
fn validate_message_line(line: &str, request: bool) -> Result<()> {
    validate_start_line(line, request)?;
    validate_response(line, request)
}
fn validate_start_line(line: &str, request: bool) -> Result<()> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    let valid = if request {
        valid_request_line(&parts)
    } else {
        valid_response_line(&parts)
    };
    if valid {
        Ok(())
    } else {
        Err(fail("HPE_INVALID_CONSTANT", "Invalid HTTP start line"))
    }
}
fn no_body(line: &str, request: bool, head: bool) -> bool {
    if request {
        return false;
    }
    let status = line.split(' ').nth(1).unwrap_or("");
    head || status == "204" || status == "304"
}
fn validate_response(line: &str, request: bool) -> Result<()> {
    if request {
        return Ok(());
    }
    let status = line
        .split(' ')
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..=999).contains(&status) {
        return Err(fail(
            "HPE_INVALID_STATUS",
            "Informational responses and upgrades are not supported",
        ));
    }
    Ok(())
}
fn transfer_phase(value: &str) -> Result<Phase> {
    if value.eq_ignore_ascii_case("chunked") {
        Ok(Phase::ChunkHead)
    } else {
        Err(fail(
            "HPE_INVALID_TRANSFER_ENCODING",
            "Unsupported HTTP transfer encoding",
        ))
    }
}
struct Connection {
    socket: TcpStream,
    decoder: Decoder,
    output: Vec<u8>,
    sent: usize,
    finish: bool,
    pending_ack: bool,
}
impl Connection {
    fn new(socket: TcpStream, request: bool, head: bool) -> Result<Self> {
        socket.set_nonblocking(true).map_err(io_error)?;
        socket.set_nodelay(true).map_err(io_error)?;
        Ok(Self {
            socket,
            decoder: Decoder::new(request, head),
            output: Vec::new(),
            sent: 0,
            finish: false,
            pending_ack: false,
        })
    }
    fn write(&mut self, id: u64, events: &mut Vec<Value>) -> Result<bool> {
        if self.pending_ack && self.flush()? {
            self.output.clear();
            self.sent = 0;
            self.pending_ack = false;
            events.push(json!({"kind":"drain","id":id}));
        }
        Ok(self.finish && self.output.is_empty())
    }
    fn flush(&mut self) -> Result<bool> {
        while self.sent < self.output.len() {
            match self.socket.write(&self.output[self.sent..]) {
                Ok(0) => return Err(fail("EPIPE", "Socket write returned zero")),
                Ok(count) => self.sent += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(true)
    }
    fn read(&mut self, id: u64, events: &mut Vec<Value>) -> Result<bool> {
        let mut bytes = [0; 16384];
        loop {
            match self.socket.read(&mut bytes) {
                Ok(0) => {
                    self.decoder.eof(id, events)?;
                    return Ok(true);
                }
                Ok(count) => self.decoder.feed(id, &bytes[..count], events)?,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(io_error(error)),
            }
        }
    }
}
#[derive(Default)]
struct Transport {
    next: u64,
    listeners: HashMap<u64, TcpListener>,
    connections: HashMap<u64, Connection>,
}
impl Transport {
    fn id(&mut self) -> Result<u64> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| fail("ENOSPC", "HTTP resource identifiers exhausted"))?;
        Ok(self.next)
    }
    fn listen(&mut self, args: &Value) -> Result<Value> {
        let listener = TcpListener::bind(arg_address(args)?).map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let address = listener.local_addr().map_err(io_error)?;
        let id = self.id()?;
        self.listeners.insert(id, listener);
        Ok(
            json!({"id":id,"port":address.port(),"address":address.ip().to_string(),"family":if address.is_ipv4(){"IPv4"}else{"IPv6"}}),
        )
    }
    fn connect(&mut self, args: &Value) -> Result<Value> {
        let socket = TcpStream::connect_timeout(&arg_address(args)?, Duration::from_millis(250))
            .map_err(io_error)?;
        let info = socket_info(&socket)?;
        let connection = Connection::new(socket, false, args["head"].as_bool().unwrap_or(false))?;
        let id = self.id()?;
        self.connections.insert(id, connection);
        Ok(json!({"id":id,"socket":info}))
    }
    fn send(&mut self, args: &Value) -> Result<Value> {
        let id = args["id"].as_u64().unwrap_or(0);
        let bytes = decoded(&args["data"])?;
        let Some(connection) = self.connections.get_mut(&id) else {
            return Err(fail("ERR_SOCKET_CLOSED", "HTTP socket is closed"));
        };
        if connection.output.len() + bytes.len() > 8 * 1024 * 1024 {
            return Err(fail(
                "ERR_HTTP_BODY_TOO_LARGE",
                "HTTP pending writes exceed 8 MiB",
            ));
        }
        connection.output.extend(bytes);
        connection.pending_ack = true;
        connection.finish |= args["finish"].as_bool().unwrap_or(false);
        Ok(Value::Null)
    }
    fn accept(&mut self, events: &mut Vec<Value>) -> Result<()> {
        let mut accepted = Vec::new();
        for (&server, listener) in &self.listeners {
            accept_all(server, listener, &mut accepted)?;
        }
        for (server, socket) in accepted {
            let info = socket_info(&socket)?;
            let id = self.id()?;
            self.connections
                .insert(id, Connection::new(socket, true, false)?);
            events.push(json!({"kind":"accept","id":id,"server":server,"socket":info}));
        }
        Ok(())
    }
    fn poll(&mut self) -> Result<Value> {
        let mut events = Vec::new();
        self.accept(&mut events)?;
        self.connections
            .retain(|&id, connection| poll_connection(id, connection, &mut events));
        Ok(Value::Array(events))
    }
    fn close(&mut self, args: &Value) -> Value {
        let id = args["id"].as_u64().unwrap_or(0);
        self.listeners.remove(&id);
        self.connections.remove(&id);
        Value::Null
    }
    fn reset(&mut self) -> Value {
        self.listeners.clear();
        self.connections.clear();
        Value::Null
    }
}
fn socket_info(socket: &TcpStream) -> Result<Value> {
    let local = socket.local_addr().map_err(io_error)?;
    let remote = socket.peer_addr().map_err(io_error)?;
    Ok(
        json!({"localAddress":local.ip().to_string(),"localPort":local.port(),"remoteAddress":remote.ip().to_string(),"remotePort":remote.port()}),
    )
}
fn arg_address(args: &Value) -> Result<SocketAddr> {
    let port = args["port"]
        .as_u64()
        .filter(|port| *port <= 65535)
        .ok_or_else(|| fail("ERR_SOCKET_BAD_PORT", "Invalid HTTP port"))?;
    address(args["host"].as_str().unwrap_or("127.0.0.1"), port as u16)
}
fn accept_all(
    server: u64,
    listener: &TcpListener,
    accepted: &mut Vec<(u64, TcpStream)>,
) -> Result<()> {
    loop {
        match listener.accept() {
            Ok((socket, _)) => accepted.push((server, socket)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(io_error(error)),
        }
    }
}
fn poll_connection(id: u64, connection: &mut Connection, events: &mut Vec<Value>) -> bool {
    let result = connection.write(id, events).and_then(|closed| {
        if closed {
            Ok(true)
        } else {
            connection.read(id, events)
        }
    });
    match result {
        Ok(false) => true,
        Ok(true) => {
            events.push(json!({"kind":"close","id":id}));
            false
        }
        Err(error) => {
            events.push(json!({"kind":"close","id":id,"error":error_value(&error)}));
            false
        }
    }
}
thread_local! { static TRANSPORT: RefCell<Transport> = RefCell::new(Transport::default()); }
fn operation(name: &str, args: &Value) -> Result<Value> {
    TRANSPORT.with(|table| {
        let mut table = table.borrow_mut();
        match name {
            "listen" => table.listen(args),
            "connect" => table.connect(args),
            "send" => table.send(args),
            "poll" => table.poll(),
            "close" => Ok(table.close(args)),
            "reset" => Ok(table.reset()),
            _ => Err(fail("ERR_INVALID_ARG_VALUE", "Unknown HTTP operation")),
        }
    })
}
/// Close all transport resources when a worker starts a fresh test context.
pub fn reset() {
    TRANSPORT.with(|table| {
        table.borrow_mut().reset();
    });
}
pub fn native_http(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let encoded = args.get(1).to_rust_string_lossy(scope);
    let result = serde_json::from_str(&encoded)
        .map_err(|_| fail("ERR_INVALID_ARG_VALUE", "Invalid HTTP arguments"))
        .and_then(|args| operation(&name, &args));
    let value = match result {
        Ok(value) => json!({"value":value}),
        Err(error) => json!({"error":error_value(&error)}),
    };
    let text = v8::String::new(scope, &value.to_string()).unwrap();
    rv.set(text.into());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunked_binary_frames_parse_incrementally() {
        let mut decoder = Decoder::new(false, false);
        let mut events = Vec::new();
        let mut wire =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3;extension=yes\r\n".to_vec();
        wire.extend([0, 255, 128]);
        wire.extend(b"\r\n0\r\nX-Trailer: yes\r\n\r\n");
        for byte in wire {
            decoder.feed(7, &[byte], &mut events).unwrap();
        }
        let mut body = Vec::new();
        for event in &events {
            if event["kind"] == "data" {
                body.extend(decoded(&event["data"]).unwrap());
            }
        }
        assert_eq!(body, [0, 255, 128]);
        assert_eq!(
            events.iter().filter(|event| event["kind"] == "end").count(),
            1
        );
        assert!(matches!(decoder.phase, Phase::Done));
    }
    #[test]
    fn invalid_framing_and_truncated_bodies_fail() {
        let invalid: &[&[u8]] = &[
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nBad Header: value\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\ng\r\n",
            b"HTTP/1.1 101 Switching Protocols\r\n\r\n",
            b"GARBAGE 200 OK\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: +1\r\n\r\na",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: gzip\r\n\r\n",
        ];
        for wire in invalid {
            assert!(Decoder::new(false, false)
                .feed(1, wire, &mut Vec::new())
                .is_err());
        }
        let mut decoder = Decoder::new(false, false);
        let mut events = Vec::new();
        decoder
            .feed(
                1,
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\na",
                &mut events,
            )
            .unwrap();
        assert_eq!(decoder.eof(1, &mut events).unwrap_err().code, "ECONNRESET");
        assert!(Decoder::new(true, false)
            .feed(1, &vec![b'a'; 65537], &mut events)
            .is_err());
    }
    #[test]
    fn bodyless_requests_and_eof_responses_finish_truthfully() {
        let mut request = Decoder::new(true, false);
        let mut events = Vec::new();
        request
            .feed(1, b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n", &mut events)
            .unwrap();
        assert!(matches!(request.phase, Phase::Done));
        let mut response = Decoder::new(false, false);
        events.clear();
        response
            .feed(2, b"HTTP/1.0 200 OK\r\n\r\nhello", &mut events)
            .unwrap();
        assert!(!events.iter().any(|event| event["kind"] == "end"));
        response.eof(2, &mut events).unwrap();
        assert_eq!(events.last().unwrap()["kind"], "end");
    }
}
