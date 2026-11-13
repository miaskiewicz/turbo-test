//! Native SHA-256 for the owned Node and Web Crypto compatibility APIs.
use sha2::{Digest, Sha256};

/// `__ttSha256(view)`: hash precisely the bytes of the supplied view, returning a fresh buffer.
/// Input normalization and algorithm validation belong to the public JS APIs.
pub fn native_sha256<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(args.get(0)) else {
        let message = v8::String::new(scope, "SHA-256 input must be an ArrayBufferView").unwrap();
        let error = v8::Exception::type_error(scope, message);
        scope.throw_exception(error);
        return;
    };
    let mut bytes = vec![0; view.byte_length()];
    view.copy_contents(&mut bytes);
    let digest = Sha256::digest(&bytes).to_vec().into_boxed_slice();
    let store = v8::ArrayBuffer::new_backing_store_from_boxed_slice(digest).make_shared();
    rv.set(v8::ArrayBuffer::with_backing_store(scope, &store).into());
}
