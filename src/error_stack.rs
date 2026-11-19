//! `error.stack` formatted when it is first read, as Node formats it (turbo-test#424).
//!
//! V8 captures an error's call sites when the error is constructed and formats the `stack`
//! string lazily, on the first read. Node always registers a PrepareStackTraceCallback (its
//! `prepareStackTraceCallback` in lib/internal/errors.js), so the header is the error's
//! `name: message` at that first read: Nest's HttpException assigns `this.message` after
//! `super()` and its stack still starts with `NotFoundException: <message>`. Without a callback
//! this V8 builds the header from the error as it was at construction, so the runner registers
//! the same callback: a function `Error.prepareStackTrace` of the error's realm wins (V8's stack
//! trace API, called with the Error constructor as receiver); otherwise the header is the
//! ErrorUtils::ToString of the error then, followed by each call site as `\n    at <site>`.
//! The call sites are V8's own, so line and column numbers are unchanged.

/// Register the callback on a runner isolate.
pub(crate) fn install(isolate: &mut v8::Isolate) {
    isolate.set_prepare_stack_trace_callback(prepare_stack_trace);
}

fn prepare_stack_trace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: v8::Local<'s, v8::Value>,
    sites: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Value> {
    // None: a getter or a user prepareStackTrace threw; V8 rethrows that pending exception
    format(scope, error, sites).unwrap_or_else(|| v8::undefined(scope).into())
}

fn format<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: v8::Local<'s, v8::Value>,
    sites: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "Error")?;
    let constructor = global.get(scope, key.into())?;
    if let Some(user) = user_prepare(scope, constructor) {
        return user.call(scope, constructor, &[error, sites.into()]);
    }
    let text = header(scope, error)? + &frames(scope, sites)?;
    Some(v8::String::new(scope, &text)?.into())
}

/// `\n    at <site>` for each call site (CallSite.prototype.toString, V8's own serialization)
fn frames(scope: &mut v8::PinScope, sites: v8::Local<v8::Array>) -> Option<String> {
    let mut text = String::new();
    for index in 0..sites.length() {
        let site = sites.get_index(scope, index)?;
        text.push_str("\n    at ");
        text.push_str(&site.to_string(scope)?.to_rust_string_lossy(scope));
    }
    Some(text)
}

/// `Error.prepareStackTrace` when it is a function
fn user_prepare<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Function>> {
    let constructor = v8::Local::<v8::Object>::try_from(constructor).ok()?;
    let key = v8::String::new(scope, "prepareStackTrace")?;
    let value = constructor.get(scope, key.into())?;
    v8::Local::<v8::Function>::try_from(value).ok()
}

/// V8's ErrorUtils::ToString: `name` (default "Error") and `message` (default ""), joined by
/// ": " when both are non-empty.
fn header(scope: &mut v8::PinScope, error: v8::Local<v8::Value>) -> Option<String> {
    let name = property(scope, error, "name")?.unwrap_or_else(|| "Error".to_owned());
    let message = property(scope, error, "message")?.unwrap_or_default();
    Some(match (name.is_empty(), message.is_empty()) {
        (true, _) => message,
        (false, true) => name,
        (false, false) => format!("{name}: {message}"),
    })
}

/// The string value of `error[key]`: Some(None) when it is undefined, None when reading or
/// converting it threw.
fn property(
    scope: &mut v8::PinScope,
    error: v8::Local<v8::Value>,
    key: &str,
) -> Option<Option<String>> {
    let object = error.to_object(scope)?;
    let key = v8::String::new(scope, key)?;
    let value = object.get(scope, key.into())?;
    if value.is_undefined() {
        return Some(None);
    }
    Some(Some(value.to_string(scope)?.to_rust_string_lossy(scope)))
}
