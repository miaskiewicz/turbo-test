//! The legacy encodings of the runtime's `TextDecoder` (turbo-test#119): every WHATWG Encoding
//! Standard encoding but UTF-8 (which runtime.js decodes itself), through `encoding_rs`, the
//! standard's implementation. runtime.js calls `__ttTextEncoding(label)` for the encoding a label
//! names, `__ttTextDecoderId()` once per decoder and `__ttTextDecode(id, encoding, ignoreBOM,
//! fatal, stream, bytes)` per `decode()`.
//!
//! A `stream: true` call leaves its `encoding_rs::Decoder` (partial sequence, ISO-2022-JP mode,
//! BOM state) here under the decoder's id for the next call; a final call flushes and drops it,
//! and so does a fatal error, so the next `decode()` starts afresh, as Node's.

use encoding_rs::{DecoderResult, Encoding};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

thread_local! {
    static OPEN: RefCell<HashMap<u64, encoding_rs::Decoder>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

/// The encoding `label` names, lower-cased as Node's `TextDecoder#encoding` reports it
/// (`"shift_jis"`, `"utf-16le"`, `"replacement"` for the replacement labels), or None for a
/// label the Encoding Standard does not list. Surrounding ASCII whitespace and ASCII case are
/// ignored.
pub fn encoding_name(label: &str) -> Option<String> {
    Encoding::for_label(label.as_bytes()).map(|e| e.name().to_ascii_lowercase())
}

/// A fresh decoder id (never reused on this thread).
pub fn new_id() -> u64 {
    NEXT_ID.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

/// Decode `bytes` as `encoding` (a name from [`encoding_name`]), continuing the stream of
/// decoder `id` if one is open. A leading BOM of the encoding is stripped unless `ignore_bom`.
/// Malformed input becomes U+FFFD, or `None` with `fatal`. With `stream` the decoder stays
/// open for the next call (a partial sequence waits for its rest); otherwise it is flushed and
/// closed.
pub fn decode(
    id: u64,
    encoding: &str,
    ignore_bom: bool,
    fatal: bool,
    stream: bool,
    bytes: &[u8],
) -> Option<String> {
    let enc = Encoding::for_label(encoding.as_bytes())?;
    let mut decoder = OPEN
        .with(|open| open.borrow_mut().remove(&id))
        .unwrap_or_else(|| {
            if ignore_bom {
                enc.new_decoder_without_bom_handling()
            } else {
                enc.new_decoder_with_bom_removal()
            }
        });
    let last = !stream;
    let mut out = String::new();
    if fatal {
        out.reserve(decoder.max_utf8_buffer_length_without_replacement(bytes.len())?);
        let (result, _) = decoder.decode_to_string_without_replacement(bytes, &mut out, last);
        if !matches!(result, DecoderResult::InputEmpty) {
            return None; // malformed (the decoder is dropped)
        }
    } else {
        out.reserve(decoder.max_utf8_buffer_length(bytes.len())?);
        // (room was reserved for the worst case, so all the input is consumed)
        let (result, _, _) = decoder.decode_to_string(bytes, &mut out, last);
        debug_assert!(matches!(result, encoding_rs::CoderResult::InputEmpty));
    }
    if stream {
        OPEN.with(|open| open.borrow_mut().insert(id, decoder));
    }
    Some(out)
}

/// `__ttTextEncoding(label)`: the encoding name, or undefined.
pub fn native_text_encoding(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let label = args.get(0).to_rust_string_lossy(scope);
    if let Some(name) = encoding_name(&label) {
        rv.set(v8::String::new(scope, &name).unwrap().into());
    }
}

/// `__ttTextDecoderId()`: a fresh decoder id.
pub fn native_text_decoder_id(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    rv.set(v8::Number::new(scope, new_id() as f64).into());
}

/// `__ttTextDecode(id, encoding, ignoreBOM, fatal, stream, view)`: the decoded string, or null
/// for malformed input with `fatal`.
pub fn native_text_decode(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let id = args.get(0).number_value(scope).unwrap_or(0.0) as u64;
    let encoding = args.get(1).to_rust_string_lossy(scope);
    let ignore_bom = args.get(2).boolean_value(scope);
    let fatal = args.get(3).boolean_value(scope);
    let stream = args.get(4).boolean_value(scope);
    let bytes = match v8::Local::<v8::ArrayBufferView>::try_from(args.get(5)) {
        Ok(view) => {
            let mut buf = vec![0; view.byte_length()];
            view.copy_contents(&mut buf);
            buf
        }
        Err(_) => Vec::new(),
    };
    let Some(text) = decode(id, &encoding, ignore_bom, fatal, stream, &bytes) else {
        return rv.set_null();
    };
    match v8::String::new(scope, &text) {
        Some(s) => rv.set(s.into()),
        None => {
            // longer than V8's string limit: Node's ERR_STRING_TOO_LONG message
            let msg = v8::String::new(
                scope,
                "Cannot create a string longer than 0x1fffffe8 characters",
            )
            .unwrap();
            let e = v8::Exception::error(scope, msg);
            scope.throw_exception(e);
        }
    }
}

#[cfg(test)]
#[path = "text_decoder_tests.rs"]
mod tests;
