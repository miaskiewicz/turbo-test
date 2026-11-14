//! X.509 parsing over real DER; signature and trust-chain verification are separate capabilities.
use x509_parser::prelude::{FromDer, X509Certificate, X509Name};
use x509_parser::time::ASN1Time;

struct Certificate {
    raw: Vec<u8>,
    spki: Vec<u8>,
    serial: String,
    valid_from: String,
    valid_to: String,
    subject: String,
    issuer: String,
}

fn name(value: &X509Name<'_>) -> Result<String, String> {
    value
        .iter_rdn()
        .map(|rdn| {
            X509Name::new(vec![rdn.clone()], &[])
                .to_string_with_registry(x509_parser::objects::oid_registry())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("\n"))
}

fn date(value: ASN1Time) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let value = value.to_datetime();
    format!(
        "{} {:>2} {:02}:{:02}:{:02} {} GMT",
        MONTHS[value.month() as usize - 1],
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.year()
    )
}

fn parse(bytes: &[u8]) -> Result<Certificate, String> {
    let (_, certificate) = X509Certificate::from_der(bytes)
        .map_err(|error| format!("Invalid X.509 certificate: {error}"))?;
    let serial = certificate.serial.to_str_radix(16).to_uppercase();
    Ok(Certificate {
        raw: certificate.as_raw().to_vec(),
        spki: certificate.public_key().raw.to_vec(),
        serial: if serial.len() % 2 == 0 {
            serial
        } else {
            format!("0{serial}")
        },
        valid_from: date(certificate.validity().not_before),
        valid_to: date(certificate.validity().not_after),
        subject: name(certificate.subject())?,
        issuer: name(certificate.issuer())?,
    })
}

fn buffer<'s>(scope: &mut v8::PinScope<'s, '_>, bytes: Vec<u8>) -> v8::Local<'s, v8::Uint8Array> {
    let length = bytes.len();
    let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length).unwrap()
}

fn string_field(scope: &mut v8::PinScope, object: v8::Local<v8::Object>, key: &str, value: &str) {
    let key = v8::String::new(scope, key).unwrap();
    let value = v8::String::new(scope, value).unwrap();
    object.set(scope, key.into(), value.into());
}

fn bytes_field(scope: &mut v8::PinScope, object: v8::Local<v8::Object>, key: &str, bytes: Vec<u8>) {
    let key = v8::String::new(scope, key).unwrap();
    let value = buffer(scope, bytes);
    object.set(scope, key.into(), value.into());
}

fn output(scope: &mut v8::PinScope, certificate: Certificate, rv: &mut v8::ReturnValue) {
    let object = v8::Object::new(scope);
    for (key, value) in [
        ("serialNumber", certificate.serial),
        ("validFrom", certificate.valid_from),
        ("validTo", certificate.valid_to),
        ("subject", certificate.subject),
        ("issuer", certificate.issuer),
    ] {
        string_field(scope, object, key, &value);
    }
    bytes_field(scope, object, "raw", certificate.raw);
    bytes_field(scope, object, "spki", certificate.spki);
    rv.set(object.into());
}

fn throw(scope: &mut v8::PinScope, message: &str) {
    let message = v8::String::new(scope, message).unwrap();
    let error = v8::Exception::error(scope, message);
    let object = v8::Local::<v8::Object>::try_from(error).unwrap();
    string_field(scope, object, "code", "ERR_OSSL_X509_PARSING_ERROR");
    scope.throw_exception(error);
}

/// Parse exactly the supplied byte view, returning owned certificate facts and SPKI bytes.
pub fn native_x509(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(args.get(0)) else {
        throw(scope, "X.509 input must be a byte view");
        return;
    };
    let mut bytes = vec![0; view.byte_length()];
    view.copy_contents(&mut bytes);
    match parse(&bytes) {
        Ok(certificate) => output(scope, certificate, &mut rv),
        Err(message) => throw(scope, &message),
    }
}
