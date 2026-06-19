//! Node-API (napi) host — load native `.node` addons in turbo-test's V8 embedding.
//!
//! Implements the subset of the napi C ABI that real addons call, backed by rusty_v8.
//! This is what Node/Deno/Bun each provide; here it is scoped to the surface turbo-dom's
//! parser addon needs (enumerated from `nm -u` on the .node):
//!   value/object/string/number, arrays, functions + cb_info, arraybuffer/typedarray,
//!   errors/exceptions, references, threadsafe-fn (stubbed).
//!
//! Bridge: `napi_value` IS a `v8::Local<Value>` (Local is `NonNull<T>` — pointer-sized),
//! so they transmute. Handles are created in the scope stored on the env for the current
//! native call (set by the function trampoline / loader before calling into the addon).

#![allow(non_camel_case_types, non_upper_case_globals, dead_code, clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::os::raw::c_char;
use std::path::Path;
use std::ptr::NonNull;

pub type napi_status = i32;
pub const napi_ok: napi_status = 0;
pub const napi_invalid_arg: napi_status = 1;
pub const napi_generic_failure: napi_status = 9;
pub const napi_pending_exception: napi_status = 10;

pub type napi_value = *mut c_void;
pub type napi_env = *mut Env;
pub type napi_ref = *mut c_void;
pub type napi_callback_info = *mut CbInfo;
pub type napi_callback = unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value;
pub type napi_finalize = unsafe extern "C" fn(napi_env, *mut c_void, *mut c_void);

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

/// Per-thread napi environment. `scope` points at the live rusty_v8 scope for the current
/// native call (the only window during which napi value calls happen).
pub struct Env {
    pub isolate: *mut v8::Isolate,
    pub scope: *mut c_void, // *mut v8::PinScope<'static,'static>, valid only during a call
    pub context: Option<v8::Global<v8::Context>>,
    pub last_exception: Option<v8::Global<v8::Value>>,
    /// registered (callback, data) for functions created via napi_create_function
    pub fns: Vec<(napi_callback, *mut c_void)>,
    pub refs: Vec<Option<v8::Global<v8::Value>>>,
}

/// Process-global lock serializing native `.node` addon entry across worker threads (the shared
/// addon's internal state isn't thread-safe). `IN_ADDON` allows same-thread re-entry.
static ADDON_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
thread_local! {
    static IN_ADDON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ENV: std::cell::RefCell<Option<Box<Env>>> = const { std::cell::RefCell::new(None) };
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

/// Callback info passed to a napi_callback during a function call.
pub struct CbInfo {
    pub args: Vec<napi_value>,
    pub this: napi_value,
    pub data: *mut c_void,
}

// ---- value getters --------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_get_undefined(env: napi_env, out: *mut napi_value) -> napi_status {
    let s = scope(env);
    put(out, v8::undefined(s).into());
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_null(env: napi_env, out: *mut napi_value) -> napi_status {
    let s = scope(env);
    put(out, v8::null(s).into());
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_global(env: napi_env, out: *mut napi_value) -> napi_status {
    let s = scope(env);
    let g = s.get_current_context().global(s);
    put(out, g.into());
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_boolean(
    env: napi_env,
    b: bool,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    put(out, v8::Boolean::new(s, b).into());
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_typeof(
    env: napi_env,
    v: napi_value,
    out: *mut napi_valuetype,
) -> napi_status {
    let val = to_local(v);
    let t = if val.is_undefined() {
        napi_valuetype::Undefined
    } else if val.is_null() {
        napi_valuetype::Null
    } else if val.is_boolean() {
        napi_valuetype::Boolean
    } else if val.is_number() {
        napi_valuetype::Number
    } else if val.is_string() {
        napi_valuetype::String
    } else if val.is_function() {
        napi_valuetype::Function
    } else if val.is_object() {
        napi_valuetype::Object
    } else {
        napi_valuetype::Object
    };
    let _ = env;
    if !out.is_null() {
        *out = t;
    }
    napi_ok
}

// ---- numbers / strings ----------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_uint32(
    env: napi_env,
    n: u32,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    put(out, v8::Integer::new_from_unsigned(s, n).into());
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_double(
    env: napi_env,
    n: f64,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    put(out, v8::Number::new(s, n).into());
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_uint32(
    env: napi_env,
    v: napi_value,
    out: *mut u32,
) -> napi_status {
    let s = scope(env);
    let n = to_local(v).number_value(s).unwrap_or(0.0) as u32;
    if !out.is_null() {
        *out = n;
    }
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf8(
    env: napi_env,
    str_: *const c_char,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let bytes = if len == usize::MAX {
        std::ffi::CStr::from_ptr(str_).to_bytes()
    } else {
        std::slice::from_raw_parts(str_ as *const u8, len)
    };
    let text = std::str::from_utf8(bytes).unwrap_or("");
    match v8::String::new(s, text) {
        Some(v) => {
            put(out, v.into());
            napi_ok
        }
        None => napi_generic_failure,
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
    let s = scope(env);
    let str_val = to_local(v).to_rust_string_lossy(s);
    let bytes = str_val.as_bytes();
    if buf.is_null() {
        if !result.is_null() {
            *result = bytes.len();
        }
        return napi_ok;
    }
    let n = bytes.len().min(bufsize.saturating_sub(1));
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
    *(buf.add(n)) = 0;
    if !result.is_null() {
        *result = n;
    }
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_string(
    env: napi_env,
    v: napi_value,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    match to_local(v).to_string(s) {
        Some(str_) => {
            put(out, str_.into());
            napi_ok
        }
        None => napi_generic_failure,
    }
}

// ---- objects / arrays -----------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_object(env: napi_env, out: *mut napi_value) -> napi_status {
    let s = scope(env);
    put(out, v8::Object::new(s).into());
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_array_with_length(
    env: napi_env,
    len: usize,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    put(out, v8::Array::new(s, len as i32).into());
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_named_property(
    env: napi_env,
    obj: napi_value,
    name: *const c_char,
    value: napi_value,
) -> napi_status {
    let s = scope(env);
    let key = std::ffi::CStr::from_ptr(name).to_string_lossy();
    let Some(k) = v8::String::new(s, &key) else {
        return napi_generic_failure;
    };
    let o: v8::Local<v8::Object> = match to_local(obj).try_into() {
        Ok(o) => o,
        Err(_) => return napi_invalid_arg,
    };
    o.set(s, k.into(), to_local(value));
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_named_property(
    env: napi_env,
    obj: napi_value,
    name: *const c_char,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let key = std::ffi::CStr::from_ptr(name).to_string_lossy();
    let Some(k) = v8::String::new(s, &key) else {
        return napi_generic_failure;
    };
    let o: v8::Local<v8::Object> = match to_local(obj).try_into() {
        Ok(o) => o,
        Err(_) => return napi_invalid_arg,
    };
    match o.get(s, k.into()) {
        Some(v) => {
            put(out, v);
            napi_ok
        }
        None => napi_generic_failure,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_element(
    env: napi_env,
    obj: napi_value,
    index: u32,
    value: napi_value,
) -> napi_status {
    let s = scope(env);
    let o: v8::Local<v8::Object> = match to_local(obj).try_into() {
        Ok(o) => o,
        Err(_) => return napi_invalid_arg,
    };
    o.set_index(s, index, to_local(value));
    napi_ok
}

// ---- functions ------------------------------------------------------------

/// Trampoline: every napi function shares this rusty_v8 callback. It recovers the
/// (napi_callback, data) by the index stored as the function's data Integer, packages
/// CbInfo, points the env scope at the current scope, and calls the addon callback.
fn napi_fn_trampoline(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let idx = args
        .data()
        .to_integer(scope)
        .map(|i| i.value() as usize)
        .unwrap_or(usize::MAX);
    ENV.with(|e| {
        let mut b = e.borrow_mut();
        let Some(env) = b.as_mut() else { return };
        let Some(&(cb, data)) = env.fns.get(idx) else { return };
        let argv: Vec<napi_value> =
            (0..args.length()).map(|i| unsafe { to_napi(args.get(i)) }).collect();
        let mut info = CbInfo {
            args: argv,
            this: unsafe { to_napi(args.this().into()) },
            data,
        };
        let env_ptr: napi_env = env.as_mut() as *mut Env;
        (env.as_mut()).scope = scope as *mut v8::PinScope as *mut c_void;
        // The .node addon (turbo-dom parser) is one shared library across all worker threads, so
        // its internal (C/Rust) state is process-global and NOT thread-safe — concurrent calls
        // race (intermittent "document is not defined" / load flakiness). Serialize native addon
        // entry across threads with a global lock; a thread-local guard allows same-thread
        // re-entry (the addon calling a JS callback that re-enters) without self-deadlock.
        let reentrant = IN_ADDON.with(|f| f.get());
        let _guard = if reentrant {
            None
        } else {
            IN_ADDON.with(|f| f.set(true));
            Some(ADDON_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
        };
        // Calling the addon's native callback crosses the FFI boundary. A panic there is UB and
        // can crash the process; catch_unwind converts it into a thrown JS error instead.
        let ret = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            cb(env_ptr, &mut info as *mut CbInfo)
        }));
        if !reentrant {
            IN_ADDON.with(|f| f.set(false));
        }
        match ret {
            Ok(ret) if !ret.is_null() => rv.set(unsafe { to_local(ret) }),
            Ok(_) => {}
            Err(_) => {
                if let Some(msg) = v8::String::new(scope, "turbo-test: native addon callback panicked") {
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                }
            }
        }
    });
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_function(
    env: napi_env,
    _name: *const c_char,
    _len: usize,
    cb: napi_callback,
    data: *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    let idx = {
        let e = &mut *env;
        e.fns.push((cb, data));
        e.fns.len() - 1
    };
    let s = scope(env);
    let data_val = v8::Integer::new(s, idx as i32);
    let tmpl = v8::FunctionTemplate::builder(napi_fn_trampoline)
        .data(data_val.into())
        .build(s);
    let f = tmpl.get_function(s).unwrap();
    put(out, f.into());
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_cb_info(
    _env: napi_env,
    info: napi_callback_info,
    argc: *mut usize,
    argv: *mut napi_value,
    this: *mut napi_value,
    data: *mut *mut c_void,
) -> napi_status {
    let cb = &*info;
    if !argc.is_null() {
        let want = *argc;
        let have = cb.args.len();
        let n = want.min(have);
        if !argv.is_null() {
            for i in 0..n {
                *argv.add(i) = cb.args[i];
            }
            // pad remaining with undefined-ish (null ptr treated as undefined by caller)
            for i in n..want {
                *argv.add(i) = std::ptr::null_mut();
            }
        }
        *argc = have;
    }
    if !this.is_null() {
        *this = cb.this;
    }
    if !data.is_null() {
        *data = cb.data;
    }
    napi_ok
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
    let s = scope(env);
    let f: v8::Local<v8::Function> = match to_local(func).try_into() {
        Ok(f) => f,
        Err(_) => return napi_invalid_arg,
    };
    let args: Vec<v8::Local<v8::Value>> =
        (0..argc).map(|i| to_local(*argv.add(i))).collect();
    let recv_local = if recv.is_null() {
        v8::undefined(s).into()
    } else {
        to_local(recv)
    };
    match f.call(s, recv_local, &args) {
        Some(v) => {
            put(out, v);
            napi_ok
        }
        None => napi_pending_exception,
    }
}

// ---- arraybuffer / typedarray (for the parser's SoA buffer) ----------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_arraybuffer(
    env: napi_env,
    len: usize,
    data: *mut *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let ab = v8::ArrayBuffer::new(s, len);
    if !data.is_null() {
        let store = ab.get_backing_store();
        *data = store.data().map(|p| p.as_ptr()).unwrap_or(std::ptr::null_mut());
    }
    put(out, ab.into());
    napi_ok
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
    let s = scope(env);
    let ab: v8::Local<v8::ArrayBuffer> = match to_local(arraybuffer).try_into() {
        Ok(a) => a,
        Err(_) => return napi_invalid_arg,
    };
    // napi typedarray type codes -> matching v8 typed array. `length` is in ELEMENTS.
    let ta: v8::Local<v8::Value> = match type_ {
        0 => v8::Int8Array::new(s, ab, byte_offset, length).unwrap().into(),
        1 => v8::Uint8Array::new(s, ab, byte_offset, length).unwrap().into(),
        2 => v8::Uint8ClampedArray::new(s, ab, byte_offset, length).unwrap().into(),
        3 => v8::Int16Array::new(s, ab, byte_offset, length).unwrap().into(),
        4 => v8::Uint16Array::new(s, ab, byte_offset, length).unwrap().into(),
        5 => v8::Int32Array::new(s, ab, byte_offset, length).unwrap().into(),
        6 => v8::Uint32Array::new(s, ab, byte_offset, length).unwrap().into(),
        7 => v8::Float32Array::new(s, ab, byte_offset, length).unwrap().into(),
        8 => v8::Float64Array::new(s, ab, byte_offset, length).unwrap().into(),
        9 => v8::BigInt64Array::new(s, ab, byte_offset, length).unwrap().into(),
        10 => v8::BigUint64Array::new(s, ab, byte_offset, length).unwrap().into(),
        _ => v8::Uint8Array::new(s, ab, byte_offset, length).unwrap().into(),
    };
    put(out, ta);
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external_arraybuffer(
    env: napi_env,
    data: *mut c_void,
    len: usize,
    _finalize: Option<napi_finalize>,
    _hint: *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    // copy into a managed ArrayBuffer (we don't track external lifetime here)
    let s = scope(env);
    let ab = v8::ArrayBuffer::new(s, len);
    if let Some(store) = ab.get_backing_store().data() {
        std::ptr::copy_nonoverlapping(data as *const u8, store.as_ptr() as *mut u8, len);
    }
    put(out, ab.into());
    napi_ok
}

// ---- buffers (Node `Buffer` == Uint8Array; addons returning bytes use these) ---
// turbo-html2pdf and most napi-rs addons that hand back binary data (PDF bytes,
// images, etc.) call the buffer family. These were previously UNEXPORTED, so the
// addon's reference resolved (flat namespace, -export_dynamic) to address 0x0 and
// the first call jumped to NULL -> SIGSEGV that killed the whole run with no output.
// Back them with a Uint8Array (the runtime exposes Buffer as a Uint8Array subclass),
// which is what real Buffer-consuming JS expects.

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer(
    env: napi_env,
    len: usize,
    data: *mut *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let ab = v8::ArrayBuffer::new(s, len);
    if !data.is_null() {
        *data = ab
            .get_backing_store()
            .data()
            .map(|p| p.as_ptr())
            .unwrap_or(std::ptr::null_mut());
    }
    let buf: v8::Local<v8::Value> = v8::Uint8Array::new(s, ab, 0, len)
        .map(|t| t.into())
        .unwrap_or_else(|| ab.into());
    put(out, buf);
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer_copy(
    env: napi_env,
    len: usize,
    src: *const c_void,
    result_data: *mut *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
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
    let buf: v8::Local<v8::Value> = v8::Uint8Array::new(s, ab, 0, len)
        .map(|t| t.into())
        .unwrap_or_else(|| ab.into());
    put(out, buf);
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external_buffer(
    env: napi_env,
    len: usize,
    data: *mut c_void,
    _finalize: Option<napi_finalize>,
    _hint: *mut c_void,
    out: *mut napi_value,
) -> napi_status {
    // We don't track the external lifetime; copy into a managed buffer.
    let s = scope(env);
    let ab = v8::ArrayBuffer::new(s, len);
    if let Some(store) = ab.get_backing_store().data() {
        if !data.is_null() && len > 0 {
            std::ptr::copy_nonoverlapping(data as *const u8, store.as_ptr() as *mut u8, len);
        }
    }
    let buf: v8::Local<v8::Value> = v8::Uint8Array::new(s, ab, 0, len)
        .map(|t| t.into())
        .unwrap_or_else(|| ab.into());
    put(out, buf);
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_buffer_info(
    env: napi_env,
    v: napi_value,
    data: *mut *mut c_void,
    length: *mut usize,
) -> napi_status {
    let s = scope(env);
    let val = to_local(v);
    // Accept any ArrayBufferView (Uint8Array Buffer) or a raw ArrayBuffer.
    if let Ok(view) = TryInto::<v8::Local<v8::ArrayBufferView>>::try_into(val) {
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
        return napi_ok;
    }
    if let Ok(ab) = TryInto::<v8::Local<v8::ArrayBuffer>>::try_into(val) {
        if !length.is_null() {
            *length = ab.byte_length();
        }
        if !data.is_null() {
            *data = ab
                .get_backing_store()
                .data()
                .map(|p| p.as_ptr())
                .unwrap_or(std::ptr::null_mut());
        }
        return napi_ok;
    }
    napi_invalid_arg
}

// ---- more value getters (bool/double/int64) -------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bool(
    _env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    if !out.is_null() {
        *out = to_local(v).boolean_value(&mut *((*_env).scope as *mut v8::PinScope<'static, 'static>));
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_double(
    env: napi_env,
    v: napi_value,
    out: *mut f64,
) -> napi_status {
    let s = scope(env);
    if !out.is_null() {
        *out = to_local(v).number_value(s).unwrap_or(0.0);
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int64(
    env: napi_env,
    v: napi_value,
    out: *mut i64,
) -> napi_status {
    let s = scope(env);
    if !out.is_null() {
        *out = to_local(v).number_value(s).unwrap_or(0.0) as i64;
    }
    napi_ok
}

// ---- arrays (length / element access / type test / key enumeration) -------

#[no_mangle]
pub unsafe extern "C" fn napi_is_array(
    _env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    if !out.is_null() {
        *out = to_local(v).is_array();
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_array_length(
    env: napi_env,
    v: napi_value,
    out: *mut u32,
) -> napi_status {
    let arr: v8::Local<v8::Array> = match to_local(v).try_into() {
        Ok(a) => a,
        Err(_) => return napi_invalid_arg,
    };
    let _ = env;
    if !out.is_null() {
        *out = arr.length();
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_element(
    env: napi_env,
    obj: napi_value,
    index: u32,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let o: v8::Local<v8::Object> = match to_local(obj).try_into() {
        Ok(o) => o,
        Err(_) => return napi_invalid_arg,
    };
    match o.get_index(s, index) {
        Some(v) => {
            put(out, v);
            napi_ok
        }
        None => napi_generic_failure,
    }
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_property_names(
    env: napi_env,
    obj: napi_value,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let o: v8::Local<v8::Object> = match to_local(obj).try_into() {
        Ok(o) => o,
        Err(_) => return napi_invalid_arg,
    };
    match o.get_own_property_names(s, v8::GetPropertyNamesArgs::default()) {
        Some(names) => {
            put(out, names.into());
            napi_ok
        }
        None => napi_generic_failure,
    }
}

// ---- wrap / new_instance: not supported; surface as a CATCHABLE JS throw ----
// Wrapping a native pointer on a JS object (napi_wrap/unwrap) and constructing a
// native class instance (napi_new_instance) need lifecycle/finalizer machinery we
// don't model. Rather than leave these UNEXPORTED (null pointer -> SIGSEGV when the
// addon calls them), export them and throw a clear, catchable JS error so the
// require() fails as a normal load-error and the rest of the run survives.
unsafe fn throw_unsupported(env: napi_env, name: &str) -> napi_status {
    let s = scope(env);
    let msg = format!("turbo-test: N-API {name} is not implemented (native addon called an unsupported entrypoint)");
    if let Some(m) = v8::String::new(s, &msg) {
        let exc = v8::Exception::error(s, m);
        s.throw_exception(exc);
    }
    napi_pending_exception
}
#[no_mangle]
pub unsafe extern "C" fn napi_wrap(
    env: napi_env,
    _js_object: napi_value,
    _native_object: *mut c_void,
    _finalize_cb: *mut c_void,
    _finalize_hint: *mut c_void,
    _result: *mut napi_ref,
) -> napi_status {
    throw_unsupported(env, "napi_wrap")
}
#[no_mangle]
pub unsafe extern "C" fn napi_unwrap(
    env: napi_env,
    _js_object: napi_value,
    _result: *mut *mut c_void,
) -> napi_status {
    throw_unsupported(env, "napi_unwrap")
}
#[no_mangle]
pub unsafe extern "C" fn napi_new_instance(
    env: napi_env,
    _constructor: napi_value,
    _argc: usize,
    _argv: *const napi_value,
    _result: *mut napi_value,
) -> napi_status {
    throw_unsupported(env, "napi_new_instance")
}

// ---- errors / exceptions --------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_throw(env: napi_env, error: napi_value) -> napi_status {
    let s = scope(env);
    s.throw_exception(to_local(error));
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_error(
    env: napi_env,
    _code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    let s = scope(env);
    let m = std::ffi::CStr::from_ptr(msg).to_string_lossy();
    let ms = v8::String::new(s, &m).unwrap();
    let exc = v8::Exception::error(s, ms);
    s.throw_exception(exc);
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_error(
    env: napi_env,
    _code: napi_value,
    msg: napi_value,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let ms: v8::Local<v8::String> = match to_local(msg).try_into() {
        Ok(m) => m,
        Err(_) => return napi_invalid_arg,
    };
    let exc = v8::Exception::error(s, ms);
    put(out, exc);
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_is_error(
    _env: napi_env,
    v: napi_value,
    out: *mut bool,
) -> napi_status {
    if !out.is_null() {
        *out = to_local(v).is_native_error();
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_is_exception_pending(
    env: napi_env,
    out: *mut bool,
) -> napi_status {
    let e = &mut *env;
    if !out.is_null() {
        *out = e.last_exception.is_some();
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_and_clear_last_exception(
    env: napi_env,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let e = &mut *env;
    match e.last_exception.take() {
        Some(g) => {
            let l = v8::Local::new(s, &g);
            put(out, l);
        }
        None => put(out, v8::undefined(s).into()),
    }
    napi_ok
}

// ---- references (minimal: hold a Global) ----------------------------------

#[no_mangle]
pub unsafe extern "C" fn napi_create_reference(
    env: napi_env,
    v: napi_value,
    _initial_refcount: u32,
    out: *mut napi_ref,
) -> napi_status {
    let s = scope(env);
    let g = v8::Global::new(s, to_local(v));
    let e = &mut *env;
    e.refs.push(Some(g));
    let id = e.refs.len();
    if !out.is_null() {
        *out = id as napi_ref;
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_reference_value(
    env: napi_env,
    r: napi_ref,
    out: *mut napi_value,
) -> napi_status {
    let s = scope(env);
    let e = &mut *env;
    let id = r as usize;
    if id == 0 || id > e.refs.len() {
        return napi_invalid_arg;
    }
    match e.refs[id - 1].as_ref() {
        Some(g) => put(out, v8::Local::new(s, g)),
        None => put(out, v8::undefined(s).into()),
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_delete_reference(env: napi_env, r: napi_ref) -> napi_status {
    let e = &mut *env;
    let id = r as usize;
    if id >= 1 && id <= e.refs.len() {
        e.refs[id - 1] = None;
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_reference_unref(
    _env: napi_env,
    _r: napi_ref,
    _result: *mut u32,
) -> napi_status {
    napi_ok
}

// ---- stubs (threadsafe fns / cleanup hooks — not exercised by sync parse) ---

#[no_mangle]
pub unsafe extern "C" fn napi_add_env_cleanup_hook(
    _env: napi_env,
    _fun: *mut c_void,
    _arg: *mut c_void,
) -> napi_status {
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_stub() -> napi_status {
    napi_generic_failure
}

/// define_class → treat the constructor as a plain function (prototype methods skipped;
/// sufficient for addons whose exercised exports are functions, e.g. the parser).
#[no_mangle]
pub unsafe extern "C" fn napi_define_class(
    env: napi_env,
    name: *const c_char,
    len: usize,
    ctor: napi_callback,
    data: *mut c_void,
    _property_count: usize,
    _properties: *const c_void,
    out: *mut napi_value,
) -> napi_status {
    napi_create_function(env, name, len, ctor, data, out)
}

// threadsafe-function family: napi-rs creates a GC-finalizer tsfn at module init, so
// create MUST succeed (hand back a dummy handle). The tsfn never actually fires for our
// synchronous parse use, so call/unref are no-ops.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn napi_create_threadsafe_function(
    _env: napi_env,
    _func: napi_value,
    _async_resource: napi_value,
    _async_resource_name: napi_value,
    _max_queue_size: usize,
    _initial_thread_count: usize,
    _thread_finalize_data: *mut c_void,
    _thread_finalize_cb: *mut c_void,
    _context: *mut c_void,
    _call_js_cb: *mut c_void,
    result: *mut *mut c_void,
) -> napi_status {
    if !result.is_null() {
        *result = 1usize as *mut c_void; // non-null dummy handle
    }
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_call_threadsafe_function(
    _func: *mut c_void,
    _data: *mut c_void,
    _mode: i32,
) -> napi_status {
    napi_ok
}
#[no_mangle]
pub unsafe extern "C" fn napi_unref_threadsafe_function(
    _env: napi_env,
    _func: *mut c_void,
) -> napi_status {
    napi_ok
}

/// The module init symbol exported by the addon.
type RegisterFn = unsafe extern "C" fn(napi_env, napi_value) -> napi_value;

/// Load a `.node` addon: dlopen, set up the env, call `napi_register_module_v1` with a
/// fresh exports object, and return that object. Must be called inside a live scope.
pub fn load_addon(
    scope_ref: &mut v8::PinScope,
    path: &Path,
) -> Result<v8::Global<v8::Value>, String> {
    unsafe {
        let lib = libloading::Library::new(path).map_err(|e| format!("dlopen {e}"))?;
        let register: libloading::Symbol<RegisterFn> = lib
            .get(b"napi_register_module_v1")
            .map_err(|e| format!("no napi_register_module_v1: {e}"))?;

        let isolate = std::ptr::null_mut();
        let context = v8::Global::new(scope_ref, scope_ref.get_current_context());
        let env = Box::new(Env {
            isolate,
            scope: scope_ref as *mut v8::PinScope as *mut c_void,
            context: Some(context),
            last_exception: None,
            fns: Vec::new(),
            refs: Vec::new(),
        });
        ENV.with(|e| *e.borrow_mut() = Some(env));

        let exports = v8::Object::new(scope_ref);
        // The addon's module init runs arbitrary native code. Guard it two ways:
        //  - A TryCatch captures any JS exception it throws (e.g. an unsupported N-API call we
        //    route through throw_unsupported) so it surfaces as a clean require() failure.
        //  - catch_unwind turns a Rust panic crossing the `extern "C"` FFI boundary (which is UB
        //    and can abort/segfault, taking the whole run down with no diagnostic) into a
        //    recoverable Err -> a thrown JS error in the loader. The rest of the run survives.
        let exports_napi = to_napi(exports.into());
        let tc = std::pin::pin!(v8::TryCatch::new(scope_ref));
        let tc = &mut tc.init();
        // napi value calls deref env.scope as *mut PinScope; a TryCatch derefs to the underlying
        // scope, so hand the addon that pointer (valid for the duration of register()).
        let scope_addr =
            (&mut **tc) as *mut v8::PinScope as *mut c_void;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ENV.with(|e| {
                let mut b = e.borrow_mut();
                let env = b.as_mut().unwrap();
                env.scope = scope_addr;
                let env_ptr: napi_env = env.as_mut() as *mut Env;
                register(env_ptr, exports_napi)
            })
        }));
        let result = match result {
            Ok(r) => r,
            Err(_) => {
                return Err(format!("native addon panicked during module init: {}", path.display()));
            }
        };
        if tc.has_caught() {
            let msg = tc
                .exception()
                .map(|e| e.to_rust_string_lossy(tc))
                .unwrap_or_else(|| "exception during module init".into());
            return Err(format!("native addon threw during module init: {msg}"));
        }
        let ret_local = if result.is_null() {
            exports.into()
        } else {
            to_local(result)
        };
        // keep the lib loaded for the process lifetime
        std::mem::forget(lib);
        Ok(v8::Global::new(tc, ret_local))
    }
}
