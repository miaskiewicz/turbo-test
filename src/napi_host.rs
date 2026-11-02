//! Node-API (napi) host — load native `.node` addons in turbo-test's V8 embedding.
//!
//! Implements the Node-API C ABI, backed by rusty_v8: every function of Node 26's
//! `js_native_api.h` and `node_api.h` up to NAPI_VERSION 10 (the inventory is
//! docs/node-api.md). The asynchronous part (async work, threadsafe functions, async contexts,
//! callback scopes) is `napi_host_async.rs`. An addon is opened with every import bound at load
//! (RTLD_NOW), so one importing a function the host does not provide fails its require() with
//! an error naming it instead of crashing on the first call.
//!
//! Bridge: `napi_value` IS a `v8::Local<Value>` (Local is `NonNull<T>` — pointer-sized),
//! so they transmute. Handles are created in the scope stored on the env for the current
//! native call (set by the function trampoline / loader before calling into the addon, and
//! restored when a nested call returns).
//!
//! Semantics follow Node's js_native_api_v8.cc (the status codes, the checks and coercions):
//! every loaded addon gets its own env, owned by its isolate; callbacks may re-enter (native ->
//! JS -> native); calls that can run JS honour a pending exception (NAPI_PREAMBLE) and park
//! what JS throws on the env (`js_call`), from where napi_get_and_clear_last_exception can take
//! it or the trampoline throws it to the JS caller. No entrypoint panics across the FFI boundary.

#![allow(
    non_camel_case_types,
    non_upper_case_globals,
    dead_code,
    clippy::missing_safety_doc
)]

use std::ffi::c_void;
use std::os::raw::c_char;
use std::path::Path;

pub type napi_status = i32;
pub const napi_ok: napi_status = 0;
pub const napi_invalid_arg: napi_status = 1;
pub const napi_object_expected: napi_status = 2;
pub const napi_string_expected: napi_status = 3;
pub const napi_name_expected: napi_status = 4;
pub const napi_number_expected: napi_status = 6;
pub const napi_boolean_expected: napi_status = 7;
pub const napi_array_expected: napi_status = 8;
pub const napi_function_expected: napi_status = 5;
pub const napi_generic_failure: napi_status = 9;
pub const napi_pending_exception: napi_status = 10;
pub const napi_cancelled: napi_status = 11;
pub const napi_escape_called_twice: napi_status = 12;
pub const napi_handle_scope_mismatch: napi_status = 13;
pub const napi_callback_scope_mismatch: napi_status = 14;
pub const napi_queue_full: napi_status = 15;
pub const napi_closing: napi_status = 16;
pub const napi_bigint_expected: napi_status = 17;
pub const napi_date_expected: napi_status = 18;
pub const napi_arraybuffer_expected: napi_status = 19;
pub const napi_detachable_arraybuffer_expected: napi_status = 20;
pub const napi_would_deadlock: napi_status = 21;
pub const napi_no_external_buffers_allowed: napi_status = 22;
pub const napi_cannot_run_js: napi_status = 23;

/// The message napi_get_last_error_info reports for each status (Node's `error_messages`).
const ERROR_MESSAGES: [&std::ffi::CStr; 24] = [
    c"",
    c"Invalid argument",
    c"An object was expected",
    c"A string was expected",
    c"A string or symbol was expected",
    c"A function was expected",
    c"A number was expected",
    c"A boolean was expected",
    c"An array was expected",
    c"Unknown failure",
    c"An exception is pending",
    c"The async work item was cancelled",
    c"napi_escape_handle already called on scope",
    c"Invalid handle scope usage",
    c"Invalid callback scope usage",
    c"Thread-safe function queue is full",
    c"Thread-safe function handle is closing",
    c"A bigint was expected",
    c"A date was expected",
    c"An arraybuffer was expected",
    c"A detachable arraybuffer was expected",
    c"Main thread would deadlock",
    c"External buffers are not allowed",
    c"Cannot run JavaScript",
];

/// napi_extended_error_info: the last call's status and its message.
#[repr(C)]
pub struct napi_extended_error_info {
    pub error_message: *const c_char,
    pub engine_reserved: *mut c_void,
    pub engine_error_code: u32,
    pub error_code: napi_status,
}

/// The module API version of an addon that declares none (Node's
/// NODE_API_DEFAULT_MODULE_API_VERSION).
pub const DEFAULT_MODULE_API_VERSION: i32 = 8;

pub type napi_value = *mut c_void;
pub type napi_env = *mut Env;
pub type napi_ref = *mut c_void;
pub type napi_callback_info = *mut CbInfo;
pub type napi_callback = unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value;
pub type napi_finalize = unsafe extern "C" fn(napi_env, *mut c_void, *mut c_void);

/// napi_property_attributes bits.
pub const napi_writable: i32 = 1;
pub const napi_enumerable: i32 = 1 << 1;
pub const napi_configurable: i32 = 1 << 2;
pub const napi_static: i32 = 1 << 10;

/// One property for napi_define_class / napi_define_properties: named by `utf8name`, else by
/// the `name` value; a `method`, a `getter` / `setter` pair, or a plain `value`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct napi_property_descriptor {
    pub utf8name: *const c_char,
    pub name: napi_value,
    pub method: Option<napi_callback>,
    pub getter: Option<napi_callback>,
    pub setter: Option<napi_callback>,
    pub value: napi_value,
    pub attributes: i32,
    pub data: *mut c_void,
}

#[repr(i32)]
pub enum napi_valuetype {
    Undefined = 0,
    Null = 1,
    Boolean = 2,
    Number = 3,
    String = 4,
    Symbol = 5,
    Object = 6,
    Function = 7,
    External = 8,
    Bigint = 9,
}

/// One loaded addon's napi environment (Node creates one `napi_env` per addon instance), owned
/// by its isolate (`AddonEnvs`). `scope` points at the live rusty_v8 scope for the current
/// native call (the only window during which napi value calls happen).
pub struct Env {
    pub isolate: v8::UnsafeRawIsolatePtr,
    pub scope: *mut c_void, // *mut v8::PinScope<'static,'static>, valid only during a call
    pub context: v8::Global<v8::Context>,
    pub last_exception: Option<v8::Global<v8::Value>>,
    /// One record per napi_create_function; each function's data is an External pointing at
    /// its record, so a function always dispatches into the env (addon) that created it.
    pub fns: Vec<*mut Callback>,
    pub refs: Vec<Option<Ref>>,
    /// The live finalizer records of this env (napi_wrap, napi_add_finalizer), by creation order.
    pub finalizers: std::collections::BTreeMap<u64, *mut Finalizer>,
    pub next_finalizer: u64,
    /// The env is being torn down: napi value calls work, JS cannot run (Node's
    /// !can_call_into_js()).
    pub tearing_down: bool,
    /// The status of the last napi call, what napi_get_last_error_info reports.
    pub last_error: napi_extended_error_info,
    /// The Node-API version the addon declares (node_api_module_get_api_version_v1), else 8.
    pub module_api_version: i32,
    /// What the addons of this env's isolate share (the teardown work).
    shared: std::rc::Rc<Shared>,
    /// Handle scopes opened and not closed (napi_open_handle_scope / _escapable_).
    open_handle_scopes: i32,
    /// Callback scopes opened and not closed (napi_open_callback_scope).
    open_callback_scopes: i32,
    /// The finalizer record of napi_set_instance_data's data.
    instance_data: Option<u64>,
    /// The addon's file as a file: URL (node_api_get_module_file_name).
    filename: std::ffi::CString,
}

impl Env {
    /// A new env for an addon loaded in `scope`'s isolate and context.
    pub fn new(scope: &mut v8::PinScope) -> Env {
        let shared = match scope.get_slot::<AddonEnvs>() {
            Some(envs) => envs.0.clone(),
            None => {
                let shared = std::rc::Rc::<Shared>::default();
                scope.set_slot(AddonEnvs(shared.clone()));
                shared
            }
        };
        Env {
            isolate: unsafe { scope.as_raw_isolate_ptr() },
            scope: scope as *mut v8::PinScope as *mut c_void,
            context: v8::Global::new(scope, scope.get_current_context()),
            last_exception: None,
            fns: Vec::new(),
            refs: Vec::new(),
            finalizers: Default::default(),
            next_finalizer: 0,
            tearing_down: false,
            last_error: napi_extended_error_info {
                error_message: std::ptr::null(),
                engine_reserved: std::ptr::null_mut(),
                engine_error_code: 0,
                error_code: napi_ok,
            },
            module_api_version: DEFAULT_MODULE_API_VERSION,
            shared,
            open_handle_scopes: 0,
            open_callback_scopes: 0,
            instance_data: None,
            filename: std::ffi::CString::default(),
        }
    }
}

impl Drop for Env {
    /// Env teardown (the isolate is being disposed). As Node's napi_env__::DeleteMe, the
    /// finalizers still pending (napi_wrap, napi_add_finalizer) run now, newest first.
    fn drop(&mut self) {
        let env: napi_env = self;
        unsafe {
            if !(*env).finalizers.is_empty() {
                teardown_finalizers(env);
            }
        }
        forget_external_strings(env);
        for &f in &self.fns {
            drop(unsafe { Box::from_raw(f) });
        }
    }
}

/// Run the env's remaining finalizers, newest first, the way Node runs them at teardown: in a
/// handle scope of the env's context, so value calls (references, creating values) work, but
/// with JS disabled, so a call that could run JS is napi_pending_exception (`js_call`). The
/// isolate is mid-disposal (OwnedIsolate::drop has exited it before dropping its slots, the env
/// included) but not yet disposed: it is entered again for the scope.
unsafe fn teardown_finalizers(env: napi_env) {
    in_teardown_scope(env, || {
        while let Some((_, fin)) = (*env).finalizers.pop_last() {
            run_finalizer(env, &Box::from_raw(fin));
        }
    });
}

/// Run `f` at env teardown in a handle scope of `env`'s context, with JS disabled for the env
/// (Node's !can_call_into_js()). The isolate is mid-disposal (OwnedIsolate::drop has exited it
/// before dropping its slots, the envs included) but not yet disposed: it is entered again.
unsafe fn in_teardown_scope(env: napi_env, f: impl FnOnce()) {
    let isolate = &mut v8::Isolate::from_raw_isolate_ptr((*env).isolate);
    isolate.enter();
    {
        v8::scope!(let hs, isolate);
        let context = v8::Local::new(hs, &(*env).context);
        let cs: &mut v8::PinScope = &mut v8::ContextScope::new(hs, context);
        (*env).scope = cs as *mut v8::PinScope as *mut c_void;
        (*env).tearing_down = true;
        f();
        (*env).scope = std::ptr::null_mut();
    }
    isolate.exit();
}

/// A napi_ref, as Node's v8impl::Reference: a value with a reference count. Above 0 the value
/// is held strongly; at 0 weakly (it goes when collected), which only objects and symbols can be
/// (any other value is dropped). The references napi_wrap and napi_add_finalizer hand back start
/// at 0, so they do not keep the object alive.
pub struct Ref {
    count: u32,
    value: RefValue,
}

pub enum RefValue {
    Strong(v8::Global<v8::Value>),
    Weak(v8::Weak<v8::Value>),
    /// No value any more: collected, dropped at count 0, or the reference of a finalizer record
    /// whose finalizer has run (or is running), as Node's Reference::Finalize resets it first.
    Empty,
}

impl Ref {
    /// A reference to `value` with `count`, weak at 0 (Node's Reference::New).
    fn new(s: &mut v8::PinScope<'_, '_>, value: v8::Local<v8::Value>, count: u32) -> Ref {
        let value = if count > 0 {
            RefValue::Strong(v8::Global::new(s, value))
        } else {
            weak_value(s, value)
        };
        Ref { count, value }
    }

    /// The value, None when it is gone.
    fn get<'s>(&self, s: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        match &self.value {
            RefValue::Strong(g) => Some(v8::Local::new(s, g)),
            RefValue::Weak(w) => w.to_local(s),
            RefValue::Empty => None,
        }
    }

    /// Reference::Ref: the new count, 0 (unchanged) once the value is gone; at 1 the value
    /// becomes strong.
    fn inc(&mut self, s: &mut v8::PinScope<'_, '_>) -> u32 {
        let Some(value) = self.get(s) else {
            self.value = RefValue::Empty;
            return 0;
        };
        self.count += 1;
        if self.count == 1 {
            self.value = RefValue::Strong(v8::Global::new(s, value));
        }
        self.count
    }

    /// Reference::Unref of a count above 0: the new count; at 0 the value becomes weak.
    fn dec(&mut self, s: &mut v8::PinScope<'_, '_>) -> u32 {
        let Some(value) = self.get(s) else {
            self.value = RefValue::Empty;
            return 0;
        };
        self.count -= 1;
        if self.count == 0 {
            self.value = weak_value(s, value);
        }
        self.count
    }
}

/// `value` held weakly: objects (functions included) and symbols, what V8 can hold weakly (Node's
/// CanBeHeldWeakly); any other value is dropped (Reference::SetWeak resets it).
fn weak_value(s: &mut v8::PinScope<'_, '_>, value: v8::Local<v8::Value>) -> RefValue {
    if value.is_object() || value.is_symbol() {
        RefValue::Weak(v8::Weak::new(s, value))
    } else {
        RefValue::Empty
    }
}

/// A native pointer tied to a JS object's lifetime, with its optional finalizer, which runs
/// once, when the object is collected or at env teardown. napi_wrap stores one on the object (an
/// External under the isolate's wrap private key points here; napi_remove_wrap drops it without
/// finalizing); napi_add_finalizer attaches any number to an object.
pub struct Finalizer {
    env: napi_env,
    id: u64,
    native: *mut c_void,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
    /// Weak handle on the object; its GC finalizer (`finalizer_collected`) frees this record.
    weak: Option<v8::Weak<v8::Value>>,
    /// The id of the result reference napi_wrap / napi_add_finalizer handed back (0: none).
    own_ref: usize,
}

/// Run `fin`'s finalizer (if any) in the env's current scope: addon code, so under the lock.
/// The env's pending exception is set aside for it.
unsafe fn run_finalizer(env: napi_env, fin: &Finalizer) {
    if let Some(cb) = fin.finalize {
        // as Node's Reference::Finalize: the record's own reference loses its value first (a
        // deleted one stays deleted)
        let refs = &mut (*env).refs;
        if let Some(Some(r)) = refs.get_mut(fin.own_ref.wrapping_sub(1)) {
            r.value = RefValue::Empty;
        }
        let pending = (*env).last_exception.take();
        let _ = call_addon(|| cb(env, fin.native, fin.hint));
        // as Node's CallFinalizer: what the finalizer leaves pending is an uncaught exception,
        // reported from the event loop (a finalizer may run inside a GC, where JS cannot); at
        // teardown, when JS cannot run, it is dropped
        if let Some(exc) = std::mem::replace(&mut (*env).last_exception, pending) {
            if !(*env).tearing_down {
                async_host::defer_uncaught(env, exc);
            }
        }
    }
}

/// What a napi-created function calls: the addon callback, its data, and the env it belongs to.
#[derive(Clone, Copy)]
pub struct Callback {
    env: napi_env,
    cb: napi_callback,
    data: *mut c_void,
}

/// What the addons of one isolate share, kept in an isolate slot (`AddonEnvs`) so it goes with
/// the isolate (the functions pointing into the envs cannot outlive it either): the teardown
/// work of their envs, in registration order.
#[derive(Default)]
pub struct Shared {
    teardown: std::cell::RefCell<Vec<Teardown>>,
    /// The native callbacks posted for the isolate's JS thread (async work, threadsafe
    /// functions).
    native: std::sync::Arc<async_host::NativeLoop>,
    /// The open threadsafe functions.
    tsfns: std::cell::RefCell<Vec<*mut async_host::Tsfn>>,
}

/// One piece of an isolate's teardown work, run newest first (Node runs its environment
/// cleanup hooks in reverse order of registration).
enum Teardown {
    /// An addon's env (registered when the addon loads): its finalizers run and it is freed.
    Env(napi_env),
    /// napi_add_env_cleanup_hook: `fun(arg)`.
    Hook(napi_env, unsafe extern "C" fn(*mut c_void), *mut c_void),
    /// napi_add_async_cleanup_hook: the hook is called with its handle, which the addon then
    /// removes (napi_remove_async_cleanup_hook frees it).
    AsyncHook(*mut AsyncCleanupHook),
    /// An open threadsafe function: closed and finalized.
    Tsfn(*mut async_host::Tsfn),
}

/// The isolate slot owning `Shared`; dropping it (isolate disposal) runs the teardown work.
struct AddonEnvs(std::rc::Rc<Shared>);

impl Drop for AddonEnvs {
    /// As Node's environment cleanup: the cleanup hooks run newest first (an env's own hook,
    /// registered when its addon loaded, tears it down: its finalizers run and it is freed);
    /// a threadsafe function's hook closes it, and once the hooks have run its finalization
    /// runs (libuv's close callbacks). An env with threadsafe functions open is torn down when
    /// the last of them is finalized, as Node's env reference count. Repeated while hooks
    /// register more.
    fn drop(&mut self) {
        let shared = &self.0;
        // no addon's env can run JS any more
        for item in shared.teardown.borrow().iter() {
            if let Teardown::Env(env) = item {
                unsafe { (**env).tearing_down = true };
            }
        }
        loop {
            let mut closing = Vec::new();
            let mut waiting = Vec::new();
            loop {
                let item = shared.teardown.borrow_mut().pop();
                let Some(item) = item else { break };
                unsafe {
                    match item {
                        Teardown::Env(env) if async_host::holds_env(shared, env) => {
                            waiting.push(env)
                        }
                        Teardown::Env(env) => drop(Box::from_raw(env)),
                        Teardown::Hook(env, fun, arg) => in_teardown_scope(env, || {
                            let _ = call_addon(|| fun(arg));
                        }),
                        Teardown::AsyncHook(h) => in_teardown_scope((*h).env, || {
                            let _ = call_addon(|| ((*h).hook)(h, (*h).arg));
                        }),
                        Teardown::Tsfn(t) => closing.push(async_host::close_for_teardown(t)),
                    }
                }
            }
            if closing.is_empty() {
                break;
            }
            for t in closing {
                unsafe {
                    let env = async_host::finalize_for_teardown(t);
                    if waiting.contains(&env) && !async_host::holds_env(shared, env) {
                        waiting.retain(|&e| e != env);
                        drop(Box::from_raw(env));
                    }
                }
            }
        }
    }
}

/// The teardown work list `env` registers into (its isolate's).
unsafe fn teardown_of<'a>(env: napi_env) -> &'a std::cell::RefCell<Vec<Teardown>> {
    let e = &*env;
    &e.shared.teardown
}

/// A napi_async_cleanup_hook_handle: the hook and its argument, until removed.
pub struct AsyncCleanupHook {
    env: napi_env,
    hook: unsafe extern "C" fn(*mut AsyncCleanupHook, *mut c_void),
    arg: *mut c_void,
}

/// Process-global lock serializing native `.node` addon entry across worker threads (the shared
/// addon's internal state isn't thread-safe). `IN_ADDON` allows same-thread re-entry.
static ADDON_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
thread_local! {
    static IN_ADDON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[inline]
unsafe fn scope<'a>(env: napi_env) -> &'a mut v8::PinScope<'static, 'static> {
    &mut *((*env).scope as *mut v8::PinScope<'static, 'static>)
}

#[inline]
unsafe fn to_napi(l: v8::Local<v8::Value>) -> napi_value {
    std::mem::transmute::<v8::Local<v8::Value>, *mut c_void>(l)
}

#[inline]
unsafe fn to_local(v: napi_value) -> v8::Local<'static, v8::Value> {
    std::mem::transmute::<*mut c_void, v8::Local<v8::Value>>(v)
}

#[inline]
unsafe fn put(out: *mut napi_value, l: v8::Local<v8::Value>) {
    if !out.is_null() {
        *out = to_napi(l);
    }
}

/// A napi call's outcome; `Err(status)` carries a non-`napi_ok` status, so `?` returns it.
type NapiResult = Result<(), napi_status>;

/// Record `status` as the env's last error (Node's napi_set_last_error / napi_clear_last_error:
/// what napi_get_last_error_info reports) and return it.
unsafe fn record(env: napi_env, status: napi_status) -> napi_status {
    let e = &mut (*env).last_error;
    e.error_code = status;
    e.engine_error_code = 0;
    e.engine_reserved = std::ptr::null_mut();
    status
}

/// Run a napi call that runs no JS (Node's calls without NAPI_PREAMBLE): a NULL env is
/// napi_invalid_arg (CHECK_ENV), and the status `f` returns is recorded as the last error.
unsafe fn checked(env: napi_env, f: impl FnOnce() -> napi_status) -> napi_status {
    if env.is_null() {
        return napi_invalid_arg;
    }
    record(env, f())
}

/// `checked` for a call written with `?`: an `Err` carries its status.
unsafe fn basic(env: napi_env, f: impl FnOnce() -> NapiResult) -> napi_status {
    checked(env, || f().err().unwrap_or(napi_ok))
}

/// A required out-pointer argument (Node's CHECK_ARG): NULL is napi_invalid_arg.
unsafe fn out_arg<'a, T>(p: *mut T) -> Result<&'a mut T, napi_status> {
    p.as_mut().ok_or(napi_invalid_arg)
}

/// A required napi_value argument (Node's CHECK_ARG): NULL is napi_invalid_arg.
unsafe fn val_arg(v: napi_value) -> Result<v8::Local<'static, v8::Value>, napi_status> {
    if v.is_null() {
        return Err(napi_invalid_arg);
    }
    Ok(to_local(v))
}

/// Run `f` the way Node runs a napi call that can execute JS (`NAPI_PREAMBLE` plus its
/// `v8impl::TryCatch`): while an exception is pending it returns `napi_pending_exception`
/// without running `f`, and when JS cannot run (env teardown) napi_pending_exception, or
/// napi_cannot_run_js for an addon declaring module API version 10 or later; otherwise it runs
/// `f` under `catching`.
unsafe fn js_call(
    env: napi_env,
    f: impl FnOnce(&mut v8::PinScope<'static, 'static>) -> NapiResult,
) -> napi_status {
    if env.is_null() {
        return napi_invalid_arg;
    }
    if (*env).last_exception.is_some() {
        return record(env, napi_pending_exception);
    }
    if (*env).tearing_down {
        let status = if (*env).module_api_version >= 10 {
            napi_cannot_run_js
        } else {
            napi_pending_exception
        };
        return record(env, status);
    }
    catching(env, f)
}

/// Run `f` in a TryCatch: a JS exception thrown inside it is caught and parked on the env as
/// its pending exception. `napi_is_exception_pending` / `napi_get_and_clear_last_exception`
/// see it, and the trampoline (or the module loader) throws it to JS when the addon returns
/// to its caller. A termination is not caught: it keeps unwinding. The status is recorded as
/// the last error; a NULL env is napi_invalid_arg.
unsafe fn catching(
    env: napi_env,
    f: impl FnOnce(&mut v8::PinScope<'static, 'static>) -> NapiResult,
) -> napi_status {
    if env.is_null() {
        return napi_invalid_arg;
    }
    let tc = std::pin::pin!(v8::TryCatch::new(scope(env)));
    let tc = &mut tc.init();
    let result = f(tc);
    // as Node's RETURN_STATUS_IF_FALSE_WITH_PREAMBLE: V8 giving nothing is a pending exception
    // when it threw, else the call's failure status
    let status = match result {
        Err(V8_GAVE_NOTHING) if tc.has_caught() => napi_pending_exception,
        Err(V8_GAVE_NOTHING) => napi_generic_failure,
        other => other.err().unwrap_or(napi_ok),
    };
    if !tc.has_terminated() {
        if let Some(exc) = tc.exception() {
            (*env).last_exception = Some(v8::Global::new(tc, exc));
        }
    }
    record(env, status)
}

/// What a `js_call` / `catching` closure returns when a V8 call gave nothing (an empty Maybe or
/// `false`): napi_pending_exception when it threw, napi_generic_failure otherwise.
const V8_GAVE_NOTHING: napi_status = -1;

/// A V8 `Maybe<bool>` that must be `Just(true)`: anything else is `V8_GAVE_NOTHING`.
fn v8_true(m: Option<bool>) -> NapiResult {
    m.filter(|&b| b).map(|_| ()).ok_or(V8_GAVE_NOTHING)
}

/// The object argument of a property call, as Node's CHECK_TO_OBJECT: NULL is
/// napi_invalid_arg, then ToObject (null / undefined: napi_object_expected).
unsafe fn obj_arg(
    s: &mut v8::PinScope<'static, 'static>,
    v: napi_value,
) -> Result<v8::Local<'static, v8::Object>, napi_status> {
    val_arg(v)?;
    to_object(s, v)
}

/// ToObject of an object argument, as Node's CHECK_TO_OBJECT: primitives become their wrapper
/// objects; null / undefined throw a TypeError and the call is napi_object_expected.
unsafe fn to_object(
    s: &mut v8::PinScope<'static, 'static>,
    obj: napi_value,
) -> Result<v8::Local<'static, v8::Object>, napi_status> {
    to_local(obj).to_object(s).ok_or(napi_object_expected)
}

/// Callback info passed to a napi_callback during a function call.
pub struct CbInfo {
    pub args: Vec<napi_value>,
    pub this: napi_value,
    pub data: *mut c_void,
    /// `new.target` of a construct call, NULL for a plain call (napi_get_new_target).
    pub new_target: napi_value,
}

// ---- value getters --------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_get_undefined(env: napi_env, out: *mut napi_value) -> napi_status {
    basic(env, || {
        let s = scope(env);
        *out_arg(out)? = to_napi(v8::undefined(s).into());
        Ok(())
    })
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_null(env: napi_env, out: *mut napi_value) -> napi_status {
    basic(env, || {
        let s = scope(env);
        *out_arg(out)? = to_napi(v8::null(s).into());
        Ok(())
    })
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_global(env: napi_env, out: *mut napi_value) -> napi_status {
    basic(env, || {
        let s = scope(env);
        *out_arg(out)? = to_napi(s.get_current_context().global(s).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_boolean(
    env: napi_env,
    b: bool,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        let s = scope(env);
        *out_arg(out)? = to_napi(v8::Boolean::new(s, b).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_typeof(
    env: napi_env,
    v: napi_value,
    out: *mut napi_valuetype,
) -> napi_status {
    basic(env, || {
        let val = val_arg(v)?;
        let out = out_arg(out)?;
        *out = if val.is_undefined() {
            napi_valuetype::Undefined
        } else if val.is_null() {
            napi_valuetype::Null
        } else if val.is_boolean() {
            napi_valuetype::Boolean
        } else if val.is_number() {
            napi_valuetype::Number
        } else if val.is_string() {
            napi_valuetype::String
        } else if val.is_symbol() {
            napi_valuetype::Symbol
        } else if val.is_big_int() {
            napi_valuetype::Bigint
        } else if val.is_function() {
            napi_valuetype::Function
        } else if val.is_external() {
            napi_valuetype::External
        } else {
            napi_valuetype::Object
        };
        Ok(())
    })
}

// ---- numbers / strings ----------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_uint32(
    env: napi_env,
    n: u32,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        let s = scope(env);
        *out_arg(out)? = to_napi(v8::Integer::new_from_unsigned(s, n).into());
        Ok(())
    })
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_double(
    env: napi_env,
    n: f64,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        let s = scope(env);
        *out_arg(out)? = to_napi(v8::Number::new(s, n).into());
        Ok(())
    })
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_uint32(
    env: napi_env,
    v: napi_value,
    out: *mut u32,
) -> napi_status {
    basic(env, || {
        val_arg(v)?;
        let out = out_arg(out)?;
        *out = to_uint32(number_of(v).ok_or(napi_number_expected)?);
        Ok(())
    })
}

/// The value of a JS number, `None` for anything else: Node's number getters never coerce
/// (they return napi_number_expected).
unsafe fn number_of(v: napi_value) -> Option<f64> {
    v8::Local::<v8::Number>::try_from(to_local(v))
        .ok()
        .map(|n| n.value())
}

/// ECMAScript ToUint32, what V8's Uint32Value (Node's napi_get_value_uint32) computes: NaN and
/// the infinities are 0, anything else is truncated and wrapped modulo 2^32.
fn to_uint32(n: f64) -> u32 {
    if n.is_finite() {
        n.trunc().rem_euclid(4294967296.0) as u32
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf8(
    env: napi_env,
    str_: *const c_char,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    // V8's String::NewFromUtf8 (what Node uses) replaces invalid sequences with U+FFFD.
    new_string(env, str_.is_null(), len, out, |s| {
        v8::String::new_from_utf8(s, utf8_arg(str_, len), v8::NewStringType::Normal)
    })
}

/// Node's v8impl::NewString: a NULL string with a non-zero length, a NULL result or a length
/// over INT_MAX (but NAPI_AUTO_LENGTH) are napi_invalid_arg; a string V8 cannot make (over its
/// maximum length) napi_generic_failure.
unsafe fn new_string(
    env: napi_env,
    str_is_null: bool,
    len: usize,
    out: *mut napi_value,
    make: impl FnOnce(&mut v8::PinScope<'static, 'static>) -> Option<v8::Local<'static, v8::String>>,
) -> napi_status {
    basic(env, || {
        if len > 0 && str_is_null {
            return Err(napi_invalid_arg);
        }
        let out = out_arg(out)?;
        if len != NAPI_AUTO_LENGTH && len > i32::MAX as usize {
            return Err(napi_invalid_arg);
        }
        *out = to_napi(make(scope(env)).ok_or(napi_generic_failure)?.into());
        Ok(())
    })
}

/// NAPI_AUTO_LENGTH: the string argument is NUL-terminated.
const NAPI_AUTO_LENGTH: usize = usize::MAX;

/// The bytes of a `(const char*, size_t)` Latin-1 argument.
unsafe fn latin1_arg<'a>(str_: *const c_char, len: usize) -> &'a [u8] {
    utf8_arg(str_, len)
}

/// The code units of a `(const char16_t*, size_t)` argument; NAPI_AUTO_LENGTH means
/// NUL-terminated.
unsafe fn utf16_arg<'a>(str_: *const u16, len: usize) -> &'a [u16] {
    if str_.is_null() {
        return &[];
    }
    let len = if len == NAPI_AUTO_LENGTH {
        (0..).take_while(|&i| *str_.add(i) != 0).count()
    } else {
        len
    };
    std::slice::from_raw_parts(str_, len)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_latin1(
    env: napi_env,
    str_: *const c_char,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    new_string(env, str_.is_null(), len, out, |s| {
        v8::String::new_from_one_byte(s, latin1_arg(str_, len), v8::NewStringType::Normal)
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf16(
    env: napi_env,
    str_: *const u16,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    new_string(env, str_.is_null(), len, out, |s| {
        v8::String::new_from_two_byte(s, utf16_arg(str_, len), v8::NewStringType::Normal)
    })
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_latin1(
    env: napi_env,
    str_: *const c_char,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    new_string(env, str_.is_null(), len, out, |s| {
        v8::String::new_from_one_byte(s, latin1_arg(str_, len), v8::NewStringType::Internalized)
    })
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_utf8(
    env: napi_env,
    str_: *const c_char,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    new_string(env, str_.is_null(), len, out, |s| {
        v8::String::new_from_utf8(s, utf8_arg(str_, len), v8::NewStringType::Internalized)
    })
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_utf16(
    env: napi_env,
    str_: *const u16,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    new_string(env, str_.is_null(), len, out, |s| {
        v8::String::new_from_two_byte(s, utf16_arg(str_, len), v8::NewStringType::Internalized)
    })
}

/// The finalizer of an external string, by its buffer: what the string's destructor runs (V8
/// hands the destructor only the buffer). An env torn down first leaves NULL, as Node's
/// TrackedStringResource::Finalize.
struct ExternalString {
    env: napi_env,
    finalize: napi_finalize,
    hint: *mut c_void,
}

// SAFETY: the records are only touched on the isolate's thread (creation, GC, teardown); the
// lock is for the process-wide map.
unsafe impl Send for ExternalString {}

static EXTERNAL_STRINGS: std::sync::Mutex<Vec<(usize, ExternalString)>> =
    std::sync::Mutex::new(Vec::new());

/// The destructor of an external string: its finalizer (if any) runs with the env that made it
/// (NULL once that env is gone), the buffer as its data, as Node's ~TrackedStringResource.
unsafe fn external_string_gone(buffer: usize) {
    let found = {
        let mut list = EXTERNAL_STRINGS.lock().unwrap_or_else(|e| e.into_inner());
        let at = list.iter().position(|(b, _)| *b == buffer);
        at.map(|at| list.remove(at).1)
    };
    if let Some(ExternalString {
        env,
        finalize,
        hint,
    }) = found
    {
        let _ = call_addon(|| finalize(env, buffer as *mut c_void, hint));
    }
}
unsafe extern "C" fn external_onebyte_gone(buffer: *mut c_char, _len: usize) {
    external_string_gone(buffer as usize)
}
unsafe extern "C" fn external_twobyte_gone(buffer: *mut u16, _len: usize) {
    external_string_gone(buffer as usize)
}

/// The external strings `env` made lose their env (it is being torn down): their finalizers,
/// run when V8 frees the strings, get NULL.
fn forget_external_strings(env: napi_env) {
    let mut list = EXTERNAL_STRINGS.lock().unwrap_or_else(|e| e.into_inner());
    for (_, rec) in list.iter_mut().filter(|(_, rec)| rec.env == env) {
        rec.env = std::ptr::null_mut();
    }
}

/// node_api_create_external_string_latin1 / _utf16, as Node without the V8 sandbox: the string
/// is external (`*copied` false), its buffer the caller's, and the finalizer runs when the
/// string is collected (`finalizer`: the finalize callback and its hint). `make` builds the
/// string over the buffer with the destructor.
unsafe fn external_string(
    env: napi_env,
    buffer: *mut c_void,
    len: usize,
    (finalize, hint): (Option<napi_finalize>, *mut c_void),
    out: *mut napi_value,
    copied: *mut bool,
    make: impl FnOnce(&mut v8::PinScope<'static, 'static>) -> Option<v8::Local<'static, v8::String>>,
) -> napi_status {
    let status = new_string(env, buffer.is_null(), len, out, |s| {
        if let Some(finalize) = finalize {
            let rec = ExternalString {
                env,
                finalize,
                hint,
            };
            let mut list = EXTERNAL_STRINGS.lock().unwrap_or_else(|e| e.into_inner());
            list.push((buffer as usize, rec));
        }
        make(s)
    });
    if status == napi_ok && !copied.is_null() {
        *copied = false;
    }
    status
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn node_api_create_external_string_latin1(
    env: napi_env,
    str_: *mut c_char,
    len: usize,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
    out: *mut napi_value,
    copied: *mut bool,
) -> napi_status {
    external_string(env, str_.cast(), len, (finalize, hint), out, copied, |s| {
        let len = latin1_arg(str_, len).len();
        v8::String::new_external_onebyte_raw(s, str_, len, external_onebyte_gone)
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn node_api_create_external_string_utf16(
    env: napi_env,
    str_: *mut u16,
    len: usize,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
    out: *mut napi_value,
    copied: *mut bool,
) -> napi_status {
    external_string(env, str_.cast(), len, (finalize, hint), out, copied, |s| {
        let len = utf16_arg(str_, len).len();
        v8::String::new_external_twobyte_raw(s, str_, len, external_twobyte_gone)
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_symbol(
    env: napi_env,
    description: napi_value,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        let out = out_arg(out)?;
        let description = if description.is_null() {
            None
        } else {
            // as Node: a description must be a string
            let d: v8::Local<v8::String> = to_local(description)
                .try_into()
                .map_err(|_| napi_string_expected)?;
            Some(d)
        };
        *out = to_napi(v8::Symbol::new(scope(env), description).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn node_api_symbol_for(
    env: napi_env,
    description: *const c_char,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        let out = out_arg(out)?;
        // as Node: the description is made like napi_create_string_utf8 (its checks and status)
        let mut key: napi_value = std::ptr::null_mut();
        let status = napi_create_string_utf8(env, description, len, &mut key);
        if status != napi_ok {
            return Err(status);
        }
        let key: v8::Local<v8::String> = to_local(key).try_into().unwrap();
        *out = to_napi(v8::Symbol::for_key(scope(env), key).into());
        Ok(())
    })
}

/// The bytes of a `(const char*, size_t)` string argument; NAPI_AUTO_LENGTH means NUL-terminated.
unsafe fn utf8_arg<'a>(str_: *const c_char, len: usize) -> &'a [u8] {
    // (a NULL string of length 0 is allowed: Node's NewString checks it only for a length)
    if str_.is_null() {
        &[]
    } else if len == usize::MAX {
        std::ffi::CStr::from_ptr(str_).to_bytes()
    } else {
        std::slice::from_raw_parts(str_ as *const u8, len)
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf8(
    env: napi_env,
    v: napi_value,
    buf: *mut c_char,
    bufsize: usize,
    result: *mut usize,
) -> napi_status {
    basic(env, || {
        let s = scope(env);
        let str_val: v8::Local<v8::String> =
            val_arg(v)?.try_into().map_err(|_| napi_string_expected)?;
        // as Node: a NULL buf is a length query, which needs `result` (CHECK_ARG(env, result))
        if buf.is_null() {
            *out_arg(result)? = str_val.utf8_length(s);
            return Ok(());
        }
        // As Node: bufsize 0 writes nothing; otherwise at most bufsize - 1 bytes of whole UTF-8
        // characters (never a split sequence; lone surrogates become U+FFFD) plus the NUL.
        let n = if bufsize == 0 {
            0
        } else {
            let dst = std::slice::from_raw_parts_mut(buf as *mut u8, bufsize - 1);
            let n = str_val.write_utf8_v2(s, dst, v8::WriteFlags::kReplaceInvalidUtf8, None);
            *buf.add(n) = 0;
            n
        };
        if !result.is_null() {
            *result = n;
        }
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_latin1(
    env: napi_env,
    v: napi_value,
    buf: *mut c_char,
    bufsize: usize,
    result: *mut usize,
) -> napi_status {
    get_string_units(env, v, buf.cast::<u8>(), bufsize, result, |s, str_, dst| {
        str_.write_one_byte_v2(s, 0, dst, v8::WriteFlags::empty())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf16(
    env: napi_env,
    v: napi_value,
    buf: *mut u16,
    bufsize: usize,
    result: *mut usize,
) -> napi_status {
    get_string_units(env, v, buf, bufsize, result, |s, str_, dst| {
        str_.write_v2(s, 0, dst, v8::WriteFlags::empty())
    })
}

/// napi_get_value_string_latin1 / _utf16, as Node: a NULL buffer is a length query (in code
/// units; it needs `result`), a bufsize of 0 writes nothing, otherwise up to bufsize - 1 code
/// units are written and NUL-terminated.
unsafe fn get_string_units<T: Copy + Default>(
    env: napi_env,
    v: napi_value,
    buf: *mut T,
    bufsize: usize,
    result: *mut usize,
    write: impl FnOnce(&mut v8::PinScope<'static, 'static>, v8::Local<v8::String>, &mut [T]),
) -> napi_status {
    basic(env, || {
        let str_: v8::Local<v8::String> =
            val_arg(v)?.try_into().map_err(|_| napi_string_expected)?;
        if buf.is_null() {
            *out_arg(result)? = str_.length();
            return Ok(());
        }
        let n = if bufsize == 0 {
            0
        } else {
            let n = (bufsize - 1).min(str_.length());
            write(scope(env), str_, std::slice::from_raw_parts_mut(buf, n));
            *buf.add(n) = T::default();
            n
        };
        if !result.is_null() {
            *result = n;
        }
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_bool(
    env: napi_env,
    v: napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(v)?;
        *out_arg(out)? = to_napi(value.to_boolean(s).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_number(
    env: napi_env,
    v: napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(v)?;
        let out = out_arg(out)?;
        // as Node's CHECK_TO_NUMBER: a ToNumber that throws is napi_number_expected
        *out = to_napi(value.to_number(s).ok_or(napi_number_expected)?.into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_object(
    env: napi_env,
    v: napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        val_arg(v)?;
        let out = out_arg(out)?;
        *out = to_napi(to_object(s, v)?.into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_string(
    env: napi_env,
    v: napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(v)?;
        let out = out_arg(out)?;
        // as Node's CHECK_TO_STRING: a ToString that throws is napi_string_expected
        *out = to_napi(value.to_string(s).ok_or(napi_string_expected)?.into());
        Ok(())
    })
}

// ---- objects / arrays -----------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_array(env: napi_env, out: *mut napi_value) -> napi_status {
    basic(env, || {
        *out_arg(out)? = to_napi(v8::Array::new(scope(env), 0).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_object(env: napi_env, out: *mut napi_value) -> napi_status {
    basic(env, || {
        let s = scope(env);
        *out_arg(out)? = to_napi(v8::Object::new(s).into());
        Ok(())
    })
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_array_with_length(
    env: napi_env,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        let s = scope(env);
        *out_arg(out)? = to_napi(v8::Array::new(s, len as i32).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_named_property(
    env: napi_env,
    obj: napi_value,
    name: *const c_char,
    value: napi_value,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(value)?;
        let o = obj_arg(s, obj)?;
        let k = cstr_arg(s, name)?;
        // V8's Object::Set is Nothing only when it threw (a throwing setter or proxy trap)
        v8_true(o.set(s, k.into(), value))
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_named_property(
    env: napi_env,
    obj: napi_value,
    name: *const c_char,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let k = cstr_arg(s, name)?;
        let o = obj_arg(s, obj)?;
        *out = to_napi(o.get(s, k.into()).ok_or(V8_GAVE_NOTHING)?);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_element(
    env: napi_env,
    obj: napi_value,
    index: u32,
    value: napi_value,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(value)?;
        let o = obj_arg(s, obj)?;
        v8_true(o.set_index(s, index, value))
    })
}

// ---- comparisons and type checks ---------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_strict_equals(
    env: napi_env,
    lhs: napi_value,
    rhs: napi_value,
    out: *mut bool,
) -> napi_status {
    js_call(env, |_| {
        let (a, b) = (val_arg(lhs)?, val_arg(rhs)?);
        *out_arg(out)? = a.strict_equals(b);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_instanceof(
    env: napi_env,
    object: napi_value,
    constructor: napi_value,
    out: *mut bool,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(object)?;
        let out = out_arg(out)?;
        *out = false;
        let ctor = obj_arg(s, constructor)?;
        // as Node: a constructor that is not a function throws a TypeError
        // (ERR_NAPI_CONS_FUNCTION) and the call is napi_function_expected
        if !ctor.is_function() {
            let msg = v8::String::new(s, "Constructor must be a function").unwrap();
            let err = new_error(s, ErrorKind::Type, msg);
            let code = v8::String::new(s, "ERR_NAPI_CONS_FUNCTION").unwrap();
            set_error_code(s, err, code)?;
            s.throw_exception(err);
            return Err(napi_function_expected);
        }
        // as Node's CHECK_MAYBE_NOTHING: an InstanceOf that throws is napi_generic_failure (the
        // exception pending)
        *out = value.instance_of(s, ctor).ok_or(napi_generic_failure)?;
        Ok(())
    })
}

/// A type test of a value (napi_is_arraybuffer, ...): the value and the result are required.
unsafe fn is_kind(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
    test: impl FnOnce(v8::Local<v8::Value>) -> bool,
) -> napi_status {
    basic(env, || {
        let value = val_arg(v)?;
        *out_arg(out)? = test(value);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_arraybuffer(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    is_kind(env, v, out, |v| v.is_array_buffer())
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_typedarray(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    is_kind(env, v, out, |v| v.is_typed_array())
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_dataview(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    is_kind(env, v, out, |v| v.is_data_view())
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_buffer(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    // as Node's node::Buffer::HasInstance: any ArrayBufferView
    is_kind(env, v, out, |v| v.is_array_buffer_view())
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_detached_arraybuffer(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    is_kind(env, v, out, |v| {
        v8::Local::<v8::ArrayBuffer>::try_from(v).is_ok_and(|ab| ab.was_detached())
    })
}

// ---- properties ------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_set_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    value: napi_value,
) -> napi_status {
    js_call(env, |s| {
        let (k, v) = (val_arg(key)?, val_arg(value)?);
        let o = obj_arg(s, object)?;
        v8_true(o.set(s, k, v))
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let k = val_arg(key)?;
        let out = out_arg(out)?;
        let o = obj_arg(s, object)?;
        *out = to_napi(o.get(s, k).ok_or(V8_GAVE_NOTHING)?);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    out: *mut bool,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let k = val_arg(key)?;
        let o = obj_arg(s, object)?;
        *out = o.has(s, k).ok_or(V8_GAVE_NOTHING)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    out: *mut bool,
) -> napi_status {
    js_call(env, |s| {
        let k = val_arg(key)?;
        let o = obj_arg(s, object)?;
        // as Node: the result is optional (false: the property is non-configurable)
        let deleted = o.delete(s, k).ok_or(V8_GAVE_NOTHING)?;
        if !out.is_null() {
            *out = deleted;
        }
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_own_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    out: *mut bool,
) -> napi_status {
    js_call(env, |s| {
        let k = val_arg(key)?;
        let out = out_arg(out)?;
        let o = obj_arg(s, object)?;
        // as Node: the key must be a string or a symbol
        let k: v8::Local<v8::Name> = k.try_into().map_err(|_| napi_name_expected)?;
        *out = o.has_own_property(s, k).ok_or(V8_GAVE_NOTHING)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_named_property(
    env: napi_env,
    object: napi_value,
    name: *const c_char,
    out: *mut bool,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let o = obj_arg(s, object)?;
        let k = cstr_arg(s, name)?;
        *out = o.has(s, k.into()).ok_or(V8_GAVE_NOTHING)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_element(
    env: napi_env,
    object: napi_value,
    index: u32,
    out: *mut bool,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let o = obj_arg(s, object)?;
        *out = o.has_index(s, index).ok_or(V8_GAVE_NOTHING)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_element(
    env: napi_env,
    object: napi_value,
    index: u32,
    out: *mut bool,
) -> napi_status {
    js_call(env, |s| {
        let o = obj_arg(s, object)?;
        let deleted = o.delete_index(s, index).ok_or(V8_GAVE_NOTHING)?;
        if !out.is_null() {
            *out = deleted;
        }
        Ok(())
    })
}

/// napi_key_filter bits and the V8 property filters they select.
const KEY_FILTERS: [(i32, v8::PropertyFilter); 5] = [
    (1, v8::PropertyFilter::ONLY_WRITABLE),
    (1 << 1, v8::PropertyFilter::ONLY_ENUMERABLE),
    (1 << 2, v8::PropertyFilter::ONLY_CONFIGURABLE),
    (1 << 3, v8::PropertyFilter::SKIP_STRINGS),
    (1 << 4, v8::PropertyFilter::SKIP_SYMBOLS),
];

#[no_mangle]
pub unsafe extern "C" fn napi_get_all_property_names(
    env: napi_env,
    object: napi_value,
    key_mode: i32,
    key_filter: i32,
    key_conversion: i32,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let o = obj_arg(s, object)?;
        let property_filter = KEY_FILTERS
            .iter()
            .filter(|(bit, _)| key_filter & bit != 0)
            .fold(v8::PropertyFilter::ALL_PROPERTIES, |f, &(_, v8f)| f | v8f);
        // as Node: an unknown napi_key_collection_mode / napi_key_conversion is napi_invalid_arg
        let mode = match key_mode {
            0 => v8::KeyCollectionMode::IncludePrototypes,
            1 => v8::KeyCollectionMode::OwnOnly,
            _ => return Err(napi_invalid_arg),
        };
        let key_conversion = match key_conversion {
            0 => v8::KeyConversionMode::KeepNumbers,
            1 => v8::KeyConversionMode::ConvertToString,
            _ => return Err(napi_invalid_arg),
        };
        let args = v8::GetPropertyNamesArgs {
            mode,
            property_filter,
            index_filter: v8::IndexFilter::IncludeIndices,
            key_conversion,
        };
        *out = to_napi(o.get_property_names(s, args).ok_or(V8_GAVE_NOTHING)?.into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_prototype(
    env: napi_env,
    object: napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let o = obj_arg(s, object)?;
        // (V8's GetPrototypeV2: a Proxy's getPrototypeOf trap is not run)
        *out = to_napi(o.get_prototype(s).ok_or(V8_GAVE_NOTHING)?);
        Ok(())
    })
}

/// napi_object_freeze / napi_object_seal: SetIntegrityLevel (napi_generic_failure, or the
/// exception a Proxy trap threw pending, when it fails).
unsafe fn set_integrity(
    env: napi_env,
    object: napi_value,
    level: v8::IntegrityLevel,
) -> napi_status {
    js_call(env, |s| {
        let o = obj_arg(s, object)?;
        v8_true(o.set_integrity_level(s, level))
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_object_freeze(env: napi_env, object: napi_value) -> napi_status {
    set_integrity(env, object, v8::IntegrityLevel::Frozen)
}

#[no_mangle]
pub unsafe extern "C" fn napi_object_seal(env: napi_env, object: napi_value) -> napi_status {
    set_integrity(env, object, v8::IntegrityLevel::Sealed)
}

// ---- functions ------------------------------------------------------------

/// Trampoline: every napi function shares this rusty_v8 callback. It recovers the function's
/// `Callback` record from the External stored as its data, packages CbInfo, points the
/// owning env's scope at the current scope, and calls the addon callback.
fn napi_fn_trampoline(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Ok(record) = v8::Local::<v8::External>::try_from(args.data()) else {
        return;
    };
    let Callback {
        env: env_ptr,
        cb,
        data,
    } = unsafe { *(record.value() as *const Callback) };
    let argv: Vec<napi_value> = (0..args.length())
        .map(|i| unsafe { to_napi(args.get(i)) })
        .collect();
    let new_target = if args.is_construct_call() {
        unsafe { to_napi(args.new_target()) }
    } else {
        std::ptr::null_mut()
    };
    let mut info = CbInfo {
        args: argv,
        this: unsafe { to_napi(args.this().into()) },
        data,
        new_target,
    };
    // Point the env at this call's scope for the callback's duration and restore it after: a
    // nested call (the addon calling JS that calls the addon again) must not leave the outer
    // callback with the inner, already-dead scope.
    let here = scope as *mut v8::PinScope as *mut c_void;
    let outer = unsafe { std::mem::replace(&mut (*env_ptr).scope, here) };
    // The .node addon (turbo-dom parser) is one shared library across all worker threads, so
    // its internal (C/Rust) state is process-global and NOT thread-safe — concurrent calls
    // race (intermittent "document is not defined" / load flakiness). Serialize native addon
    // entry across threads with a global lock; a thread-local guard allows same-thread
    // re-entry (the addon calling a JS callback that re-enters) without self-deadlock.
    // Calling the addon's native callback crosses the FFI boundary. A panic there is UB and
    // can crash the process; call_addon converts it into a thrown JS error instead.
    let ret = call_addon(|| unsafe { cb(env_ptr, &mut info as *mut CbInfo) });
    unsafe { (*env_ptr).scope = outer };
    // As Node's CallIntoModule: an exception the callback left pending is thrown to its caller.
    if let Some(exc) = unsafe { (*env_ptr).last_exception.take() } {
        let exc = v8::Local::new(scope, &exc);
        scope.throw_exception(exc);
    }
    if let Some(ret) = callback_return(scope, ret) {
        rv.set(unsafe { to_local(ret) });
    }
}

/// Enter native addon code on this thread: `(reentrant, guard)`. The outermost entry marks the
/// thread as inside the addon and takes the process-global `ADDON_LOCK`; a same-thread re-entry
/// takes no lock (it would self-deadlock). The caller clears `IN_ADDON` when not `reentrant`.
fn enter_addon() -> (bool, Option<std::sync::MutexGuard<'static, ()>>) {
    let reentrant = IN_ADDON.with(|f| f.get());
    let guard = if reentrant {
        None
    } else {
        IN_ADDON.with(|f| f.set(true));
        Some(ADDON_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
    };
    (reentrant, guard)
}

/// Run native addon code (a callback, a module init) under `enter_addon`, catching a Rust panic
/// out of it (one crossing the `extern "C"` boundary is UB) as an `Err`.
fn call_addon<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    let (reentrant, _guard) = enter_addon();
    let ret = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    if !reentrant {
        IN_ADDON.with(|f| f.set(false));
    }
    ret
}

/// The JS-visible outcome of a native callback: a non-null `napi_value` is the return value,
/// NULL leaves the return slot `undefined` (`None`), and a panic is thrown as a JS error.
fn callback_return(
    scope: &mut v8::PinScope,
    ret: std::thread::Result<napi_value>,
) -> Option<napi_value> {
    match ret {
        Ok(ret) if !ret.is_null() => Some(ret),
        Ok(_) => None,
        Err(_) => {
            if let Some(msg) = v8::String::new(scope, "turbo-test: native addon callback panicked")
            {
                let exc = v8::Exception::error(scope, msg);
                scope.throw_exception(exc);
            }
            None
        }
    }
}

/// A panic out of the addon's module init becomes a load error naming the addon.
fn init_outcome(
    result: std::thread::Result<napi_value>,
    path: &Path,
) -> Result<napi_value, String> {
    result.map_err(|_| {
        format!(
            "native addon panicked during module init: {}",
            path.display()
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_function(
    env: napi_env,
    name: *const c_char,
    len: usize,
    cb: Option<napi_callback>,
    data: *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let cb = cb.ok_or(napi_invalid_arg)?;
        let f = new_function(s, env, cb, data)?;
        // as Node: a non-NULL name becomes the function's `name` (a name V8 cannot hold is
        // napi_generic_failure)
        if !name.is_null() {
            let name = utf8_arg(name, len);
            let name = v8::String::new_from_utf8(s, name, v8::NewStringType::Internalized)
                .ok_or(napi_generic_failure)?;
            f.set_name(name);
        }
        *out = to_napi(f.into());
        Ok(())
    })
}

/// A function template dispatching to the addon callback `cb` with `data` (its `Callback`
/// record is owned by the env), optionally with a receiver `signature` — Node's
/// FunctionCallbackWrapper::NewTemplate.
unsafe fn callback_template<'s>(
    s: &mut v8::PinScope<'s, 'static>,
    env: napi_env,
    cb: napi_callback,
    data: *mut c_void,
    signature: Option<v8::Local<'s, v8::Signature>>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let record = Box::into_raw(Box::new(Callback { env, cb, data }));
    (*env).fns.push(record);
    let data_val = v8::External::new(s, record as *mut c_void);
    let builder = v8::FunctionTemplate::builder(napi_fn_trampoline).data(data_val.into());
    match signature {
        Some(sig) => builder.signature(sig).build(s),
        None => builder.build(s),
    }
}

/// An unnamed function dispatching to `cb` (Node's FunctionCallbackWrapper::NewFunction).
unsafe fn new_function<'s>(
    s: &mut v8::PinScope<'s, 'static>,
    env: napi_env,
    cb: napi_callback,
    data: *mut c_void,
) -> Result<v8::Local<'s, v8::Function>, napi_status> {
    let tmpl = callback_template(s, env, cb, data, None);
    tmpl.get_function(s).ok_or(napi_generic_failure)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_new_target(
    env: napi_env,
    info: napi_callback_info,
    result: *mut napi_value,
) -> napi_status {
    checked(env, || {
        // as Node: both arguments are required; a plain (non-construct) call gives NULL
        if info.is_null() || result.is_null() {
            return napi_invalid_arg;
        }
        *result = (*info).new_target;
        napi_ok
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_cb_info(
    env: napi_env,
    info: napi_callback_info,
    argc: *mut usize,
    argv: *mut napi_value,
    this: *mut napi_value,
    data: *mut *mut c_void,
) -> napi_status {
    basic(env, || {
        // as Node: the info is required, and so is argc when argv is given
        let cb = info.as_ref().ok_or(napi_invalid_arg)?;
        if !argv.is_null() {
            let want = *out_arg(argc)?;
            let n = want.min(cb.args.len());
            argv.copy_from_nonoverlapping(cb.args.as_ptr(), n);
            // as in Node, slots past the actual arguments are filled with `undefined`
            let undefined = to_napi(v8::undefined(scope(env)).into());
            for i in n..want {
                *argv.add(i) = undefined;
            }
        }
        if !argc.is_null() {
            *argc = cb.args.len();
        }
        if !this.is_null() {
            *this = cb.this;
        }
        if !data.is_null() {
            *data = cb.data;
        }
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_function(
    env: napi_env,
    recv: napi_value,
    func: napi_value,
    argc: usize,
    argv: *const napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        // as Node (CHECK_ARG(env, recv)): a receiver is required; an addon passes `undefined`.
        // So is argv when argc > 0 (CHECK_ARG(env, argv)): nothing is called.
        if recv.is_null() || (argc > 0 && argv.is_null()) {
            return Err(napi_invalid_arg);
        }
        let f: v8::Local<v8::Function> = val_arg(func)?.try_into().map_err(|_| napi_invalid_arg)?;
        let args: Vec<v8::Local<v8::Value>> = (0..argc).map(|i| to_local(*argv.add(i))).collect();
        put(
            out,
            f.call(s, to_local(recv), &args)
                .ok_or(napi_pending_exception)?,
        );
        Ok(())
    })
}

// ---- arraybuffer / typedarray (for the parser's SoA buffer) ----------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_arraybuffer(
    env: napi_env,
    len: usize,
    data: *mut *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let ab = v8::ArrayBuffer::new(s, len);
        if !data.is_null() {
            let store = ab.get_backing_store();
            *data = store
                .data()
                .map(|p| p.as_ptr())
                .unwrap_or(std::ptr::null_mut());
        }
        *out = to_napi(ab.into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_typedarray(
    env: napi_env,
    type_: i32,
    length: usize,
    arraybuffer: napi_value,
    byte_offset: usize,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(arraybuffer)?;
        let out = out_arg(out)?;
        // as Node: an ArrayBuffer or a SharedArrayBuffer (one JSArrayBuffer in V8, so a
        // SharedArrayBuffer handle serves where V8's typed array constructors take an
        // ArrayBuffer)
        if !(value.is_array_buffer() || value.is_shared_array_buffer()) {
            return Err(napi_invalid_arg);
        }
        let ab: v8::Local<v8::ArrayBuffer> = v8::Local::cast_unchecked(value);
        // napi_typedarray_type codes (Node 26: 0..=11): element size and constructor name.
        let (size, name) = match type_ {
            0 => (1, "Int8Array"),
            1 => (1, "Uint8Array"),
            2 => (1, "Uint8ClampedArray"),
            3 => (2, "Int16Array"),
            4 => (2, "Uint16Array"),
            5 => (4, "Int32Array"),
            6 => (4, "Uint32Array"),
            7 => (4, "Float32Array"),
            8 => (8, "Float64Array"),
            9 => (8, "BigInt64Array"),
            10 => (8, "BigUint64Array"),
            11 => (2, "Float16Array"),
            // as Node: an unknown napi_typedarray_type is napi_invalid_arg
            _ => return Err(napi_invalid_arg),
        };
        // As Node (THROW_RANGE_ERROR_IF_FALSE): a misaligned offset or a view running past the
        // buffer throws a RangeError; V8's constructors would not check (or abort). `length` is
        // in ELEMENTS; a product overflowing usize is a length error too.
        if !byte_offset.is_multiple_of(size) {
            let msg = format!("start offset of {name} should be a multiple of {size}");
            return throw_range_error(s, "ERR_NAPI_INVALID_TYPEDARRAY_ALIGNMENT", &msg);
        }
        let end = length
            .checked_mul(size)
            .and_then(|n| n.checked_add(byte_offset));
        if end.is_none_or(|end| end > ab.byte_length()) {
            let code = "ERR_NAPI_INVALID_TYPEDARRAY_LENGTH";
            return throw_range_error(s, code, "Invalid typed array length");
        }
        let (o, n) = (byte_offset, length);
        let ta: Option<v8::Local<v8::Value>> = match type_ {
            0 => v8::Int8Array::new(s, ab, o, n).map(Into::into),
            1 => v8::Uint8Array::new(s, ab, o, n).map(Into::into),
            2 => v8::Uint8ClampedArray::new(s, ab, o, n).map(Into::into),
            3 => v8::Int16Array::new(s, ab, o, n).map(Into::into),
            4 => v8::Uint16Array::new(s, ab, o, n).map(Into::into),
            5 => v8::Int32Array::new(s, ab, o, n).map(Into::into),
            6 => v8::Uint32Array::new(s, ab, o, n).map(Into::into),
            7 => v8::Float32Array::new(s, ab, o, n).map(Into::into),
            8 => v8::Float64Array::new(s, ab, o, n).map(Into::into),
            9 => v8::BigInt64Array::new(s, ab, o, n).map(Into::into),
            // (this V8 binding has no Float16Array -> Value `From`; the upcast is always valid)
            11 => v8::Float16Array::new(s, ab, o, n).map(|t| v8::Local::cast_unchecked(t)),
            _ => v8::BigUint64Array::new(s, ab, o, n).map(Into::into), // 10
        };
        *out = to_napi(ta.ok_or(napi_generic_failure)?);
        Ok(())
    })
}

/// Throw a RangeError carrying `code`, as Node's napi_throw_range_error, and fail the call with
/// napi_generic_failure.
unsafe fn throw_range_error(
    s: &mut v8::PinScope<'static, 'static>,
    code: &str,
    msg: &str,
) -> NapiResult {
    let msg = v8::String::new(s, msg).ok_or(napi_generic_failure)?;
    let code = v8::String::new(s, code).ok_or(napi_generic_failure)?;
    let err = v8::Exception::range_error(s, msg);
    set_error_code(s, err, code)?;
    s.throw_exception(err);
    Err(napi_generic_failure)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external_arraybuffer(
    env: napi_env,
    data: *mut c_void,
    len: usize,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        // as Node (napi_create_external_buffer, then its ArrayBuffer): a NULL result is
        // allowed, the buffer is made all the same
        let ab = external_arraybuffer(s, env, data, len, finalize, hint);
        put(out, ab.into());
        Ok(())
    })
}

/// The backing-store deleter of an external ArrayBuffer: the memory is the addon's, released by
/// its finalizer.
unsafe extern "C" fn keep_external(_data: *mut c_void, _len: usize, _deleter_data: *mut c_void) {}

/// An ArrayBuffer over the addon's `data`, not copied (as Node without the V8 sandbox): writes
/// on either side are seen by the other, and the finalizer runs once the ArrayBuffer is
/// collected (or at env teardown), as Node's BufferFinalizer. NULL data (Node would hand out a
/// buffer over address 0) gets zero-filled memory of its own.
unsafe fn external_arraybuffer(
    s: &mut v8::PinScope<'static, 'static>,
    env: napi_env,
    data: *mut c_void,
    len: usize,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
) -> v8::Local<'static, v8::ArrayBuffer> {
    if data.is_null() {
        return v8::ArrayBuffer::new(s, len);
    }
    let store =
        v8::ArrayBuffer::new_backing_store_from_ptr(data, len, keep_external, std::ptr::null_mut());
    let ab = v8::ArrayBuffer::with_backing_store(s, &store.make_shared());
    if finalize.is_some() {
        track_finalizer(s, env, ab.into(), data, finalize, hint);
    }
    ab
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_arraybuffer_info(
    env: napi_env,
    v: napi_value,
    data: *mut *mut c_void,
    len: *mut usize,
) -> napi_status {
    basic(env, || {
        let value = val_arg(v)?;
        // as Node: an ArrayBuffer or a SharedArrayBuffer; data and length are optional
        let store = if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
            ab.get_backing_store()
        } else {
            let sab: v8::Local<v8::SharedArrayBuffer> =
                value.try_into().map_err(|_| napi_invalid_arg)?;
            sab.get_backing_store()
        };
        if !data.is_null() {
            *data = store.data().map_or(std::ptr::null_mut(), |p| p.as_ptr());
        }
        if !len.is_null() {
            *len = store.byte_length();
        }
        Ok(())
    })
}

/// The napi_typedarray_type of a typed array, in Node's test order.
fn typedarray_type(v: v8::Local<v8::Value>) -> i32 {
    let tests: [fn(&v8::Value) -> bool; 11] = [
        v8::Value::is_int8_array,
        v8::Value::is_uint8_array,
        v8::Value::is_uint8_clamped_array,
        v8::Value::is_int16_array,
        v8::Value::is_uint16_array,
        v8::Value::is_int32_array,
        v8::Value::is_uint32_array,
        v8::Value::is_float16_array,
        v8::Value::is_float32_array,
        v8::Value::is_float64_array,
        v8::Value::is_big_int64_array,
    ];
    const CODES: [i32; 11] = [0, 1, 2, 3, 4, 5, 6, 11, 7, 8, 9];
    tests
        .iter()
        .position(|test| test(&v))
        .map_or(10, |i| CODES[i]) // napi_biguint64_array
}

/// The data pointer, buffer and offset a typed array or DataView reports (Node's
/// napi_get_typedarray_info / napi_get_dataview_info: each out-parameter optional).
unsafe fn view_info(
    s: &mut v8::PinScope<'static, 'static>,
    view: v8::Local<v8::ArrayBufferView>,
    data: *mut *mut c_void,
    arraybuffer: *mut napi_value,
    byte_offset: *mut usize,
) {
    if !data.is_null() || !arraybuffer.is_null() {
        let buffer = view.buffer(s).unwrap();
        if !data.is_null() {
            let base = buffer.data().map_or(std::ptr::null_mut(), |p| p.as_ptr());
            *data = base.cast::<u8>().wrapping_add(view.byte_offset()).cast();
        }
        put(arraybuffer, buffer.into());
    }
    if !byte_offset.is_null() {
        *byte_offset = view.byte_offset();
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_typedarray_info(
    env: napi_env,
    v: napi_value,
    type_: *mut i32,
    length: *mut usize,
    data: *mut *mut c_void,
    arraybuffer: *mut napi_value,
    byte_offset: *mut usize,
) -> napi_status {
    basic(env, || {
        let value = val_arg(v)?;
        let ta: v8::Local<v8::TypedArray> = value.try_into().map_err(|_| napi_invalid_arg)?;
        if !type_.is_null() {
            *type_ = typedarray_type(value);
        }
        if !length.is_null() {
            *length = ta.length();
        }
        view_info(scope(env), ta.into(), data, arraybuffer, byte_offset);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_dataview(
    env: napi_env,
    byte_length: usize,
    arraybuffer: napi_value,
    byte_offset: usize,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(arraybuffer)?;
        let out = out_arg(out)?;
        // as Node: an ArrayBuffer or a SharedArrayBuffer (one JSArrayBuffer in V8, so a
        // SharedArrayBuffer handle serves where V8's DataView::New takes an ArrayBuffer)
        let ab: v8::Local<v8::ArrayBuffer> =
            if value.is_array_buffer() || value.is_shared_array_buffer() {
                v8::Local::cast_unchecked(value)
            } else {
                return Err(napi_invalid_arg);
            };
        let end = byte_length.checked_add(byte_offset);
        if end.is_none_or(|end| end > ab.byte_length()) {
            let msg = "byte_offset + byte_length should be less than or equal to the size in \
                       bytes of the array passed in";
            // as Node: the RangeError is pending and the call napi_pending_exception
            let _ = throw_range_error(s, "ERR_NAPI_INVALID_DATAVIEW_ARGS", msg);
            return Err(napi_pending_exception);
        }
        *out = to_napi(v8::DataView::new(s, ab, byte_offset, byte_length).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_dataview_info(
    env: napi_env,
    v: napi_value,
    byte_length: *mut usize,
    data: *mut *mut c_void,
    arraybuffer: *mut napi_value,
    byte_offset: *mut usize,
) -> napi_status {
    basic(env, || {
        let value = val_arg(v)?;
        let dv: v8::Local<v8::DataView> = value.try_into().map_err(|_| napi_invalid_arg)?;
        if !byte_length.is_null() {
            *byte_length = dv.byte_length();
        }
        view_info(scope(env), dv.into(), data, arraybuffer, byte_offset);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_detach_arraybuffer(env: napi_env, v: napi_value) -> napi_status {
    basic(env, || {
        let ab: v8::Local<v8::ArrayBuffer> = val_arg(v)?
            .try_into()
            .map_err(|_| napi_arraybuffer_expected)?;
        if !ab.is_detachable() {
            return Err(napi_detachable_arraybuffer_expected);
        }
        ab.detach(None);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_buffer_from_arraybuffer(
    env: napi_env,
    arraybuffer: napi_value,
    byte_offset: usize,
    byte_length: usize,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(arraybuffer)?;
        let out = out_arg(out)?;
        let ab: v8::Local<v8::ArrayBuffer> = value.try_into().map_err(|_| napi_invalid_arg)?;
        let end = byte_offset.checked_add(byte_length);
        if end.is_none_or(|end| end > ab.byte_length()) {
            // as Node: the RangeError is thrown and the call returns napi_throw_range_error's
            // status, napi_ok
            let msg = v8::String::new(s, "The byte offset + length is out of range").unwrap();
            let err = new_error(s, ErrorKind::Range, msg);
            let code = v8::String::new(s, "ERR_OUT_OF_RANGE").unwrap();
            set_error_code(s, err, code)?;
            s.throw_exception(err);
            return Ok(());
        }
        // a Buffer over the same memory
        *out = to_napi(new_buffer(s, ab, byte_offset, byte_length).unwrap());
        Ok(())
    })
}

// ---- buffers (Node `Buffer` == Uint8Array; addons returning bytes use these) ---
// turbo-html2pdf and most napi-rs addons that hand back binary data (PDF bytes,
// images, etc.) call the buffer family. These were previously UNEXPORTED, so the
// addon's reference resolved (flat namespace, -export_dynamic) to address 0x0 and
// the first call jumped to NULL -> SIGSEGV that killed the whole run with no output.
// Back them with a Uint8Array (the runtime exposes Buffer as a Uint8Array subclass),
// which is what real Buffer-consuming JS expects.

/// A Buffer over `ab`, as Node's node::Buffer::New: a Uint8Array with the runtime's
/// Buffer.prototype (so Buffer.isBuffer and the Buffer methods apply).
fn new_buffer<'s>(
    s: &mut v8::PinScope<'s, 'static>,
    ab: v8::Local<'s, v8::ArrayBuffer>,
    offset: usize,
    len: usize,
) -> Option<v8::Local<'s, v8::Value>> {
    let buf = v8::Uint8Array::new(s, ab, offset, len)?;
    let global = s.get_current_context().global(s);
    let ctor = v8::String::new(s, "Buffer").unwrap();
    let key = v8::String::new(s, "prototype").unwrap();
    let proto = global
        .get(s, ctor.into())
        .and_then(|b| v8::Local::<v8::Object>::try_from(b).ok())
        .and_then(|b| b.get(s, key.into()));
    if let Some(proto) = proto {
        buf.set_prototype(s, proto);
    }
    Some(buf.into())
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer(
    env: napi_env,
    len: usize,
    data: *mut *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let ab = v8::ArrayBuffer::new(s, len);
        if !data.is_null() {
            *data = ab
                .get_backing_store()
                .data()
                .map(|p| p.as_ptr())
                .unwrap_or(std::ptr::null_mut());
        }
        let buf = new_buffer(s, ab, 0, len).unwrap_or_else(|| ab.into());
        *out = to_napi(buf);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer_copy(
    env: napi_env,
    len: usize,
    src: *const c_void,
    result_data: *mut *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let ab = v8::ArrayBuffer::new(s, len);
        if let Some(store) = ab.get_backing_store().data() {
            if !src.is_null() && len > 0 {
                std::ptr::copy_nonoverlapping(src as *const u8, store.as_ptr() as *mut u8, len);
            }
            if !result_data.is_null() {
                *result_data = store.as_ptr();
            }
        } else if !result_data.is_null() {
            *result_data = std::ptr::null_mut();
        }
        let buf = new_buffer(s, ab, 0, len).unwrap_or_else(|| ab.into());
        *out = to_napi(buf);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external_buffer(
    env: napi_env,
    len: usize,
    data: *mut c_void,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let ab = external_arraybuffer(s, env, data, len, finalize, hint);
        let buf = new_buffer(s, ab, 0, len).unwrap_or_else(|| ab.into());
        *out = to_napi(buf);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_buffer_info(
    env: napi_env,
    v: napi_value,
    data: *mut *mut c_void,
    length: *mut usize,
) -> napi_status {
    basic(env, || {
        let s = scope(env);
        // As Node (node::Buffer::HasInstance): any ArrayBufferView (Buffer, Uint8Array, DataView,
        // ...); anything else, a raw ArrayBuffer included, is napi_invalid_arg.
        let view: v8::Local<v8::ArrayBufferView> =
            val_arg(v)?.try_into().map_err(|_| napi_invalid_arg)?;
        let len = view.byte_length();
        if !length.is_null() {
            *length = len;
        }
        if !data.is_null() {
            let off = view.byte_offset();
            *data = view
                .buffer(s)
                .and_then(|b| b.get_backing_store().data())
                .map(|p| (p.as_ptr() as *mut u8).add(off) as *mut c_void)
                .unwrap_or(std::ptr::null_mut());
        }
        Ok(())
    })
}

// ---- more value getters (bool/double/int64) -------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bool(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    basic(env, || {
        let value = val_arg(v)?;
        let out = out_arg(out)?;
        // as Node: only a JS boolean, no ToBoolean coercion
        let b: v8::Local<v8::Boolean> = value.try_into().map_err(|_| napi_boolean_expected)?;
        *out = b.is_true();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_double(
    env: napi_env,
    v: napi_value,
    out: *mut f64,
) -> napi_status {
    basic(env, || {
        val_arg(v)?;
        *out_arg(out)? = number_of(v).ok_or(napi_number_expected)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int64(
    env: napi_env,
    v: napi_value,
    out: *mut i64,
) -> napi_status {
    basic(env, || {
        val_arg(v)?;
        let out = out_arg(out)?;
        let n = number_of(v).ok_or(napi_number_expected)?;
        // as Node: NaN and the infinities are 0; finite values truncate, saturating at the int64
        // range (V8's IntegerValue)
        *out = if n.is_finite() { n as i64 } else { 0 };
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_int32(
    env: napi_env,
    n: i32,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        *out_arg(out)? = to_napi(v8::Integer::new(scope(env), n).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_int64(
    env: napi_env,
    n: i64,
    out: *mut napi_value,
) -> napi_status {
    // as Node: a Number (the nearest double), not a BigInt
    basic(env, || {
        *out_arg(out)? = to_napi(v8::Number::new(scope(env), n as f64).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int32(
    env: napi_env,
    v: napi_value,
    out: *mut i32,
) -> napi_status {
    basic(env, || {
        val_arg(v)?;
        let out = out_arg(out)?;
        // as Node: ToInt32 of a number (V8's Int32Value): NaN and the infinities are 0
        let n = number_of(v).ok_or(napi_number_expected)?;
        *out = to_uint32(n) as i32;
        Ok(())
    })
}

// ---- bigints ---------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_int64(
    env: napi_env,
    n: i64,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        *out_arg(out)? = to_napi(v8::BigInt::new_from_i64(scope(env), n).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_uint64(
    env: napi_env,
    n: u64,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        *out_arg(out)? = to_napi(v8::BigInt::new_from_u64(scope(env), n).into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_words(
    env: napi_env,
    sign_bit: i32,
    word_count: usize,
    words: *const u64,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        if words.is_null() {
            return Err(napi_invalid_arg);
        }
        let out = out_arg(out)?;
        if word_count > i32::MAX as usize {
            return Err(napi_invalid_arg);
        }
        let words = std::slice::from_raw_parts(words, word_count);
        // V8 throws a RangeError for a BigInt over its maximum size
        let big =
            v8::BigInt::new_from_words(s, sign_bit != 0, words).ok_or(napi_pending_exception)?;
        *out = to_napi(big.into());
        Ok(())
    })
}

/// The BigInt argument of a BigInt getter: NULL is napi_invalid_arg, anything but a BigInt
/// napi_bigint_expected.
unsafe fn bigint_arg(v: napi_value) -> Result<v8::Local<'static, v8::BigInt>, napi_status> {
    val_arg(v)?.try_into().map_err(|_| napi_bigint_expected)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_int64(
    env: napi_env,
    v: napi_value,
    out: *mut i64,
    lossless: *mut bool,
) -> napi_status {
    basic(env, || {
        let (out, lossless) = (out_arg(out)?, out_arg(lossless)?);
        (*out, *lossless) = bigint_arg(v)?.i64_value();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_uint64(
    env: napi_env,
    v: napi_value,
    out: *mut u64,
    lossless: *mut bool,
) -> napi_status {
    basic(env, || {
        let (out, lossless) = (out_arg(out)?, out_arg(lossless)?);
        (*out, *lossless) = bigint_arg(v)?.u64_value();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_words(
    env: napi_env,
    v: napi_value,
    sign_bit: *mut i32,
    word_count: *mut usize,
    words: *mut u64,
) -> napi_status {
    basic(env, || {
        let count = out_arg(word_count)?;
        let big = bigint_arg(v)?;
        // as Node: without a sign and words it is a word-count query; otherwise both are needed
        // and up to *word_count words are written; *word_count becomes the BigInt's count
        if !(sign_bit.is_null() && words.is_null()) {
            let sign = out_arg(sign_bit)?;
            if words.is_null() {
                return Err(napi_invalid_arg);
            }
            let dst = std::slice::from_raw_parts_mut(words, (*count).min(i32::MAX as usize));
            *sign = big.to_words_array(dst).0 as i32;
        }
        *count = big.word_count();
        Ok(())
    })
}

// ---- dates -----------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_date(
    env: napi_env,
    time: f64,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let date = v8::Date::new(s, time).ok_or(napi_generic_failure)?;
        *out = to_napi(date.into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_date(env: napi_env, v: napi_value, out: *mut bool) -> napi_status {
    basic(env, || {
        let value = val_arg(v)?;
        *out_arg(out)? = value.is_date();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_date_value(
    env: napi_env,
    v: napi_value,
    out: *mut f64,
) -> napi_status {
    js_call(env, |_| {
        let value = val_arg(v)?;
        let out = out_arg(out)?;
        let date: v8::Local<v8::Date> = value.try_into().map_err(|_| napi_date_expected)?;
        *out = date.value_of();
        Ok(())
    })
}

// ---- arrays (length / element access / type test / key enumeration) -------

#[no_mangle]
pub unsafe extern "C" fn napi_is_array(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    is_kind(env, v, out, |v| v.is_array())
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_array_length(
    env: napi_env,
    v: napi_value,
    out: *mut u32,
) -> napi_status {
    js_call(env, |_| {
        let value = val_arg(v)?;
        let out = out_arg(out)?;
        let arr: v8::Local<v8::Array> = value.try_into().map_err(|_| napi_array_expected)?;
        *out = arr.length();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_element(
    env: napi_env,
    obj: napi_value,
    index: u32,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let o = obj_arg(s, obj)?;
        *out = to_napi(o.get_index(s, index).ok_or(V8_GAVE_NOTHING)?);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property_names(
    env: napi_env,
    obj: napi_value,
    out: *mut napi_value,
) -> napi_status {
    // as Node: the for-in keys (prototype chain, enumerable, no symbols), indices as strings
    napi_get_all_property_names(env, obj, 0, (1 << 1) | (1 << 4), 1, out)
}

// ---- wrap / unwrap / remove_wrap ---------------------------------------------

/// The isolate-wide private key under which an object carries its wrap (as Node's
/// `napi_wrapper` private symbol: every addon of an isolate sees the same wraps).
fn wrap_key<'s>(s: &v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(s, "turbo-test::napi_wrap");
    v8::Private::for_api(s, name)
}

/// The object a wrap call targets: as Node, NULL or a non-object is napi_invalid_arg.
unsafe fn wrap_target(v: napi_value) -> Result<v8::Local<'static, v8::Object>, napi_status> {
    if v.is_null() {
        return Err(napi_invalid_arg);
    }
    to_local(v).try_into().map_err(|_| napi_invalid_arg)
}

#[no_mangle]
pub unsafe extern "C" fn napi_wrap(
    env: napi_env,
    js_object: napi_value,
    native_object: *mut c_void,
    finalize_cb: Option<napi_finalize>,
    finalize_hint: *mut c_void,
    result: *mut napi_ref,
) -> napi_status {
    js_call(env, |s| {
        let obj = wrap_target(js_object)?;
        let key = wrap_key(s);
        // as Node: an object carries one wrap; a result reference needs a finalizer (it is to
        // be deleted from the finalizer)
        if obj.has_private(s, key) == Some(true) || (!result.is_null() && finalize_cb.is_none()) {
            return Err(napi_invalid_arg);
        }
        let wrap = track_finalizer(
            s,
            env,
            obj.into(),
            native_object,
            finalize_cb,
            finalize_hint,
        );
        obj.set_private(s, key, v8::External::new(s, wrap as *mut c_void).into());
        if !result.is_null() {
            *result = weak_ref(s, env, obj);
            (*wrap).own_ref = *result as usize;
        }
        Ok(())
    })
}

/// A new finalizer record for `obj`, held by the env until the object is collected (or the env
/// is torn down).
unsafe fn track_finalizer(
    s: &mut v8::PinScope<'static, 'static>,
    env: napi_env,
    obj: v8::Local<v8::Value>,
    native: *mut c_void,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
) -> *mut Finalizer {
    let fin = new_finalizer(env, native, finalize, hint);
    let on_gc = Box::new(move |isolate: &mut v8::Isolate| finalizer_collected(isolate, fin));
    (*fin).weak = Some(v8::Weak::with_finalizer(s, obj, on_gc));
    fin
}

/// A new finalizer record held by the env (run at env teardown unless removed first).
unsafe fn new_finalizer(
    env: napi_env,
    native: *mut c_void,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
) -> *mut Finalizer {
    let e = &mut *env;
    let id = e.next_finalizer;
    e.next_finalizer += 1;
    let fin = Box::into_raw(Box::new(Finalizer {
        env,
        id,
        native,
        finalize,
        hint,
        weak: None,
        own_ref: 0,
    }));
    e.finalizers.insert(id, fin);
    fin
}

/// A new weak reference to `obj` (the optional result of napi_wrap / napi_add_finalizer).
unsafe fn weak_ref(
    s: &mut v8::PinScope<'static, 'static>,
    env: napi_env,
    obj: v8::Local<v8::Object>,
) -> napi_ref {
    new_ref(&mut *env, Ref::new(s, obj.into(), 0))
}

/// GC finalizer of an object with a finalizer record: the record leaves its env and its
/// finalizer runs in a fresh handle scope of the env's context (V8 calls this after the
/// collection, where napi calls are allowed).
fn finalizer_collected(isolate: &mut v8::Isolate, fin: *mut Finalizer) {
    unsafe {
        let fin = Box::from_raw(fin);
        let env = fin.env;
        (*env).finalizers.remove(&fin.id);
        v8::scope!(let hs, isolate);
        let context = v8::Local::new(hs, &(*env).context);
        let cs: &mut v8::PinScope = &mut v8::ContextScope::new(hs, context);
        let here = cs as *mut v8::PinScope as *mut c_void;
        let outer = std::mem::replace(&mut (*env).scope, here);
        run_finalizer(env, &fin);
        (*env).scope = outer;
    }
}

/// napi_unwrap (`remove` false) and napi_remove_wrap (`remove` true), as Node's v8impl::Unwrap:
/// the wrapped pointer, and for remove_wrap the wrap is dropped without running its finalizer.
unsafe fn unwrap(
    env: napi_env,
    js_object: napi_value,
    result: *mut *mut c_void,
    remove: bool,
) -> napi_status {
    js_call(env, |s| {
        // napi_unwrap needs `result`; napi_remove_wrap may discard the pointer
        if !remove && result.is_null() {
            return Err(napi_invalid_arg);
        }
        let obj = wrap_target(js_object)?;
        let key = wrap_key(s);
        let held = obj.get_private(s, key).ok_or(napi_pending_exception)?;
        let ext: v8::Local<v8::External> = held.try_into().map_err(|_| napi_invalid_arg)?;
        let wrap = ext.value() as *mut Finalizer;
        if !result.is_null() {
            *result = (*wrap).native;
        }
        if remove {
            obj.delete_private(s, key);
            (*(*wrap).env).finalizers.remove(&(*wrap).id);
            drop(Box::from_raw(wrap));
        }
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_unwrap(
    env: napi_env,
    js_object: napi_value,
    result: *mut *mut c_void,
) -> napi_status {
    unwrap(env, js_object, result, false)
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_wrap(
    env: napi_env,
    js_object: napi_value,
    result: *mut *mut c_void,
) -> napi_status {
    unwrap(env, js_object, result, true)
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_finalizer(
    env: napi_env,
    js_object: napi_value,
    finalize_data: *mut c_void,
    finalize_cb: Option<napi_finalize>,
    finalize_hint: *mut c_void,
    result: *mut napi_ref,
) -> napi_status {
    checked(env, || {
        // as Node: no NAPI_PREAMBLE (it runs no JS, also with an exception pending); the object and
        // the finalizer are required, and the object must be one (napi_invalid_arg)
        let Some(finalize_cb) = finalize_cb else {
            return napi_invalid_arg;
        };
        let Ok(obj) = wrap_target(js_object) else {
            return napi_invalid_arg;
        };
        let s = scope(env);
        let fin = track_finalizer(
            s,
            env,
            obj.into(),
            finalize_data,
            Some(finalize_cb),
            finalize_hint,
        );
        if !result.is_null() {
            *result = weak_ref(s, env, obj);
            (*fin).own_ref = *result as usize;
        }
        napi_ok
    })
}

// ---- type tags ------------------------------------------------------------------

/// A 128-bit type tag (napi_type_tag).
#[repr(C)]
pub struct napi_type_tag {
    pub lower: u64,
    pub upper: u64,
}

/// The isolate-wide private key under which an object carries its type tag (Node's
/// `napi_type_tag` private symbol: one tag per object, whatever its value).
fn type_tag_key<'s>(s: &v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(s, "turbo-test::napi_type_tag");
    v8::Private::for_api(s, name)
}

/// The object a type-tag call targets, as Node's CHECK_TO_OBJECT: NULL is napi_invalid_arg,
/// a primitive is converted with ToObject (null / undefined throw: napi_object_expected).
unsafe fn tag_target(
    s: &mut v8::PinScope<'static, 'static>,
    v: napi_value,
) -> Result<v8::Local<'static, v8::Object>, napi_status> {
    if v.is_null() {
        return Err(napi_invalid_arg);
    }
    to_object(s, v)
}

#[no_mangle]
pub unsafe extern "C" fn napi_type_tag_object(
    env: napi_env,
    object: napi_value,
    type_tag: *const napi_type_tag,
) -> napi_status {
    js_call(env, |s| {
        // as Node: an External carries its tag in its wrapper (it holds no properties)
        if let Some(wrapper) = external_wrapper(object) {
            let tag = type_tag.as_ref().ok_or(napi_invalid_arg)?;
            if wrapper.tag.is_some() {
                return Err(napi_invalid_arg);
            }
            wrapper.tag = Some((tag.lower, tag.upper));
            return Ok(());
        }
        let obj = tag_target(s, object)?;
        if type_tag.is_null() {
            return Err(napi_invalid_arg);
        }
        // as Node: the tag is a BigInt of the two words, under one private key: an object that
        // already carries a tag is napi_invalid_arg
        let key = type_tag_key(s);
        if obj.has_private(s, key) != Some(false) {
            return Err(napi_invalid_arg);
        }
        let tag = &*type_tag;
        let value = v8::BigInt::new_from_words(s, false, &[tag.lower, tag.upper])
            .ok_or(napi_generic_failure)?;
        obj.set_private(s, key, value.into())
            .filter(|&set| set)
            .ok_or(napi_generic_failure)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_check_object_type_tag(
    env: napi_env,
    object: napi_value,
    type_tag: *const napi_type_tag,
    result: *mut bool,
) -> napi_status {
    js_call(env, |s| {
        if let Some(wrapper) = external_wrapper(object) {
            let tag = type_tag.as_ref().ok_or(napi_invalid_arg)?;
            *out_arg(result)? = wrapper.tag == Some((tag.lower, tag.upper));
            return Ok(());
        }
        let obj = tag_target(s, object)?;
        if type_tag.is_null() || result.is_null() {
            return Err(napi_invalid_arg);
        }
        let key = type_tag_key(s);
        let held = obj.get_private(s, key).ok_or(napi_generic_failure)?;
        let tag = &*type_tag;
        *result = v8::Local::<v8::BigInt>::try_from(held).is_ok_and(|held| {
            // as Node: a non-negative BigInt of up to two words, missing high words being 0
            let mut words = [0u64; 2];
            let (negative, _) = held.to_words_array(&mut words);
            !negative && words == [tag.lower, tag.upper]
        });
        Ok(())
    })
}

// ---- externals -------------------------------------------------------------------

/// What a napi_create_external External points at, as Node's ExternalWrapper: the addon's
/// data and the type tag an External can carry (it holds no properties); freed with the
/// External, by the finalizer of its weak handle.
struct ExternalWrapper {
    data: *mut c_void,
    tag: Option<(u64, u64)>,
    weak: Option<v8::Weak<v8::External>>,
}

/// The wrapper of a napi_create_external External, None for anything else (or NULL).
unsafe fn external_wrapper<'a>(v: napi_value) -> Option<&'a mut ExternalWrapper> {
    if v.is_null() {
        return None;
    }
    let ext = v8::Local::<v8::External>::try_from(to_local(v)).ok()?;
    Some(&mut *(ext.value() as *mut ExternalWrapper))
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external(
    env: napi_env,
    data: *mut c_void,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let out = out_arg(out)?;
        let wrapper = Box::into_raw(Box::new(ExternalWrapper {
            data,
            tag: None,
            weak: None,
        }));
        let ext = v8::External::new(s, wrapper.cast());
        let free = Box::new(move |_: &mut v8::Isolate| drop(Box::from_raw(wrapper)));
        (*wrapper).weak = Some(v8::Weak::with_finalizer(s, ext, free));
        // as Node: the finalizer runs once the External is collected (or at env teardown)
        if finalize.is_some() {
            track_finalizer(s, env, ext.into(), data, finalize, hint);
        }
        *out = to_napi(ext.into());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_external(
    env: napi_env,
    v: napi_value,
    out: *mut *mut c_void,
) -> napi_status {
    basic(env, || {
        val_arg(v)?;
        let out = out_arg(out)?;
        *out = external_wrapper(v).ok_or(napi_invalid_arg)?.data;
        Ok(())
    })
}

// ---- promises --------------------------------------------------------------------

/// A napi_deferred: the resolver of its promise, until it is resolved or rejected.
type Deferred = v8::Global<v8::PromiseResolver>;

#[no_mangle]
pub unsafe extern "C" fn napi_create_promise(
    env: napi_env,
    deferred: *mut *mut c_void,
    promise: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let (deferred, promise) = (out_arg(deferred)?, out_arg(promise)?);
        let resolver = v8::PromiseResolver::new(s).ok_or(napi_generic_failure)?;
        *promise = to_napi(resolver.get_promise(s).into());
        *deferred = Box::into_raw(Box::new(Deferred::new(s, resolver))).cast();
        Ok(())
    })
}

/// napi_resolve_deferred / napi_reject_deferred, as Node's ConcludeDeferred: the deferred is
/// used up (with an exception pending it is left alone, the call napi_pending_exception).
unsafe fn conclude_deferred(
    env: napi_env,
    deferred: *mut c_void,
    value: napi_value,
    resolve: bool,
) -> napi_status {
    js_call(env, |s| {
        let value = val_arg(value)?;
        // (a NULL deferred: Node would crash)
        if deferred.is_null() {
            return Err(napi_invalid_arg);
        }
        let resolver = Box::from_raw(deferred as *mut Deferred);
        let resolver = v8::Local::new(s, &*resolver);
        let done = if resolve {
            resolver.resolve(s, value)
        } else {
            resolver.reject(s, value)
        };
        done.filter(|&ok| ok)
            .map(|_| ())
            .ok_or(napi_generic_failure)
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_resolve_deferred(
    env: napi_env,
    deferred: *mut c_void,
    value: napi_value,
) -> napi_status {
    conclude_deferred(env, deferred, value, true)
}

#[no_mangle]
pub unsafe extern "C" fn napi_reject_deferred(
    env: napi_env,
    deferred: *mut c_void,
    value: napi_value,
) -> napi_status {
    conclude_deferred(env, deferred, value, false)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_promise(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    is_kind(env, v, out, |v| v.is_promise())
}

#[no_mangle]
pub unsafe extern "C" fn napi_new_instance(
    env: napi_env,
    constructor: napi_value,
    argc: usize,
    argv: *const napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let ctor = val_arg(constructor)?;
        if argc > 0 && argv.is_null() {
            return Err(napi_invalid_arg);
        }
        let out = out_arg(out)?;
        let ctor: v8::Local<v8::Function> = ctor.try_into().map_err(|_| napi_invalid_arg)?;
        let args: Vec<v8::Local<v8::Value>> = (0..argc).map(|i| to_local(*argv.add(i))).collect();
        *out = to_napi(
            ctor.new_instance(s, &args)
                .ok_or(napi_pending_exception)?
                .into(),
        );
        Ok(())
    })
}

// ---- errors / exceptions --------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_throw(env: napi_env, error: napi_value) -> napi_status {
    js_call(env, |s| {
        s.throw_exception(val_arg(error)?);
        Ok(())
    })
}

/// The kind of error napi_throw_*error / napi_create_*error make.
#[derive(Clone, Copy)]
enum ErrorKind {
    Error,
    Type,
    Range,
    Syntax,
}

/// A new error of `kind` with message `msg` (v8::Exception::Error, TypeError, ...).
fn new_error<'s>(
    s: &mut v8::PinScope<'s, 'static>,
    kind: ErrorKind,
    msg: v8::Local<'s, v8::String>,
) -> v8::Local<'s, v8::Value> {
    match kind {
        ErrorKind::Error => v8::Exception::error(s, msg),
        ErrorKind::Type => v8::Exception::type_error(s, msg),
        ErrorKind::Range => v8::Exception::range_error(s, msg),
        ErrorKind::Syntax => v8::Exception::syntax_error(s, msg),
    }
}

/// An internalized string from a NUL-terminated UTF-8 argument, as Node's CHECK_NEW_FROM_UTF8:
/// NULL is napi_invalid_arg, a string V8 cannot make napi_generic_failure.
unsafe fn cstr_arg<'s>(
    s: &mut v8::PinScope<'s, 'static>,
    p: *const c_char,
) -> Result<v8::Local<'s, v8::String>, napi_status> {
    if p.is_null() {
        return Err(napi_invalid_arg);
    }
    let bytes = std::ffi::CStr::from_ptr(p).to_bytes();
    v8::String::new_from_utf8(s, bytes, v8::NewStringType::Internalized).ok_or(napi_generic_failure)
}

/// napi_throw_error / napi_throw_type_error / napi_throw_range_error /
/// node_api_throw_syntax_error, as Node: a new error of `kind` with the message (and the
/// optional `code`), thrown.
unsafe fn throw_new_error(
    env: napi_env,
    kind: ErrorKind,
    code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    js_call(env, |s| {
        let ms = cstr_arg(s, msg)?;
        let exc = new_error(s, kind, ms);
        if !code.is_null() {
            let code = cstr_arg(s, code)?;
            set_error_code(s, exc, code)?;
        }
        s.throw_exception(exc);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw_error(
    env: napi_env,
    code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    throw_new_error(env, ErrorKind::Error, code, msg)
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_type_error(
    env: napi_env,
    code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    throw_new_error(env, ErrorKind::Type, code, msg)
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_range_error(
    env: napi_env,
    code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    throw_new_error(env, ErrorKind::Range, code, msg)
}
#[no_mangle]
pub unsafe extern "C" fn node_api_throw_syntax_error(
    env: napi_env,
    code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    throw_new_error(env, ErrorKind::Syntax, code, msg)
}

/// napi_create_error / napi_create_type_error / napi_create_range_error /
/// node_api_create_syntax_error, as Node: `msg` (and a non-NULL `code`) must be strings
/// (napi_string_expected), `msg` and `out` are required (napi_invalid_arg).
unsafe fn create_new_error(
    env: napi_env,
    kind: ErrorKind,
    code: napi_value,
    msg: napi_value,
    out: *mut napi_value,
) -> napi_status {
    // No NAPI_PREAMBLE in Node, but setting `code` can run JS (a setter): park what it throws.
    catching(env, |s| {
        if msg.is_null() || out.is_null() {
            return Err(napi_invalid_arg);
        }
        let ms: v8::Local<v8::String> =
            to_local(msg).try_into().map_err(|_| napi_string_expected)?;
        let exc = new_error(s, kind, ms);
        if !code.is_null() {
            let code: v8::Local<v8::String> = to_local(code)
                .try_into()
                .map_err(|_| napi_string_expected)?;
            set_error_code(s, exc, code)?;
        }
        put(out, exc);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_error(
    env: napi_env,
    code: napi_value,
    msg: napi_value,
    out: *mut napi_value,
) -> napi_status {
    create_new_error(env, ErrorKind::Error, code, msg, out)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_type_error(
    env: napi_env,
    code: napi_value,
    msg: napi_value,
    out: *mut napi_value,
) -> napi_status {
    create_new_error(env, ErrorKind::Type, code, msg, out)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_range_error(
    env: napi_env,
    code: napi_value,
    msg: napi_value,
    out: *mut napi_value,
) -> napi_status {
    create_new_error(env, ErrorKind::Range, code, msg, out)
}
#[no_mangle]
pub unsafe extern "C" fn node_api_create_syntax_error(
    env: napi_env,
    code: napi_value,
    msg: napi_value,
    out: *mut napi_value,
) -> napi_status {
    create_new_error(env, ErrorKind::Syntax, code, msg, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_last_error_info(
    env: napi_env,
    result: *mut *const napi_extended_error_info,
) -> napi_status {
    // as Node: the status of the last call (not this one), its message filled in on request
    if env.is_null() {
        return napi_invalid_arg;
    }
    if result.is_null() {
        return record(env, napi_invalid_arg);
    }
    let e = &mut (*env).last_error;
    e.error_message = if e.error_code == napi_ok {
        std::ptr::null()
    } else {
        ERROR_MESSAGES[e.error_code as usize].as_ptr()
    };
    *result = e;
    napi_ok
}

/// Hand `err` to the runtime's uncaught-exception report (what an exception thrown by a timer
/// callback gets), as Node's TriggerUncaughtException.
fn report_uncaught(s: &mut v8::PinScope, err: v8::Local<v8::Value>) {
    let global = s.get_current_context().global(s);
    let key = v8::String::new(s, "__reportUncaught").unwrap();
    if let Some(Ok(report)) = global
        .get(s, key.into())
        .map(v8::Local::<v8::Function>::try_from)
    {
        let recv = v8::undefined(s).into();
        report.call(s, recv, &[err]);
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_fatal_exception(env: napi_env, err: napi_value) -> napi_status {
    js_call(env, |s| {
        if err.is_null() {
            return Err(napi_invalid_arg);
        }
        report_uncaught(s, to_local(err));
        Ok(())
    })
}

/// The line napi_fatal_error prints, as Node's OnFatalError: `FATAL ERROR: <location>
/// <message>`, each a `(const char*, size_t)` argument (NAPI_AUTO_LENGTH: NUL-terminated).
unsafe fn fatal_error_line(
    location: *const c_char,
    location_len: usize,
    message: *const c_char,
    message_len: usize,
) -> String {
    let location = String::from_utf8_lossy(utf8_arg(location, location_len));
    let message = String::from_utf8_lossy(utf8_arg(message, message_len));
    format!("FATAL ERROR: {location} {message}")
}

/// Write the coverage counters of a `cargo tarpaulin` build (LLVM's profile runtime, linked into
/// instrumented binaries) before the process aborts, which skips the exit hook that normally
/// writes them; nothing otherwise.
#[cfg(tarpaulin)]
fn flush_coverage() {
    extern "C" {
        fn __llvm_profile_write_file() -> i32;
    }
    unsafe { __llvm_profile_write_file() };
}
#[cfg(not(tarpaulin))]
fn flush_coverage() {}

#[no_mangle]
pub unsafe extern "C" fn napi_fatal_error(
    location: *const c_char,
    location_len: usize,
    message: *const c_char,
    message_len: usize,
) -> ! {
    // as Node: print the error and abort the process
    eprintln!(
        "{}",
        fatal_error_line(location, location_len, message, message_len)
    );
    flush_coverage();
    std::process::abort()
}

/// `err.code = code`, as Node's set_error_code for napi_throw_error / napi_create_error: a
/// failed (throwing) Set is napi_generic_failure.
unsafe fn set_error_code(
    s: &mut v8::PinScope<'static, 'static>,
    err: v8::Local<v8::Value>,
    code: v8::Local<v8::String>,
) -> NapiResult {
    let err: v8::Local<v8::Object> = err.try_into().map_err(|_| napi_generic_failure)?;
    let key = v8::String::new(s, "code").ok_or(napi_generic_failure)?;
    err.set(s, key.into(), code.into())
        .ok_or(napi_generic_failure)?;
    Ok(())
}
#[no_mangle]
pub unsafe extern "C" fn napi_is_error(
    env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    is_kind(env, v, out, |v| v.is_native_error())
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_exception_pending(env: napi_env, out: *mut bool) -> napi_status {
    basic(env, || {
        *out_arg(out)? = (*env).last_exception.is_some();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_and_clear_last_exception(
    env: napi_env,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        let out = out_arg(out)?;
        let s = scope(env);
        *out = match (*env).last_exception.take() {
            Some(g) => to_napi(v8::Local::new(s, &g)),
            None => to_napi(v8::undefined(s).into()),
        };
        Ok(())
    })
}

// ---- references ------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_reference(
    env: napi_env,
    v: napi_value,
    initial_refcount: u32,
    out: *mut napi_ref,
) -> napi_status {
    basic(env, || {
        let s = scope(env);
        let value = val_arg(v)?;
        let out = out_arg(out)?;
        // as Node: before module API version 10 only objects, functions and symbols can be
        // referenced (from 10 any value; one that cannot be held weakly goes at count 0)
        if (*env).module_api_version < 10 && !(value.is_object() || value.is_symbol()) {
            return Err(napi_invalid_arg);
        }
        *out = new_ref(&mut *env, Ref::new(s, value, initial_refcount));
        Ok(())
    })
}

/// Store `r` on the env: its napi_ref is its 1-based id.
fn new_ref(e: &mut Env, r: Ref) -> napi_ref {
    e.refs.push(Some(r));
    e.refs.len() as napi_ref
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_reference_value(
    env: napi_env,
    r: napi_ref,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        let s = scope(env);
        let e = &mut *env;
        let id = r as usize;
        if id == 0 || id > e.refs.len() {
            return Err(napi_invalid_arg);
        }
        let out = out_arg(out)?;
        let value = match e.refs[id - 1].as_ref() {
            // as Node: a weak reference whose value was collected (or finalized) yields NULL
            Some(r) => r.get(s),
            None => Some(v8::undefined(s).into()),
        };
        *out = value.map_or(std::ptr::null_mut(), |v| to_napi(v));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_reference(env: napi_env, r: napi_ref) -> napi_status {
    checked(env, || {
        // as Node (CHECK_ARG(env, ref)): a NULL reference is napi_invalid_arg
        if r.is_null() {
            return napi_invalid_arg;
        }
        let e = &mut *env;
        let id = r as usize;
        if id <= e.refs.len() {
            e.refs[id - 1] = None;
        }
        napi_ok
    })
}
/// The live reference `r` of the env, as napi_reference_ref / napi_reference_unref take it: a
/// NULL, unknown or deleted reference is napi_invalid_arg (Node: CHECK_ARG, then undefined
/// behaviour).
unsafe fn live_ref<'a>(env: napi_env, r: napi_ref) -> Result<&'a mut Ref, napi_status> {
    let id = r as usize;
    let refs = &mut (*env).refs;
    refs.get_mut(id.wrapping_sub(1))
        .and_then(|slot| slot.as_mut())
        .ok_or(napi_invalid_arg)
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_ref(
    env: napi_env,
    r: napi_ref,
    result: *mut u32,
) -> napi_status {
    checked(env, || {
        let s = scope(env);
        match live_ref(env, r) {
            Ok(r) => {
                let count = r.inc(s);
                if !result.is_null() {
                    *result = count;
                }
                napi_ok
            }
            Err(status) => status,
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_unref(
    env: napi_env,
    r: napi_ref,
    result: *mut u32,
) -> napi_status {
    checked(env, || {
        let s = scope(env);
        match live_ref(env, r) {
            // as Node: unref of a count of 0 is napi_generic_failure
            Ok(r) if r.count == 0 => napi_generic_failure,
            Ok(r) => {
                let count = r.dec(s);
                if !result.is_null() {
                    *result = count;
                }
                napi_ok
            }
            Err(status) => status,
        }
    })
}

// ---- stubs (threadsafe fns / cleanup hooks — not exercised by sync parse) ---

// ---- instance data, cleanup hooks, scopes, scripts ---------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_set_instance_data(
    env: napi_env,
    data: *mut c_void,
    finalize: Option<napi_finalize>,
    hint: *mut c_void,
) -> napi_status {
    basic(env, || {
        // as Node: the previous data is dropped without its finalizer
        if let Some(old) = (*env).instance_data.take() {
            drop(Box::from_raw((*env).finalizers.remove(&old).unwrap()));
        }
        // the finalizer runs at env teardown, with the other finalizers
        (*env).instance_data = Some((*new_finalizer(env, data, finalize, hint)).id);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_instance_data(
    env: napi_env,
    data: *mut *mut c_void,
) -> napi_status {
    basic(env, || {
        let data = out_arg(data)?;
        let e = &*env;
        *data = e
            .instance_data
            .and_then(|id| e.finalizers.get(&id))
            .map_or(std::ptr::null_mut(), |&fin| (*fin).native);
        Ok(())
    })
}

/// The cleanup hook argument of napi_add_env_cleanup_hook / napi_remove_env_cleanup_hook.
type CleanupHook = unsafe extern "C" fn(*mut c_void);

#[no_mangle]
pub unsafe extern "C" fn napi_add_env_cleanup_hook(
    env: napi_env,
    fun: Option<CleanupHook>,
    arg: *mut c_void,
) -> napi_status {
    // as Node: the status is not recorded on success (no napi_clear_last_error)
    if env.is_null() {
        return napi_invalid_arg;
    }
    let Some(fun) = fun else {
        return record(env, napi_invalid_arg);
    };
    let mut teardown = teardown_of(env).borrow_mut();
    // (the same fun and arg twice: Node aborts the process)
    if teardown
        .iter()
        .any(|t| matches!(*t, Teardown::Hook(_, f, a) if std::ptr::fn_addr_eq(f, fun) && a == arg))
    {
        return record(env, napi_invalid_arg);
    }
    teardown.push(Teardown::Hook(env, fun, arg));
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_env_cleanup_hook(
    env: napi_env,
    fun: Option<CleanupHook>,
    arg: *mut c_void,
) -> napi_status {
    if env.is_null() {
        return napi_invalid_arg;
    }
    let Some(fun) = fun else {
        return record(env, napi_invalid_arg);
    };
    // as Node: removing a hook that is not registered does nothing
    let mut teardown = teardown_of(env).borrow_mut();
    teardown.retain(
        |t| !matches!(*t, Teardown::Hook(_, f, a) if std::ptr::fn_addr_eq(f, fun) && a == arg),
    );
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_async_cleanup_hook(
    env: napi_env,
    hook: Option<unsafe extern "C" fn(*mut AsyncCleanupHook, *mut c_void)>,
    arg: *mut c_void,
    remove_handle: *mut *mut AsyncCleanupHook,
) -> napi_status {
    basic(env, || {
        let hook = hook.ok_or(napi_invalid_arg)?;
        let h = Box::into_raw(Box::new(AsyncCleanupHook { env, hook, arg }));
        teardown_of(env).borrow_mut().push(Teardown::AsyncHook(h));
        if !remove_handle.is_null() {
            *remove_handle = h;
        }
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_async_cleanup_hook(
    remove_handle: *mut AsyncCleanupHook,
) -> napi_status {
    if remove_handle.is_null() {
        return napi_invalid_arg;
    }
    // before teardown the hook is dropped; from (or after) the hook, its handle is freed
    teardown_of((*remove_handle).env)
        .borrow_mut()
        .retain(|t| !matches!(*t, Teardown::AsyncHook(h) if h == remove_handle));
    drop(Box::from_raw(remove_handle));
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_adjust_external_memory(
    env: napi_env,
    change_in_bytes: i64,
    adjusted_value: *mut i64,
) -> napi_status {
    basic(env, || {
        let out = out_arg(adjusted_value)?;
        let isolate = &mut v8::Isolate::from_raw_isolate_ptr((*env).isolate);
        *out = isolate.adjust_amount_of_external_allocated_memory(change_in_bytes);
        Ok(())
    })
}

/// A napi_handle_scope / napi_escapable_handle_scope: whether it has escaped a value. Values
/// are not freed when a scope closes (they live until the native call returns); the scopes are
/// counted, and an escapable one escapes once, as Node's.
pub struct HandleScopeRecord {
    escaped: bool,
}

unsafe fn open_scope(env: napi_env, out: *mut *mut HandleScopeRecord) -> napi_status {
    basic(env, || {
        *out_arg(out)? = Box::into_raw(Box::new(HandleScopeRecord { escaped: false }));
        (*env).open_handle_scopes += 1;
        Ok(())
    })
}

unsafe fn close_scope(env: napi_env, scope: *mut HandleScopeRecord) -> napi_status {
    if env.is_null() {
        return napi_invalid_arg;
    }
    if scope.is_null() {
        return record(env, napi_invalid_arg);
    }
    // as Node: more closes than opens is napi_handle_scope_mismatch (not recorded)
    if (*env).open_handle_scopes == 0 {
        return napi_handle_scope_mismatch;
    }
    (*env).open_handle_scopes -= 1;
    drop(Box::from_raw(scope));
    record(env, napi_ok)
}

#[no_mangle]
pub unsafe extern "C" fn napi_open_handle_scope(
    env: napi_env,
    out: *mut *mut HandleScopeRecord,
) -> napi_status {
    open_scope(env, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_close_handle_scope(
    env: napi_env,
    scope: *mut HandleScopeRecord,
) -> napi_status {
    close_scope(env, scope)
}

#[no_mangle]
pub unsafe extern "C" fn napi_open_escapable_handle_scope(
    env: napi_env,
    out: *mut *mut HandleScopeRecord,
) -> napi_status {
    open_scope(env, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_close_escapable_handle_scope(
    env: napi_env,
    scope: *mut HandleScopeRecord,
) -> napi_status {
    close_scope(env, scope)
}

#[no_mangle]
pub unsafe extern "C" fn napi_escape_handle(
    env: napi_env,
    scope: *mut HandleScopeRecord,
    escapee: napi_value,
    out: *mut napi_value,
) -> napi_status {
    basic(env, || {
        if scope.is_null() {
            return Err(napi_invalid_arg);
        }
        val_arg(escapee)?;
        let out = out_arg(out)?;
        if (*scope).escaped {
            return Err(napi_escape_called_twice);
        }
        (*scope).escaped = true;
        *out = escapee;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_run_script(
    env: napi_env,
    script: napi_value,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        let script = val_arg(script)?;
        let out = out_arg(out)?;
        let source: v8::Local<v8::String> = script.try_into().map_err(|_| napi_string_expected)?;
        // as Node's CHECK_MAYBE_EMPTY: a script that fails to compile or throws is
        // napi_generic_failure (its exception pending)
        let compiled = v8::Script::compile(s, source, None).ok_or(napi_generic_failure)?;
        *out = to_napi(compiled.run(s).ok_or(napi_generic_failure)?);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn node_api_get_module_file_name(
    env: napi_env,
    out: *mut *const c_char,
) -> napi_status {
    basic(env, || {
        *out_arg(out)? = (*env).filename.as_ptr();
        Ok(())
    })
}

/// A path as a file: URL, as Node's url::FromFilePath: `%` and what the URL path
/// percent-encode set holds (controls, space, `"#<>?\`{}`, non-ASCII bytes) are escaped.
fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for &b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_graphic() && !b"%\"#<>?`{}".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}
#[no_mangle]
pub unsafe extern "C" fn napi_stub() -> napi_status {
    napi_generic_failure
}

// ---- classes and property descriptors -----------------------------------------

/// The key a descriptor names, as Node's V8NameFromPropertyDescriptor: `utf8name`, else the
/// `name` value, which must be a string or a symbol (napi_name_expected).
unsafe fn property_name(
    s: &mut v8::PinScope<'static, 'static>,
    p: &napi_property_descriptor,
) -> Result<v8::Local<'static, v8::Name>, napi_status> {
    if !p.utf8name.is_null() {
        let bytes = std::ffi::CStr::from_ptr(p.utf8name).to_bytes();
        let name = v8::String::new_from_utf8(s, bytes, v8::NewStringType::Internalized)
            .ok_or(napi_generic_failure)?;
        return Ok(name.into());
    }
    if p.name.is_null() {
        return Err(napi_name_expected);
    }
    to_local(p.name).try_into().map_err(|_| napi_name_expected)
}

/// The template attributes of a prototype member, as Node's V8PropertyAttributesFromDescriptor:
/// a data member without napi_writable is read-only; no napi_enumerable is DontEnum, no
/// napi_configurable DontDelete.
fn template_attributes(p: &napi_property_descriptor) -> v8::PropertyAttribute {
    let mut attr = v8::PropertyAttribute::NONE;
    if p.getter.is_none() && p.setter.is_none() && p.attributes & napi_writable == 0 {
        attr = attr | v8::PropertyAttribute::READ_ONLY;
    }
    if p.attributes & napi_enumerable == 0 {
        attr = attr | v8::PropertyAttribute::DONT_ENUM;
    }
    if p.attributes & napi_configurable == 0 {
        attr = attr | v8::PropertyAttribute::DONT_DELETE;
    }
    attr
}

/// The descriptors behind a `(count, properties)` argument pair: properties are required for a
/// count above 0 (napi_invalid_arg).
unsafe fn descriptors<'a>(
    count: usize,
    properties: *const napi_property_descriptor,
) -> Result<&'a [napi_property_descriptor], napi_status> {
    if count == 0 {
        return Ok(&[]);
    }
    if properties.is_null() {
        return Err(napi_invalid_arg);
    }
    Ok(std::slice::from_raw_parts(properties, count))
}

/// napi_define_class, as Node: a constructor function named `name` whose prototype carries the
/// instance members (methods with the class as their receiver signature, accessors, values) of
/// the descriptors; the napi_static ones are then defined on the constructor like
/// napi_define_properties (the class is handed back even when one of those fails).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn napi_define_class(
    env: napi_env,
    name: *const c_char,
    len: usize,
    ctor: Option<napi_callback>,
    data: *mut c_void,
    property_count: usize,
    properties: *const napi_property_descriptor,
    out: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        if out.is_null() {
            return Err(napi_invalid_arg);
        }
        let ctor = ctor.ok_or(napi_invalid_arg)?;
        let props = descriptors(property_count, properties)?;
        // as Node's CHECK_NEW_FROM_UTF8_LEN: the class name is required
        if name.is_null() {
            return Err(napi_invalid_arg);
        }
        let tpl = callback_template(s, env, ctor, data, None);
        let class_name =
            v8::String::new_from_utf8(s, utf8_arg(name, len), v8::NewStringType::Internalized)
                .ok_or(napi_generic_failure)?;
        tpl.set_class_name(class_name);
        let proto = tpl.prototype_template(s);
        for p in props.iter().filter(|p| p.attributes & napi_static == 0) {
            let key = property_name(s, p)?;
            let attr = template_attributes(p);
            if p.getter.is_some() || p.setter.is_some() {
                let getter = p
                    .getter
                    .map(|cb| callback_template(s, env, cb, p.data, None));
                let setter = p
                    .setter
                    .map(|cb| callback_template(s, env, cb, p.data, None));
                proto.set_accessor_property(key, getter, setter, attr);
            } else if let Some(method) = p.method {
                let sig = v8::Signature::new(s, tpl);
                let t = callback_template(s, env, method, p.data, Some(sig));
                proto.set_with_attr(key, t.into(), attr);
            } else {
                // A template holds only primitives: V8 aborts the process on an object (Node
                // crashes there); refuse it instead.
                if p.value.is_null() || to_local(p.value).is_object() {
                    return Err(napi_invalid_arg);
                }
                proto.set_with_attr(key, to_local(p.value).into(), attr);
            }
        }
        let class = tpl.get_function(s).ok_or(napi_generic_failure)?;
        put(out, class.into());
        let statics: Vec<napi_property_descriptor> = props
            .iter()
            .filter(|p| p.attributes & napi_static != 0)
            .copied()
            .collect();
        define_properties(s, env, class.into(), &statics)
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_define_properties(
    env: napi_env,
    object: napi_value,
    property_count: usize,
    properties: *const napi_property_descriptor,
) -> napi_status {
    js_call(env, |s| {
        let props = descriptors(property_count, properties)?;
        if object.is_null() {
            return Err(napi_invalid_arg);
        }
        let obj = to_object(s, object)?;
        define_properties(s, env, obj, props)
    })
}

unsafe extern "C" {
    /// The binding behind rusty_v8's `PropertyDescriptor::new_from_get_set` (V8's
    /// `PropertyDescriptor(get, set)`), which takes either half as an empty (NULL) handle; the
    /// Rust wrapper only accepts two values.
    fn v8__PropertyDescriptor__CONSTRUCT__Get_Set(
        this: *mut v8::PropertyDescriptor,
        get: *const v8::Value,
        set: *const v8::Value,
    );
}

/// An accessor descriptor with the halves given: a `None` half is absent from the descriptor
/// (V8's empty handle), so DefineProperty leaves that half of an existing accessor alone.
fn accessor_descriptor(
    get: Option<v8::Local<v8::Value>>,
    set: Option<v8::Local<v8::Value>>,
) -> v8::PropertyDescriptor {
    let half = |h: Option<v8::Local<v8::Value>>| h.map_or(std::ptr::null(), |h| &*h as *const _);
    let mut desc = std::mem::MaybeUninit::<v8::PropertyDescriptor>::uninit();
    unsafe {
        v8__PropertyDescriptor__CONSTRUCT__Get_Set(desc.as_mut_ptr(), half(get), half(set));
        desc.assume_init()
    }
}

/// Define `props` on `obj`, as Node's napi_define_properties: an accessor pair or a method
/// becomes a new function per property (no signature, no name); a data property with all three
/// attributes takes the CreateDataProperty fast path. A definition V8 refuses is
/// napi_invalid_arg (napi_generic_failure for a method).
unsafe fn define_properties(
    s: &mut v8::PinScope<'static, 'static>,
    env: napi_env,
    obj: v8::Local<'static, v8::Object>,
    props: &[napi_property_descriptor],
) -> NapiResult {
    for p in props {
        let key = property_name(s, p)?;
        let enumerable = p.attributes & napi_enumerable != 0;
        let configurable = p.attributes & napi_configurable != 0;
        let writable = p.attributes & napi_writable != 0;
        let (mut desc, refused) = if p.getter.is_some() || p.setter.is_some() {
            // as Node: a missing half is an empty handle, left out of the definition (an existing
            // accessor keeps that half), not `undefined`
            let mut accessor = |cb: Option<napi_callback>| match cb {
                Some(cb) => new_function(s, env, cb, p.data).map(|f| Some(f.into())),
                None => Ok(None),
            };
            let (get, set) = (accessor(p.getter)?, accessor(p.setter)?);
            (accessor_descriptor(get, set), napi_invalid_arg)
        } else if let Some(method) = p.method {
            let f = new_function(s, env, method, p.data)?;
            let desc = v8::PropertyDescriptor::new_from_value_writable(f.into(), writable);
            (desc, napi_generic_failure)
        } else {
            if p.value.is_null() {
                return Err(napi_invalid_arg);
            }
            let value = to_local(p.value);
            if enumerable && writable && configurable {
                if obj.create_data_property(s, key, value) != Some(true) {
                    return Err(napi_invalid_arg);
                }
                continue;
            }
            let desc = v8::PropertyDescriptor::new_from_value_writable(value, writable);
            (desc, napi_invalid_arg)
        };
        desc.set_enumerable(enumerable);
        desc.set_configurable(configurable);
        if obj.define_property(s, key, &desc) != Some(true) {
            return Err(refused);
        }
    }
    Ok(())
}

// ---- versions --------------------------------------------------------------------

/// The highest Node-API version the host implements (Node's NODE_API_SUPPORTED_VERSION_MAX).
pub const NAPI_VERSION: u32 = 10;

/// NAPI_VERSION_EXPERIMENTAL, the module API version of an addon built with NAPI_EXPERIMENTAL.
const NAPI_VERSION_EXPERIMENTAL: i32 = i32::MAX;

#[no_mangle]
pub unsafe extern "C" fn napi_get_version(env: napi_env, out: *mut u32) -> napi_status {
    basic(env, || {
        *out_arg(out)? = NAPI_VERSION;
        Ok(())
    })
}

/// napi_node_version: the Node.js version the host stands in for.
#[repr(C)]
pub struct napi_node_version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub release: *const c_char,
}

// SAFETY: the version is immutable static data (`release` points at a static string).
unsafe impl Sync for napi_node_version {}

/// The Node.js version reported to addons: the one the runtime's process.version names.
static NODE_VERSION: napi_node_version = napi_node_version {
    major: 24,
    minor: 0,
    patch: 0,
    release: c"node".as_ptr(),
};

#[no_mangle]
pub unsafe extern "C" fn napi_get_node_version(
    env: napi_env,
    out: *mut *const napi_node_version,
) -> napi_status {
    basic(env, || {
        *out_arg(out)? = &NODE_VERSION;
        Ok(())
    })
}

/// The module API version `lib` declares (its node_api_module_get_api_version_v1), as Node's
/// node_napi_env__::New: none or below 8 is 8, above 10 (but NAPI_VERSION_EXPERIMENTAL) an
/// error naming the addon.
unsafe fn module_api_version(lib: &libloading::Library, path: &Path) -> Result<i32, String> {
    type GetVersion = unsafe extern "C" fn() -> i32;
    let Ok(get) = lib.get::<GetVersion>(b"node_api_module_get_api_version_v1") else {
        return Ok(DEFAULT_MODULE_API_VERSION);
    };
    match get() {
        v if v > NAPI_VERSION as i32 && v != NAPI_VERSION_EXPERIMENTAL => Err(format!(
            "{} requires Node-API version {v}, but this version of Node.js only supports version \
             {NAPI_VERSION} add-ons.",
            file_url(path)
        )),
        v => Ok(v.max(DEFAULT_MODULE_API_VERSION)),
    }
}

/// The module init symbol exported by the addon.
type RegisterFn = unsafe extern "C" fn(napi_env, napi_value) -> napi_value;

/// A module an addon registers itself with napi_module_register (the NAPI_MODULE macro, from a
/// static constructor while it is being loaded).
#[repr(C)]
pub struct napi_module {
    nm_version: i32,
    nm_flags: u32,
    nm_filename: *const c_char,
    nm_register_func: Option<RegisterFn>,
    nm_modname: *const c_char,
    nm_priv: *mut c_void,
    reserved: [*mut c_void; 4],
}

thread_local! {
    /// The init a library registered with napi_module_register while this thread loaded it.
    static SELF_REGISTERED: std::cell::Cell<Option<RegisterFn>> = const { std::cell::Cell::new(None) };
}

/// The init of every self-registering library loaded, by its canonical path: a library loaded
/// again (another worker, another path to the same file) does not run its constructors again,
/// as Node's global handle map of saved modules.
static SAVED_MODULES: std::sync::Mutex<Vec<(std::path::PathBuf, RegisterFn)>> =
    std::sync::Mutex::new(Vec::new());

#[no_mangle]
pub unsafe extern "C" fn napi_module_register(module: *mut napi_module) {
    if let Some(module) = module.as_ref() {
        SELF_REGISTERED.with(|r| r.set(module.nm_register_func));
    }
}

/// The init of the library just loaded from `path`: what it registered with
/// napi_module_register (now or when it was first loaded), else its napi_register_module_v1.
unsafe fn module_init(lib: &libloading::Library, path: &Path) -> Result<RegisterFn, String> {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut saved = SAVED_MODULES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(init) = SELF_REGISTERED.with(|r| r.take()) {
        saved.push((key, init));
        return Ok(init);
    }
    if let Some(&(_, init)) = saved.iter().find(|(p, _)| *p == key) {
        return Ok(init);
    }
    let register: libloading::Symbol<RegisterFn> = lib
        .get(b"napi_register_module_v1")
        .map_err(|e| format!("no napi_register_module_v1: {}", loader_message(&e)))?;
    Ok(*register)
}

/// dlopen the addon with every symbol bound now (RTLD_NOW), so an import the process does not
/// provide fails the load instead of binding to NULL and crashing on its first call (macOS
/// binds an unresolved flat-namespace import lazily, to address 0).
#[cfg(unix)]
unsafe fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
    Library::open(Some(path), RTLD_NOW | RTLD_LOCAL).map(Into::into)
}
#[cfg(not(unix))]
unsafe fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    libloading::Library::new(path)
}

/// What the loader said (dlerror), which libloading keeps as the error's source.
fn loader_message(e: &libloading::Error) -> String {
    let source = std::error::Error::source(e);
    source.map_or_else(|| e.to_string(), ToString::to_string)
}

/// The load error for a failed dlopen: an import the process does not provide is named (a
/// Node-API one as missing from the host), anything else is the loader's message.
fn dlopen_error(err: &str) -> String {
    match missing_symbol(err) {
        Some(sym) if sym.starts_with("napi_") || sym.starts_with("node_api_") => format!(
            "it imports {sym}, which turbo-test's Node-API host does not provide (dlopen {err})"
        ),
        Some(sym) => format!("it imports {sym}, which nothing loaded provides (dlopen {err})"),
        None => format!("dlopen {err}"),
    }
}

/// The symbol a dlopen error says is missing, without the Mach-O leading underscore: macOS
/// "symbol not found in flat namespace '_name'", glibc "undefined symbol: name", musl
/// "Error relocating <file>: name: symbol not found".
fn missing_symbol(err: &str) -> Option<&str> {
    if let Some((_, rest)) = err.split_once("symbol not found in flat namespace '") {
        let sym = rest.split('\'').next().unwrap_or(rest);
        return Some(sym.strip_prefix('_').unwrap_or(sym));
    }
    if let Some((_, rest)) = err.split_once("undefined symbol: ") {
        return rest.split([' ', ',', ')']).next();
    }
    let (head, _) = err.split_once(": symbol not found")?;
    head.rsplit(": ").next()
}

/// Load a `.node` addon: dlopen, set up the env, call `napi_register_module_v1` with a
/// fresh exports object, and return that object. Must be called inside a live scope.
pub fn load_addon(
    scope_ref: &mut v8::PinScope,
    path: &Path,
) -> Result<v8::Global<v8::Value>, String> {
    unsafe {
        SELF_REGISTERED.with(|r| r.set(None));
        let lib = open_library(path).map_err(|e| dlopen_error(&loader_message(&e)))?;
        let register = module_init(&lib, path)?;
        let version = module_api_version(&lib, path)?;

        let mut env = Box::new(Env::new(scope_ref));
        env.filename = std::ffi::CString::new(file_url(path)).unwrap_or_default();
        env.module_api_version = version;
        let env_ptr: napi_env = Box::into_raw(env);
        // The isolate owns the env from here on (freed with it, like the functions using it), at
        // its place in the teardown order (Node registers the env's own cleanup hook here).
        teardown_of(env_ptr)
            .borrow_mut()
            .push(Teardown::Env(env_ptr));

        let exports = v8::Object::new(scope_ref);
        // The addon's module init runs arbitrary native code. Guard it two ways:
        //  - A JS exception it leaves pending (napi_throw_*, a throwing JS call) surfaces as a
        //    clean require() failure.
        //  - call_addon turns a Rust panic crossing the `extern "C"` FFI boundary (which is UB
        //    and can abort/segfault, taking the whole run down with no diagnostic) into a
        //    recoverable Err -> a thrown JS error in the loader. The rest of the run survives.
        // call_addon also holds the addon lock, as for every other entry into addon code: two
        // workers loading the same library must not run its init at once (a require() from
        // inside a native callback or another addon's init is a same-thread re-entry).
        let exports_napi = to_napi(exports.into());
        let result = call_addon(|| register(env_ptr, exports_napi));
        let result = init_outcome(result, path)?;
        if let Some(exc) = (*env_ptr).last_exception.take() {
            let msg = v8::Local::new(scope_ref, &exc).to_rust_string_lossy(scope_ref);
            return Err(format!("native addon threw during module init: {msg}"));
        }
        let ret_local = if result.is_null() {
            exports.into()
        } else {
            to_local(result)
        };
        // keep the lib loaded for the process lifetime
        std::mem::forget(lib);
        Ok(v8::Global::new(scope_ref, ret_local))
    }
}

#[path = "napi_host_async.rs"]
mod async_host;
pub use async_host::{keeps_loop_alive, run_native_task, wait_native_task};

#[cfg(all(test, unix))]
#[path = "napi_host_tests.rs"]
mod tests;
