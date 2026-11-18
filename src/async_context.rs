//! V8 continuation frames: the engine carries this value across native Promise/await jobs.
//! No Promise prototype patch or strong Rust handle registry is involved.

pub fn native_get_frame(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    rv.set(scope.get_continuation_preserved_embedder_data());
}

pub fn native_set_frame(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    scope.set_continuation_preserved_embedder_data(args.get(0));
}
