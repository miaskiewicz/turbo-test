//! Owned Node compatibility callbacks: IP classification, gzip bytes and V8 serialization.
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr};
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

fn valid_zone(zone: &str) -> bool {
    !zone.is_empty()
        && zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:".contains(&b))
}

pub fn ip_version(input: &str) -> i32 {
    if input.parse::<Ipv4Addr>().is_ok() {
        return 4;
    }
    let address = match input.split_once('%') {
        Some((address, zone)) if valid_zone(zone) => address,
        Some(_) => return 0,
        None => input,
    };
    if address.parse::<Ipv6Addr>().is_ok() {
        6
    } else {
        0
    }
}

pub fn native_is_ip(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(input) = args.get(0).to_string(scope) else {
        return;
    };
    rv.set_int32(ip_version(&input.to_rust_string_lossy(scope)));
}

fn bytes(value: v8::Local<v8::Value>) -> Option<Vec<u8>> {
    let view = v8::Local::<v8::ArrayBufferView>::try_from(value).ok()?;
    let mut data = vec![0; view.byte_length()];
    view.copy_contents(&mut data);
    Some(data)
}

fn throw_error(scope: &mut v8::PinScope, message: &str) {
    let text = v8::String::new(scope, message).unwrap();
    let error = v8::Exception::error(scope, text);
    scope.throw_exception(error);
}

fn return_bytes(scope: &mut v8::PinScope, rv: &mut v8::ReturnValue, data: Vec<u8>) {
    let len = data.len();
    let store = v8::ArrayBuffer::new_backing_store_from_vec(data).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    rv.set(v8::Uint8Array::new(scope, buffer, 0, len).unwrap().into());
}

pub fn gzip(data: &[u8], level: u32) -> std::io::Result<Vec<u8>> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(level));
    writer.write_all(data)?;
    writer.finish()
}

pub fn gunzip(mut data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    loop {
        // Decode one member without buffering past its footer. Node stops at zero
        // padding after a complete member, but empty/truncated initial input is an error.
        let mut reader = flate2::bufread::GzDecoder::new(data);
        reader.read_to_end(&mut output)?;
        data = reader.into_inner();
        if data.is_empty() || data[0] == 0 {
            return Ok(output);
        }
    }
}

/// `__ttGzip(bytes, decompress, level)`: JS validates options and supplies a byte view.
pub fn native_gzip(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(data) = bytes(args.get(0)) else {
        return throw_error(scope, "Expected a byte view");
    };
    let result = if args.get(1).boolean_value(scope) {
        gunzip(&data)
    } else {
        gzip(&data, args.get(2).uint32_value(scope).unwrap_or(6))
    };
    match result {
        Ok(data) => return_bytes(scope, &mut rv, data),
        Err(error) => throw_error(scope, &error.to_string()),
    }
}

struct Serializer;
impl v8::ValueSerializerImpl for Serializer {
    fn throw_data_clone_error<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        message: v8::Local<'s, v8::String>,
    ) {
        let error = v8::Exception::error(scope, message);
        scope.throw_exception(error);
    }
}
struct Deserializer;
impl v8::ValueDeserializerImpl for Deserializer {}

pub fn native_serialize(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let context = scope.get_current_context();
    let serializer = v8::ValueSerializer::new(scope, Box::new(Serializer));
    serializer.write_header();
    if serializer.write_value(context, args.get(0)) == Some(true) {
        return_bytes(scope, &mut rv, serializer.release());
    }
}

pub fn native_deserialize(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(data) = bytes(args.get(0)) else {
        return throw_error(scope, "Expected a byte view");
    };
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    let context = tc.get_current_context();
    let deserializer = v8::ValueDeserializer::new(tc, Box::new(Deserializer), &data);
    let value = match deserializer.read_header(context) {
        Some(true) => deserializer.read_value(context),
        _ => None,
    };
    if let Some(value) = value {
        return rv.set(value);
    }
    if tc.has_caught() {
        tc.rethrow();
    } else {
        throw_error(tc, "Unable to deserialize cloned data");
        tc.rethrow();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_literals_match_node_edge_cases() {
        for ip in ["127.0.0.1", "0.0.0.0", "255.255.255.255"] {
            assert_eq!(ip_version(ip), 4, "{ip}");
        }
        for ip in [
            "::",
            "::1",
            "::ffff:192.168.1.1",
            "fe80::1%lo0",
            "::1%a-.:0",
        ] {
            assert_eq!(ip_version(ip), 6, "{ip}");
        }
        for ip in [
            "",
            "localhost",
            "127.1",
            "01.2.3.4",
            "256.0.0.1",
            "::1%",
            "::1%a b",
            "::1%a%b",
            "127.0.0.1%a",
            "[::1]",
            "::ffff:192.168.01.1",
        ] {
            assert_eq!(ip_version(ip), 0, "{ip}");
        }
    }

    #[test]
    fn gzip_is_real_and_rejects_corrupt_data() {
        let compressed = gzip(b"hello hello hello", 6).unwrap();
        assert_eq!(&compressed[..3], &[31, 139, 8]);
        assert_eq!(gunzip(&compressed).unwrap(), b"hello hello hello");
        assert!(gunzip(b"plain bytes").is_err());
        assert!(gunzip(&compressed[..compressed.len() - 1]).is_err());
        let mut joined = gzip(b"one", 0).unwrap();
        joined.extend(gzip(b"two", 9).unwrap());
        assert_eq!(gunzip(&joined).unwrap(), b"onetwo");
        let mut padded = compressed.clone();
        padded.extend([0, 1]);
        assert_eq!(gunzip(&padded).unwrap(), b"hello hello hello");
        let mut invalid = compressed.clone();
        invalid.push(1);
        assert!(gunzip(&invalid).is_err());
        assert!(gunzip(b"").is_err());
        assert_eq!(gunzip(&gzip(b"", 6).unwrap()).unwrap(), b"");
    }
}
