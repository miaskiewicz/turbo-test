//! Unit tests for the napi host paths an addon cannot drive through `require()`: the panic
//! guards (a C callback cannot unwind into Rust), stored pending exceptions, termination
//! handling, same-thread re-entry of the addon lock, and the trampoline's dispatch through its
//! callback record.
//! The end-to-end surface is covered by tests/napi_host.rs.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Run `f` inside a fresh isolate + context. The isolate boots from the runner's framework
/// snapshot like every production isolate: V8's read-only heap is shared process-wide, so an
/// isolate from V8's built-in snapshot must never be alive at the same time as snapshot-booted
/// ones (other unit tests run concurrently and use the framework snapshot).
fn with_scope(f: impl FnOnce(&mut v8::PinScope)) {
    crate::runner::init_v8();
    let blob = crate::runner::framework_snapshot();
    let params = v8::Isolate::create_params().snapshot_blob(v8::StartupData::from(blob.clone()));
    let isolate = &mut v8::Isolate::new(params);
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    f(scope);
}

/// An env whose napi calls run in `scope`.
fn env_for(scope: &mut v8::PinScope) -> Box<Env> {
    Box::new(Env::new(scope))
}

#[test]
fn init_outcome_maps_a_panic_to_a_load_error() {
    let path = Path::new("/addons/demo.node");
    let value = 0x40usize as napi_value;
    assert_eq!(init_outcome(Ok(value), path), Ok(value));
    assert_eq!(
        init_outcome(Err(Box::new("boom")), path),
        Err("native addon panicked during module init: /addons/demo.node".to_string())
    );
}

#[test]
fn callback_return_value_null_and_panic() {
    with_scope(|s| {
        let tc = std::pin::pin!(v8::TryCatch::new(s));
        let tc = &mut tc.init();
        let five = unsafe { to_napi(v8::Integer::new(tc, 5).into()) };
        assert_eq!(callback_return(tc, Ok(five)), Some(five));
        assert_eq!(callback_return(tc, Ok(std::ptr::null_mut())), None);
        assert!(!tc.has_caught());
        assert_eq!(callback_return(tc, Err(Box::new("boom"))), None);
        assert!(tc.has_caught());
        let msg = tc.exception().unwrap().to_rust_string_lossy(tc);
        assert_eq!(msg, "Error: turbo-test: native addon callback panicked");
    });
}

#[test]
fn stored_pending_exception_is_reported_then_cleared() {
    with_scope(|s| {
        let mut env = env_for(s);
        let stored: v8::Local<v8::Value> = v8::String::new(s, "stored").unwrap().into();
        env.last_exception = Some(v8::Global::new(s, stored));
        let e: napi_env = &mut *env;
        unsafe {
            let mut pending = false;
            assert_eq!(napi_is_exception_pending(e, &mut pending), napi_ok);
            assert!(pending);
            let mut out: napi_value = std::ptr::null_mut();
            assert_eq!(napi_get_and_clear_last_exception(e, &mut out), napi_ok);
            assert_eq!(to_local(out).to_rust_string_lossy(s), "stored");
            assert_eq!(napi_is_exception_pending(e, &mut pending), napi_ok);
            assert!(!pending);
            assert_eq!(napi_get_and_clear_last_exception(e, &mut out), napi_ok);
            assert!(to_local(out).is_undefined());
        }
    });
}

#[test]
fn addon_lock_is_reentrant_per_thread_and_exclusive_across_threads() {
    std::thread::spawn(|| {
        let (outer_reentrant, outer) = enter_addon();
        assert!(!outer_reentrant);
        assert!(outer.is_some());
        assert!(IN_ADDON.with(|f| f.get()));
        // another thread cannot enter while this one is inside the addon
        let blocked = std::thread::spawn(|| ADDON_LOCK.try_lock().is_err())
            .join()
            .unwrap();
        assert!(blocked);
        // same-thread re-entry takes no lock (no self-deadlock)
        let (inner_reentrant, inner) = enter_addon();
        assert!(inner_reentrant);
        assert!(inner.is_none());
        IN_ADDON.with(|f| f.set(false));
        drop(outer);
        let free = std::thread::spawn(|| ADDON_LOCK.try_lock().is_ok())
            .join()
            .unwrap();
        assert!(free);
    })
    .join()
    .unwrap();
}

static CALLED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn record_call(_env: napi_env, _info: napi_callback_info) -> napi_value {
    CALLED.store(true, Ordering::SeqCst);
    std::ptr::null_mut()
}

#[test]
fn trampoline_dispatches_through_the_callback_record() {
    with_scope(|s| {
        let recv: v8::Local<v8::Value> = v8::undefined(s).into();
        // a function on the trampoline without a callback record (not made by
        // napi_create_function) calls nothing and returns undefined
        let bare = v8::FunctionTemplate::new(s, napi_fn_trampoline)
            .get_function(s)
            .unwrap();
        assert!(bare.call(s, recv, &[]).unwrap().is_undefined());
        assert!(!CALLED.load(Ordering::SeqCst));
        // a napi-created function reaches its addon callback through the record
        let mut env = env_for(s);
        let mut f: napi_value = std::ptr::null_mut();
        let status = unsafe {
            napi_create_function(
                &mut *env,
                std::ptr::null(),
                0,
                Some(record_call),
                std::ptr::null_mut(),
                &mut f,
            )
        };
        assert_eq!(status, napi_ok);
        assert_eq!(env.fns.len(), 1);
        let func: v8::Local<v8::Function> = unsafe { to_local(f) }.try_into().unwrap();
        assert!(func.call(s, recv, &[]).unwrap().is_undefined());
        assert!(CALLED.load(Ordering::SeqCst));
    });
}

fn terminate(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    scope.terminate_execution();
}

#[test]
fn js_call_parks_exceptions_but_not_a_termination() {
    with_scope(|s| {
        let mut env = env_for(s);
        let e: napi_env = &mut *env;
        let recv = unsafe { to_napi(v8::undefined(s).into()) };
        let mut out: napi_value = std::ptr::null_mut();
        let term = v8::Function::new(s, terminate).unwrap();
        let key = v8::String::new(s, "term").unwrap();
        let global = s.get_current_context().global(s);
        global.set(s, key.into(), term.into()).unwrap();
        let js = |s: &mut v8::PinScope, src: &str| {
            let src = v8::String::new(s, src).unwrap();
            let f = v8::Script::compile(s, src, None).unwrap().run(s).unwrap();
            unsafe { to_napi(f) }
        };
        // a termination (e.g. a test timeout) inside JS the addon called keeps unwinding: it
        // fails the call but is not parked as a pending exception to be rethrown later
        let spin = js(s, "() => { term(); for (;;) {} }");
        let st = unsafe { napi_call_function(e, recv, spin, 0, std::ptr::null(), &mut out) };
        s.cancel_terminate_execution();
        assert_eq!(st, napi_pending_exception);
        assert!(env.last_exception.is_none());
        // a JS exception is parked on the env
        let thrower = js(s, "() => { throw 7 }");
        let st = unsafe { napi_call_function(e, recv, thrower, 0, std::ptr::null(), &mut out) };
        assert_eq!(st, napi_pending_exception);
        let parked = env.last_exception.take().unwrap();
        assert_eq!(v8::Local::new(s, &parked).int32_value(s), Some(7));
    });
}

/// napi_reference_ref / napi_reference_unref of a NULL, unknown or deleted reference are
/// napi_invalid_arg (Node checks NULL; an unknown handle is undefined behaviour there).
#[test]
fn ref_and_unref_of_a_dead_reference_are_invalid_arg() {
    with_scope(|s| {
        let mut env = env_for(s);
        let e: napi_env = &mut *env;
        let mut count = 7u32;
        unsafe {
            for r in [std::ptr::null_mut(), 99usize as napi_ref] {
                assert_eq!(napi_reference_ref(e, r, &mut count), napi_invalid_arg);
                assert_eq!(napi_reference_unref(e, r, &mut count), napi_invalid_arg);
            }
            let obj = to_napi(v8::Object::new(s).into());
            let mut r: napi_ref = std::ptr::null_mut();
            assert_eq!(napi_create_reference(e, obj, 1, &mut r), napi_ok);
            assert_eq!(napi_delete_reference(e, r), napi_ok);
            assert_eq!(napi_reference_ref(e, r, &mut count), napi_invalid_arg);
            assert_eq!(napi_reference_unref(e, r, &mut count), napi_invalid_arg);
        }
        assert_eq!(count, 7);
        // a counted reference whose value a finalizer cleared: ref / unref leave it at 0, as
        // Node's Reference::Ref / Unref of an empty persistent
        let mut cleared = Ref {
            count: 1,
            value: RefValue::Empty,
        };
        assert_eq!(cleared.dec(s), 0);
        assert_eq!(cleared.inc(s), 0);
        assert_eq!(cleared.count, 1);
    });
}

/// A closure of `catching` reporting that V8 gave nothing: napi_pending_exception when it
/// threw, napi_generic_failure otherwise (Node's RETURN_STATUS_IF_FALSE_WITH_PREAMBLE); the
/// status is recorded as the last error.
#[test]
fn v8_giving_nothing_is_pending_exception_or_generic_failure() {
    with_scope(|s| {
        let mut env = env_for(s);
        let e: napi_env = &mut *env;
        unsafe {
            assert_eq!(catching(e, |_| Err(V8_GAVE_NOTHING)), napi_generic_failure);
            assert_eq!(env.last_error.error_code, napi_generic_failure);
            let st = catching(e, |s| {
                let msg = v8::String::new(s, "thrown").unwrap();
                let exc = v8::Exception::error(s, msg);
                s.throw_exception(exc);
                Err(V8_GAVE_NOTHING)
            });
            assert_eq!(st, napi_pending_exception);
            assert!(env.last_exception.take().is_some());
        }
    });
}

/// A dlopen failure names an import the process does not provide (the loaders' three message
/// forms); any other failure keeps the loader's message.
#[test]
fn dlopen_errors_name_the_missing_import() {
    let mac = "dlopen(/p/a.node, 0x0006): symbol not found in flat namespace '_napi_bogus'";
    assert_eq!(missing_symbol(mac), Some("napi_bogus"));
    assert_eq!(
        dlopen_error(mac),
        format!("it imports napi_bogus, which turbo-test's Node-API host does not provide (dlopen {mac})")
    );
    let glibc = "/p/a.node: undefined symbol: node_api_bogus";
    assert_eq!(missing_symbol(glibc), Some("node_api_bogus"));
    let musl = "Error relocating /p/a.node: uv_bogus: symbol not found";
    assert_eq!(missing_symbol(musl), Some("uv_bogus"));
    assert_eq!(
        dlopen_error(musl),
        format!("it imports uv_bogus, which nothing loaded provides (dlopen {musl})")
    );
    let other = "dlopen(/p/a.node, 0x0006): slice is not valid mach-o file";
    assert_eq!(missing_symbol(other), None);
    assert_eq!(dlopen_error(other), format!("dlopen {other}"));
    assert_eq!(
        missing_symbol("symbol not found in flat namespace '_x"),
        Some("x")
    );
}

/// node_api_get_module_file_name's file: URL escapes `%` and the URL path percent-encode set
/// (as Node's url::FromFilePath).
#[test]
fn module_file_names_are_file_urls() {
    assert_eq!(file_url(Path::new("/a/b.node")), "file:///a/b.node");
    assert_eq!(
        file_url(Path::new("/x y/%#?\"<>`{}/é.node")),
        "file:///x%20y/%25%23%3F%22%3C%3E%60%7B%7D/%C3%A9.node"
    );
}

unsafe extern "C" fn execute_nothing(_env: napi_env, _data: *mut c_void) {}

/// The drive loop's view of an isolate's native work: before any addon nothing is queued or
/// in flight; queued async work keeps the loop alive until its complete has run, and waiting
/// with no timeout returns once the pool posted it.
#[test]
fn native_tasks_keep_the_loop_alive_until_they_run() {
    with_scope(|s| {
        assert!(!run_native_task(s));
        assert!(!keeps_loop_alive(s));
        assert!(!wait_native_task(s, Some(std::time::Duration::ZERO)));
        let mut env = env_for(s);
        let e: napi_env = &mut *env;
        unsafe {
            let name = to_napi(v8::String::new(s, "unit").unwrap().into());
            let mut work: *mut async_host::AsyncWork = std::ptr::null_mut();
            let st = async_host::napi_create_async_work(
                e,
                std::ptr::null_mut(),
                name,
                Some(execute_nothing),
                None,
                std::ptr::null_mut(),
                &mut work,
            );
            assert_eq!(st, napi_ok);
            assert_eq!(async_host::napi_queue_async_work(e, work), napi_ok);
            assert!(keeps_loop_alive(s));
            assert!(wait_native_task(s, None));
            assert!(run_native_task(s));
            assert!(!keeps_loop_alive(s));
            assert_eq!(async_host::napi_delete_async_work(e, work), napi_ok);
        }
    });
}

/// The host-only answers where Node crashes or aborts the process: a NULL deferred, and the
/// same env cleanup hook (function and argument) registered twice.
#[test]
fn null_deferred_and_duplicate_cleanup_hook_are_invalid_arg() {
    unsafe extern "C" fn hook(_: *mut c_void) {}
    with_scope(|s| {
        let mut env = env_for(s);
        let e: napi_env = &mut *env;
        let arg = std::ptr::dangling_mut::<c_void>();
        unsafe {
            let v = to_napi(v8::undefined(s).into());
            let null = std::ptr::null_mut();
            assert_eq!(napi_resolve_deferred(e, null, v), napi_invalid_arg);
            assert_eq!(napi_reject_deferred(e, null, v), napi_invalid_arg);
            assert_eq!(napi_add_env_cleanup_hook(e, Some(hook), arg), napi_ok);
            assert_eq!(
                napi_add_env_cleanup_hook(e, Some(hook), arg),
                napi_invalid_arg
            );
            // another argument is another hook
            assert_eq!(napi_add_env_cleanup_hook(e, Some(hook), null), napi_ok);
            assert_eq!(napi_remove_env_cleanup_hook(e, Some(hook), arg), napi_ok);
            assert_eq!(napi_remove_env_cleanup_hook(e, Some(hook), null), napi_ok);
            assert!(teardown_of(e).borrow().is_empty());
        }
    });
}
