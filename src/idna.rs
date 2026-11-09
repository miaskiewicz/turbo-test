//! UTS#46 IDN ToASCII for the runtime's `URL` host parse (turbo-test#259).
//!
//! `new URL('http://ü.com').host` must come back as `xn--tda.com`, like Node's WHATWG URL
//! — runtime.js only lowercases ASCII and would otherwise store the raw `ü.com`. runtime.js
//! calls `__ttIdnaToAscii(host)` from the host parse for a special non-file, non-IPv6 host;
//! the `idna` crate's default `domain_to_ascii` is UTS#46 non-transitional ToASCII (the
//! WHATWG URL spec's domain-to-ASCII algorithm).
//!
//! Returns `undefined` for an input the algorithm rejects, so the runtime can keep the
//! original value (ASCII-only labels, trailing dots and already-`xn--`-encoded labels all
//! round-trip through the crate cleanly; the case to surface is a label that fails
//! ToASCII, not a plain non-ASCII host).

/// `__ttIdnaToAscii(host)`: the UTS#46 ToASCII serialization of `host`, or undefined on
/// failure (returned as `undefined` so runtime.js falls back to the original value).
pub fn native_idna_to_ascii(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let host = args.get(0).to_rust_string_lossy(scope);
    if let Ok(ascii) = idna::domain_to_ascii(&host) {
        if let Some(s) = v8::String::new(scope, &ascii) {
            rv.set(s.into());
        }
    }
}
