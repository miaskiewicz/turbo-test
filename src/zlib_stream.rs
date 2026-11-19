//! Stateful zlib handles behind the runtime's streaming zlib classes (turbo-test#404).
//!
//! `src/node_zlib.js` opens one handle per stream with `__ttZlibOpen(mode, level)` (Node's
//! `zlib.constants` modes DEFLATE..UNZIP), feeds every chunk through
//! `__ttZlibWrite(id, bytes, flush)` (Z_NO_FLUSH, Z_SYNC_FLUSH, Z_FULL_FLUSH or Z_FINISH) and
//! gets back the bytes that chunk produced, so compression state carries across chunks as
//! in Node. A Z_FINISH write, an error and `__ttZlibClose(id)` drop the handle.
//!
//! Deflate and raw deflate run on flate2's `Compress` / `Decompress` (miniz_oxide here, so
//! compressed bytes differ from Node's zlib while decoding either way round works). Gzip frames
//! a raw deflate body itself with Node's header (XFL by level, the platform OS byte) and its
//! CRC-32 / ISIZE trailer; Gunzip parses members incrementally, as zlib does, and stops at
//! bytes after a member that do not start another one. Unzip picks gzip or zlib from the
//! first two bytes. Errors carry Node's codes: malformed input is Z_DATA_ERROR (-3), input
//! that ends early is Z_BUF_ERROR (-5) "unexpected end of file".

use flate2::{Compress, Compression, Crc, Decompress, FlushCompress, FlushDecompress, Status};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

pub const DEFLATE: u32 = 1;
pub const INFLATE: u32 = 2;
pub const GZIP: u32 = 3;
pub const GUNZIP: u32 = 4;
pub const DEFLATERAW: u32 = 5;
pub const INFLATERAW: u32 = 6;
pub const UNZIP: u32 = 7;
pub const NO_FLUSH: u32 = 0;
pub const SYNC_FLUSH: u32 = 2;
pub const FULL_FLUSH: u32 = 3;
pub const FINISH: u32 = 4;

#[derive(Debug, PartialEq)]
pub enum ZlibError {
    /// Malformed input (Node's Z_DATA_ERROR), with zlib's message.
    Data(&'static str),
    /// The input ended inside a stream (Node's Z_BUF_ERROR).
    Truncated,
    /// A write to a handle that was finished, failed or closed.
    Closed,
}

impl ZlibError {
    /// Node's message, `code` and `errno` for this error.
    pub fn node(&self) -> (&'static str, &'static str, Option<i32>) {
        match self {
            ZlibError::Data(message) => (message, "Z_DATA_ERROR", Some(-3)),
            ZlibError::Truncated => ("unexpected end of file", "Z_BUF_ERROR", Some(-5)),
            ZlibError::Closed => ("zlib binding closed", "ERR_ZLIB_BINDING_CLOSED", None),
        }
    }
}

type Result<T> = std::result::Result<T, ZlibError>;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
#[cfg(target_os = "macos")]
const GZIP_OS: u8 = 19;
#[cfg(not(target_os = "macos"))]
const GZIP_OS: u8 = 3;

/// A decompressor over zlib-wrapped or raw deflate data.
struct Inflater {
    body: Decompress,
    ended: bool,
}

/// A gzip compressor: header (until written), raw deflate body, CRC of the input.
struct GzipOut {
    header: Option<[u8; 10]>,
    body: Compress,
    crc: Crc,
}

enum Member {
    Header(Vec<u8>),
    Body,
    Trailer(Vec<u8>),
    Done,
}

/// A gzip decompressor walking member header, body and trailer as bytes arrive.
struct GzipIn {
    member: Member,
    body: Decompress,
    crc: Crc,
}

enum Codec {
    Deflate(Compress),
    Inflate(Inflater),
    Gzip(GzipOut),
    Gunzip(GzipIn),
    Unzip(Vec<u8>),
}

fn data_error(_: impl std::error::Error) -> ZlibError {
    ZlibError::Data("invalid compressed data")
}

fn grow(out: &mut Vec<u8>) {
    if out.len() == out.capacity() {
        out.reserve(out.capacity().max(256));
    }
}

fn compress_flush(flush: u32) -> FlushCompress {
    match flush {
        SYNC_FLUSH => FlushCompress::Sync,
        FULL_FLUSH => FlushCompress::Full,
        FINISH => FlushCompress::Finish,
        _ => FlushCompress::None,
    }
}

/// Whether a (de)compression round leaves nothing to do: all input taken with output room
/// to spare (so any flush is complete), or no progress at all.
fn settled(input: &[u8], out: &[u8], room: usize, progressed: bool) -> bool {
    (input.is_empty() && out.len() < room) || !progressed
}

/// Compresses all of `input`, then the flush, appending to `out`.
fn compress(
    c: &mut Compress,
    mut input: &[u8],
    flush: FlushCompress,
    out: &mut Vec<u8>,
) -> Result<()> {
    loop {
        grow(out);
        let (before, written) = (c.total_in(), out.len());
        let status = c.compress_vec(input, out, flush).map_err(data_error)?;
        input = &input[(c.total_in() - before) as usize..];
        let progressed = c.total_in() > before || out.len() > written;
        if status == Status::StreamEnd || settled(input, out, out.capacity(), progressed) {
            return Ok(());
        }
    }
}

/// Decompresses `input` until it is used up or the deflate stream ends; returns the bytes
/// after the end of the stream.
fn inflate<'a>(
    d: &mut Decompress,
    mut input: &'a [u8],
    out: &mut Vec<u8>,
) -> Result<Option<&'a [u8]>> {
    loop {
        grow(out);
        let (before, written) = (d.total_in(), out.len());
        let status = d
            .decompress_vec(input, out, FlushDecompress::None)
            .map_err(data_error)?;
        input = &input[(d.total_in() - before) as usize..];
        if status == Status::StreamEnd {
            return Ok(Some(input));
        }
        let progressed = d.total_in() > before || out.len() > written;
        if settled(input, out, out.capacity(), progressed) {
            return Ok(None);
        }
    }
}

impl Inflater {
    fn new(zlib: bool) -> Self {
        Inflater {
            body: Decompress::new(zlib),
            ended: false,
        }
    }

    fn write(&mut self, input: &[u8], finish: bool, out: &mut Vec<u8>) -> Result<()> {
        if !self.ended {
            self.ended = inflate(&mut self.body, input, out)?.is_some();
        }
        if finish && !self.ended {
            return Err(ZlibError::Truncated);
        }
        Ok(())
    }
}

impl GzipOut {
    fn new(level: u32) -> Self {
        let xfl = match level {
            9 => 2,
            0 | 1 => 4,
            _ => 0,
        };
        GzipOut {
            header: Some([0x1f, 0x8b, 8, 0, 0, 0, 0, 0, xfl, GZIP_OS]),
            body: Compress::new(Compression::new(level), false),
            crc: Crc::new(),
        }
    }

    fn write(&mut self, input: &[u8], flush: u32, out: &mut Vec<u8>) -> Result<()> {
        if let Some(header) = self.header.take() {
            out.extend_from_slice(&header);
        }
        self.crc.update(input);
        compress(&mut self.body, input, compress_flush(flush), out)?;
        if flush == FINISH {
            out.extend_from_slice(&self.crc.sum().to_le_bytes());
            out.extend_from_slice(&self.crc.amount().to_le_bytes());
        }
        Ok(())
    }
}

/// The offset after a NUL-terminated header field starting at `at`.
fn after_nul(buf: &[u8], at: usize) -> Option<usize> {
    let field = buf.get(at..)?;
    Some(at + field.iter().position(|&b| b == 0)? + 1)
}

/// The offset after the fixed header and FEXTRA, if FLG announces one.
fn after_extra(buf: &[u8], flags: u8) -> Option<usize> {
    if flags & 4 == 0 {
        return Some(10);
    }
    Some(12 + u16::from_le_bytes([*buf.get(10)?, *buf.get(11)?]) as usize)
}

/// The length of the optional header fields FLG announces, once all of them have arrived.
fn header_fields(buf: &[u8], flags: u8) -> Option<usize> {
    let mut at = after_extra(buf, flags)?;
    for field in [8, 16] {
        if flags & field != 0 {
            at = after_nul(buf, at)?;
        }
    }
    at += usize::from(flags & 2);
    (at <= buf.len()).then_some(at)
}

/// The gzip header length once `buf` holds all of it, `None` while more is needed.
fn header_len(buf: &[u8]) -> Result<Option<usize>> {
    if !GZIP_MAGIC.starts_with(&buf[..buf.len().min(2)]) {
        return Err(ZlibError::Data("incorrect header check"));
    }
    if buf.len() < 10 {
        return Ok(None);
    }
    if buf[2] != 8 {
        return Err(ZlibError::Data("unknown compression method"));
    }
    Ok(header_fields(buf, buf[3]))
}

/// The unread tail of `input`, which ends the `buffered` bytes of which `used` were read.
fn rest(input: &[u8], buffered: usize, used: usize) -> &[u8] {
    &input[input.len() - (buffered - used)..]
}

impl GzipIn {
    fn new() -> Self {
        GzipIn {
            member: Member::Header(Vec::new()),
            body: Decompress::new(false),
            crc: Crc::new(),
        }
    }

    fn header<'a>(&mut self, mut buf: Vec<u8>, input: &'a [u8]) -> Result<&'a [u8]> {
        buf.extend_from_slice(input);
        let Some(len) = header_len(&buf)? else {
            self.member = Member::Header(buf);
            return Ok(&[]);
        };
        self.member = Member::Body;
        Ok(rest(input, buf.len(), len))
    }

    fn body<'a>(&mut self, input: &'a [u8], out: &mut Vec<u8>) -> Result<&'a [u8]> {
        let start = out.len();
        let ended = inflate(&mut self.body, input, out)?;
        self.crc.update(&out[start..]);
        let Some(after) = ended else { return Ok(&[]) };
        self.member = Member::Trailer(Vec::new());
        Ok(after)
    }

    fn trailer<'a>(&mut self, mut buf: Vec<u8>, input: &'a [u8]) -> Result<&'a [u8]> {
        let take = input.len().min(8 - buf.len());
        buf.extend_from_slice(&input[..take]);
        if buf.len() < 8 {
            self.member = Member::Trailer(buf);
            return Ok(&[]);
        }
        if buf[..4] != self.crc.sum().to_le_bytes() {
            return Err(ZlibError::Data("incorrect data check"));
        }
        if buf[4..] != self.crc.amount().to_le_bytes() {
            return Err(ZlibError::Data("incorrect length check"));
        }
        self.member = Member::Done;
        Ok(&input[take..])
    }

    /// After a member: another member starts with the magic byte, anything else is ignored.
    fn next<'a>(&mut self, input: &'a [u8]) -> &'a [u8] {
        if input[0] != GZIP_MAGIC[0] {
            return &[];
        }
        *self = GzipIn::new();
        input
    }

    fn step<'a>(&mut self, input: &'a [u8], out: &mut Vec<u8>) -> Result<&'a [u8]> {
        match std::mem::replace(&mut self.member, Member::Done) {
            Member::Header(buf) => self.header(buf, input),
            Member::Body => {
                self.member = Member::Body;
                self.body(input, out)
            }
            Member::Trailer(buf) => self.trailer(buf, input),
            Member::Done => Ok(self.next(input)),
        }
    }

    fn write(&mut self, mut input: &[u8], finish: bool, out: &mut Vec<u8>) -> Result<()> {
        while !input.is_empty() {
            input = self.step(input, out)?;
        }
        if finish && !matches!(self.member, Member::Done) {
            return Err(ZlibError::Truncated);
        }
        Ok(())
    }
}

/// The codec Unzip settles on once the first bytes tell gzip from zlib.
fn unzip_choice(pending: &[u8]) -> Option<Codec> {
    match pending {
        [] | [0x1f] => None,
        [0x1f, 0x8b, ..] => Some(Codec::Gunzip(GzipIn::new())),
        _ => Some(Codec::Inflate(Inflater::new(true))),
    }
}

impl Codec {
    fn new(mode: u32, level: u32) -> Option<Codec> {
        let compression = Compression::new(level);
        Some(match mode {
            DEFLATE | DEFLATERAW => Codec::Deflate(Compress::new(compression, mode == DEFLATE)),
            INFLATE | INFLATERAW => Codec::Inflate(Inflater::new(mode == INFLATE)),
            GZIP => Codec::Gzip(GzipOut::new(level)),
            GUNZIP => Codec::Gunzip(GzipIn::new()),
            UNZIP => Codec::Unzip(Vec::new()),
            _ => return None,
        })
    }

    fn unzip(&mut self, mut pending: Vec<u8>, input: &[u8], flush: u32) -> Result<Vec<u8>> {
        pending.extend_from_slice(input);
        let Some(codec) = unzip_choice(&pending) else {
            *self = Codec::Unzip(pending);
            return if flush == FINISH {
                Err(ZlibError::Truncated)
            } else {
                Ok(Vec::new())
            };
        };
        *self = codec;
        self.write(&pending, flush)
    }

    fn write(&mut self, input: &[u8], flush: u32) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(input.len() / 2 + 64);
        let finish = flush == FINISH;
        match self {
            Codec::Deflate(c) => compress(c, input, compress_flush(flush), &mut out)?,
            Codec::Inflate(i) => i.write(input, finish, &mut out)?,
            Codec::Gzip(g) => g.write(input, flush, &mut out)?,
            Codec::Gunzip(g) => g.write(input, finish, &mut out)?,
            Codec::Unzip(pending) => {
                let pending = std::mem::take(pending);
                return self.unzip(pending, input, flush);
            }
        }
        Ok(out)
    }
}

thread_local! {
    static OPEN: RefCell<HashMap<u64, Codec>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

/// Opens a handle for Node zlib `mode` at compression `level` (0-9), or `None` for an
/// unknown mode.
pub fn open(mode: u32, level: u32) -> Option<u64> {
    let codec = Codec::new(mode, level.min(9))?;
    let id = NEXT_ID.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    OPEN.with(|open| open.borrow_mut().insert(id, codec));
    Some(id)
}

/// Feeds `input` with `flush` through handle `id`, returning the bytes it produced. A
/// Z_FINISH write or an error closes the handle.
pub fn write(id: u64, input: &[u8], flush: u32) -> Result<Vec<u8>> {
    let mut codec = OPEN
        .with(|open| open.borrow_mut().remove(&id))
        .ok_or(ZlibError::Closed)?;
    let out = codec.write(input, flush)?;
    if flush != FINISH {
        OPEN.with(|open| open.borrow_mut().insert(id, codec));
    }
    Ok(out)
}

/// Drops handle `id` (a no-op for one already closed).
pub fn close(id: u64) {
    OPEN.with(|open| open.borrow_mut().remove(&id));
}

/// Drops every handle this thread holds (between test files).
pub fn reset() {
    OPEN.with(|open| open.borrow_mut().clear());
}

/// The number of open handles on this thread.
pub fn open_count() -> usize {
    OPEN.with(|open| open.borrow().len())
}

fn throw_zlib_error(scope: &mut v8::PinScope, error: ZlibError) {
    let (message, code, errno) = error.node();
    let text = v8::String::new(scope, message).unwrap();
    let exception = v8::Exception::error(scope, text);
    let object = exception.to_object(scope).unwrap();
    let key = v8::String::new(scope, "code").unwrap();
    let value = v8::String::new(scope, code).unwrap();
    object.set(scope, key.into(), value.into());
    if let Some(errno) = errno {
        let key = v8::String::new(scope, "errno").unwrap();
        let value = v8::Integer::new(scope, errno);
        object.set(scope, key.into(), value.into());
    }
    scope.throw_exception(exception);
}

fn handle_id(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> u64 {
    value.integer_value(scope).unwrap_or(0).max(0) as u64
}

/// `__ttZlibOpen(mode, level)`: a handle id; a TypeError for an unknown mode.
pub fn native_zlib_open(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let mode = args.get(0).uint32_value(scope).unwrap_or(0);
    let level = args.get(1).uint32_value(scope).unwrap_or(6);
    let Some(id) = open(mode, level) else {
        let text = v8::String::new(scope, "invalid zlib mode").unwrap();
        let error = v8::Exception::type_error(scope, text);
        scope.throw_exception(error);
        return;
    };
    rv.set_double(id as f64);
}

/// `__ttZlibWrite(id, bytes, flush)`: the produced bytes as a Uint8Array, or a thrown Error
/// carrying Node's `code` / `errno`.
pub fn native_zlib_write(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let id = handle_id(scope, args.get(0));
    let Some(input) = crate::node_compat::bytes(args.get(1)) else {
        return crate::node_compat::throw_error(scope, "Expected a byte view");
    };
    let flush = args.get(2).uint32_value(scope).unwrap_or(NO_FLUSH);
    match write(id, &input, flush) {
        Ok(out) => crate::node_compat::return_bytes(scope, &mut rv, out),
        Err(error) => throw_zlib_error(scope, error),
    }
}

/// `__ttZlibClose(id)`: drops the handle; `__ttZlibClose()` drops them all.
pub fn native_zlib_close(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    if args.get(0).is_undefined() {
        return reset();
    }
    close(handle_id(scope, args.get(0)));
}

#[cfg(test)]
#[path = "zlib_stream_tests.rs"]
mod tests;
