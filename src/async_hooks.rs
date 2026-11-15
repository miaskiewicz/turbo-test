//! Context-owned V8 Promise hooks. Disabled contexts have no native Promise callback installed.

fn callback<'s>(value: v8::Local<'s, v8::Value>) -> Option<v8::Local<'s, v8::Function>> {
    v8::Local::<v8::Function>::try_from(value).ok()
}

/// Install or clear the engine's init/before/after/resolve hooks, without patching Promise.then.
pub fn native_set_promise_hooks(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    scope.set_promise_hooks(
        callback(args.get(0)),
        callback(args.get(1)),
        callback(args.get(2)),
        callback(args.get(3)),
    );
}
