//! The asynchronous Node-API surface of the host (src/napi_host.rs): async work on a worker
//! pool, threadsafe functions, async contexts and napi_make_callback, callback scopes, and the
//! event-loop handle.
//!
//! Node runs these on libuv; here each isolate has a `NativeLoop`, a queue of native callbacks
//! for its JS thread that any thread can post to. The runner's event loop (runner.rs
//! `drive_tests_bounded`) runs one queued callback per turn (`run_native_task`), and while
//! async work is in flight or a referenced threadsafe function is open it waits for one
//! (`wait_native_task`) before firing the next timer, for at most that timer's delay.

use super::*;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A raw pointer that crosses threads: the host's own records, whose use is serialized by the
/// queues below (the pool runs an async work's execute, the JS thread everything else).
struct SendPtr<T>(*mut T);
// SAFETY: see the type's doc; the records live until the JS thread frees them.
unsafe impl<T> Send for SendPtr<T> {}

/// One native callback waiting for its isolate's JS thread.
enum NativeTask {
    /// An async work's complete callback, with napi_ok or napi_cancelled.
    Complete(SendPtr<AsyncWork>, napi_status),
    /// A threadsafe function has queued calls (or is closing).
    Dispatch(SendPtr<Tsfn>),
    /// A threadsafe function's handles are closed: drop what is queued, run its finalizer.
    Finalize(SendPtr<Tsfn>),
    /// An exception a finalizer left: reported as uncaught.
    Uncaught(napi_env, SendPtr<v8::Global<v8::Value>>),
}

/// The native callbacks posted for an isolate, and the async work it has in flight.
#[derive(Default)]
pub struct NativeLoop {
    state: Mutex<LoopState>,
    wake: Condvar,
}

#[derive(Default)]
struct LoopState {
    ready: VecDeque<NativeTask>,
    /// Async works queued or running whose complete has not run (they keep the loop alive).
    in_flight: usize,
}

impl NativeLoop {
    fn post(&self, task: NativeTask) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.ready.push_back(task);
        self.wake.notify_all();
    }
}

/// The shared napi state of `env`'s isolate.
unsafe fn shared_env<'a>(env: napi_env) -> &'a Shared {
    let e = &*env;
    &e.shared
}

/// The isolate's shared napi state, None before any addon loaded into it.
fn shared_of(scope: &mut v8::PinScope) -> Option<std::rc::Rc<Shared>> {
    scope.get_slot::<AddonEnvs>().map(|envs| envs.0.clone())
}

/// Run one native callback queued for this isolate's addons (an async work's complete, one
/// threadsafe-function call or finalization): false when none is queued.
pub fn run_native_task(scope: &mut v8::PinScope) -> bool {
    let Some(shared) = shared_of(scope) else {
        return false;
    };
    let task = shared.native.state.lock().unwrap().ready.pop_front();
    let Some(task) = task else {
        return false;
    };
    unsafe {
        match task {
            NativeTask::Complete(work, status) => complete_work(scope, work.0, status),
            NativeTask::Dispatch(tsfn) => dispatch_one(scope, tsfn.0),
            NativeTask::Uncaught(env, exc) => {
                let exc = Box::from_raw(exc.0);
                in_env_scope(scope, env, || {
                    let s = super::scope(env);
                    let exc = v8::Local::new(s, &*exc);
                    report_uncaught(s, exc);
                });
            }
            NativeTask::Finalize(tsfn) => {
                let env = (*tsfn.0).env;
                in_env_scope(scope, env, || finalize_tsfn(tsfn.0));
            }
        }
    }
    true
}

/// Report `exc`, which a finalizer left, as an uncaught exception once the JS thread runs its
/// native callbacks again.
pub(super) unsafe fn defer_uncaught(env: napi_env, exc: v8::Global<v8::Value>) {
    let exc = SendPtr(Box::into_raw(Box::new(exc)));
    shared_env(env).native.post(NativeTask::Uncaught(env, exc));
}

/// Whether this isolate's addons keep the event loop alive, as libuv's referenced handles and
/// requests do: async work in flight, or an open threadsafe function that is referenced.
pub fn keeps_loop_alive(scope: &mut v8::PinScope) -> bool {
    let Some(shared) = shared_of(scope) else {
        return false;
    };
    let in_flight = shared.native.state.lock().unwrap().in_flight > 0;
    in_flight
        || shared
            .tsfns
            .borrow()
            .iter()
            .any(|&t| unsafe { (*t).refed && !(*t).handles_closing })
}

/// Wait up to `timeout` (forever: None) for a native callback to be posted for this isolate:
/// whether one is queued.
pub fn wait_native_task(scope: &mut v8::PinScope, timeout: Option<Duration>) -> bool {
    let Some(shared) = shared_of(scope) else {
        return false;
    };
    let native = &shared.native;
    let st = native.state.lock().unwrap();
    let st = match timeout {
        Some(t) => {
            native
                .wake
                .wait_timeout_while(st, t, |s| s.ready.is_empty())
                .unwrap()
                .0
        }
        None => native.wake.wait_while(st, |s| s.ready.is_empty()).unwrap(),
    };
    !st.ready.is_empty()
}

/// Run `f` for `env` in `scope` (the JS thread's, between macrotasks): the env's calls use it.
unsafe fn in_env_scope(scope: &mut v8::PinScope, env: napi_env, f: impl FnOnce()) {
    v8::scope!(let hs, scope);
    let here = hs as *mut v8::PinScope as *mut c_void;
    let outer = std::mem::replace(&mut (*env).scope, here);
    f();
    (*env).scope = outer;
}

/// What an exception left pending by an asynchronous native callback becomes, as Node's
/// CallbackIntoModule: an uncaught exception, or (an addon below module API version 10, for a
/// threadsafe-function call) a deprecation warning, once, and nothing else.
unsafe fn callback_exception(env: napi_env, enforce: bool) {
    let Some(exc) = (*env).last_exception.take() else {
        return;
    };
    let s = scope(env);
    if enforce || (*env).module_api_version >= 10 {
        let exc = v8::Local::new(s, &exc);
        report_uncaught(s, exc);
    } else {
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "[DEP0168] DeprecationWarning: Uncaught Node-API callback exception detected, \
                 please run node with option --force-node-api-uncaught-exceptions-policy=true \
                 to handle those exceptions properly."
            );
        }
    }
}

// ---- async work ----------------------------------------------------------------

type ExecuteFn = unsafe extern "C" fn(napi_env, *mut c_void);
type CompleteFn = unsafe extern "C" fn(napi_env, napi_status, *mut c_void);

/// A napi_async_work: execute runs on the worker pool, then complete on the JS thread.
pub struct AsyncWork {
    env: napi_env,
    execute: ExecuteFn,
    complete: Option<CompleteFn>,
    data: *mut c_void,
    native: Arc<NativeLoop>,
}

/// The process's worker pool, as libuv's (4 threads, a FIFO of queued work).
struct Pool {
    jobs: Mutex<VecDeque<SendPtr<AsyncWork>>>,
    ready: Condvar,
}

/// The pool's size (libuv's default UV_THREADPOOL_SIZE).
const POOL_THREADS: usize = 4;

/// libuv's UV_EBUSY: what canceling work that already started reports as its engine error.
const UV_EBUSY: i32 = -16;

fn pool() -> &'static Pool {
    static POOL: std::sync::OnceLock<&'static Pool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let pool: &'static Pool = Box::leak(Box::new(Pool {
            jobs: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        }));
        for _ in 0..POOL_THREADS {
            std::thread::spawn(move || pool_thread(pool));
        }
        pool
    })
}

/// A pool thread: run each queued work's execute, then post its complete.
fn pool_thread(pool: &'static Pool) {
    loop {
        let work = {
            let jobs = pool.jobs.lock().unwrap();
            let mut jobs = pool.ready.wait_while(jobs, |j| j.is_empty()).unwrap();
            jobs.pop_front().unwrap()
        };
        unsafe {
            let w = &*work.0;
            let (execute, env, data) = (w.execute, w.env, w.data);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(env, data)));
            w.native.post(NativeTask::Complete(work, napi_ok));
        }
    }
}

/// Run a finished (or cancelled) work's complete on the JS thread, as Node's
/// AfterThreadPoolWork: in a handle scope, an exception it leaves is uncaught.
unsafe fn complete_work(scope: &mut v8::PinScope, work: *mut AsyncWork, status: napi_status) {
    let (env, complete, data) = ((*work).env, (*work).complete, (*work).data);
    (&*work).native.state.lock().unwrap().in_flight -= 1;
    if let Some(complete) = complete {
        in_env_scope(scope, env, || {
            let _ = call_addon(|| complete(env, status, data));
            callback_exception(env, true);
        });
    }
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn napi_create_async_work(
    env: napi_env,
    async_resource: napi_value,
    async_resource_name: napi_value,
    execute: Option<ExecuteFn>,
    complete: Option<CompleteFn>,
    data: *mut c_void,
    result: *mut *mut AsyncWork,
) -> napi_status {
    basic(env, || {
        let execute = execute.ok_or(napi_invalid_arg)?;
        let result = out_arg(result)?;
        resource_and_name(env, async_resource, async_resource_name)?;
        let native = shared_env(env).native.clone();
        *result = Box::into_raw(Box::new(AsyncWork {
            env,
            execute,
            complete,
            data,
            native,
        }));
        Ok(())
    })
}

/// The async resource and its name (napi_create_async_work, napi_async_init, threadsafe
/// functions), as Node checks them: a resource is ToObject'd (napi_object_expected), the name
/// is required and ToString'd (napi_string_expected). The host keeps neither (no async_hooks).
unsafe fn resource_and_name(env: napi_env, resource: napi_value, name: napi_value) -> NapiResult {
    let s = scope(env);
    if !resource.is_null() {
        to_object(s, resource)?;
    }
    val_arg(name)?
        .to_string(s)
        .ok_or(napi_string_expected)
        .map(|_| ())
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_async_work(
    env: napi_env,
    work: *mut AsyncWork,
) -> napi_status {
    basic(env, || {
        if work.is_null() {
            return Err(napi_invalid_arg);
        }
        drop(Box::from_raw(work));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_queue_async_work(env: napi_env, work: *mut AsyncWork) -> napi_status {
    basic(env, || {
        if work.is_null() {
            return Err(napi_invalid_arg);
        }
        (&*work).native.state.lock().unwrap().in_flight += 1;
        let pool = pool();
        pool.jobs.lock().unwrap().push_back(SendPtr(work));
        pool.ready.notify_one();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_cancel_async_work(
    env: napi_env,
    work: *mut AsyncWork,
) -> napi_status {
    if env.is_null() {
        return napi_invalid_arg;
    }
    if work.is_null() {
        return record(env, napi_invalid_arg);
    }
    // as Node's uv_cancel: work still waiting for a pool thread is cancelled (its complete gets
    // napi_cancelled); work that started cannot be (napi_generic_failure, engine error EBUSY)
    let mut jobs = pool().jobs.lock().unwrap();
    let Some(at) = jobs.iter().position(|j| j.0 == work) else {
        record(env, napi_generic_failure);
        (*env).last_error.engine_error_code = UV_EBUSY as u32;
        return napi_generic_failure;
    };
    jobs.remove(at);
    (&*work)
        .native
        .post(NativeTask::Complete(SendPtr(work), napi_cancelled));
    record(env, napi_ok)
}

/// What napi_get_uv_event_loop hands out: an opaque handle (the host has no libuv; an addon
/// importing uv_* functions fails to load, since nothing provides them).
static UV_LOOP: u8 = 0;

#[no_mangle]
pub unsafe extern "C" fn napi_get_uv_event_loop(
    env: napi_env,
    out: *mut *const c_void,
) -> napi_status {
    basic(env, || {
        *out_arg(out)? = (&UV_LOOP as *const u8).cast();
        Ok(())
    })
}

// ---- async contexts, napi_make_callback, callback scopes -------------------------------

/// A napi_async_context (the host has no async_hooks: it identifies nothing more).
pub struct AsyncContext;

#[no_mangle]
pub unsafe extern "C" fn napi_async_init(
    env: napi_env,
    async_resource: napi_value,
    async_resource_name: napi_value,
    result: *mut *mut AsyncContext,
) -> napi_status {
    basic(env, || {
        if async_resource_name.is_null() {
            return Err(napi_invalid_arg);
        }
        let result = out_arg(result)?;
        resource_and_name(env, async_resource, async_resource_name)?;
        *result = Box::into_raw(Box::new(AsyncContext));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_async_destroy(
    env: napi_env,
    async_context: *mut AsyncContext,
) -> napi_status {
    basic(env, || {
        if async_context.is_null() {
            return Err(napi_invalid_arg);
        }
        drop(Box::from_raw(async_context));
        Ok(())
    })
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn napi_make_callback(
    env: napi_env,
    _async_context: *mut AsyncContext,
    recv: napi_value,
    func: napi_value,
    argc: usize,
    argv: *const napi_value,
    result: *mut napi_value,
) -> napi_status {
    js_call(env, |s| {
        if recv.is_null() || (argc > 0 && argv.is_null()) {
            return Err(napi_invalid_arg);
        }
        let recv = to_object(s, recv)?;
        let f: v8::Local<v8::Function> = val_arg(func)?.try_into().map_err(|_| napi_invalid_arg)?;
        let args: Vec<v8::Local<v8::Value>> = (0..argc).map(|i| to_local(*argv.add(i))).collect();
        // (Node runs the nextTicks and microtasks the call queued when its outermost callback
        // scope closes: here, when the native call returns to JS or to the event loop)
        let ret = f.call(s, recv.into(), &args).ok_or(V8_GAVE_NOTHING)?;
        put(result, ret);
        Ok(())
    })
}

/// A napi_callback_scope (counted: a close without an open one is
/// napi_callback_scope_mismatch). The nextTicks and microtasks a callback queues run when the
/// native call returns to JS or to the event loop, as Node's outermost callback scope runs them.
pub struct CallbackScopeRecord;

#[no_mangle]
pub unsafe extern "C" fn napi_open_callback_scope(
    env: napi_env,
    _resource_object: napi_value,
    _async_context: *mut AsyncContext,
    result: *mut *mut CallbackScopeRecord,
) -> napi_status {
    basic(env, || {
        *out_arg(result)? = Box::into_raw(Box::new(CallbackScopeRecord));
        (*env).open_callback_scopes += 1;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_close_callback_scope(
    env: napi_env,
    scope_: *mut CallbackScopeRecord,
) -> napi_status {
    if env.is_null() {
        return napi_invalid_arg;
    }
    if scope_.is_null() {
        return record(env, napi_invalid_arg);
    }
    // as Node: more closes than opens is napi_callback_scope_mismatch (not recorded)
    if (*env).open_callback_scopes == 0 {
        return napi_callback_scope_mismatch;
    }
    (*env).open_callback_scopes -= 1;
    drop(Box::from_raw(scope_));
    record(env, napi_ok)
}

// ---- threadsafe functions --------------------------------------------------------

type CallJsFn = unsafe extern "C" fn(napi_env, napi_value, *mut c_void, *mut c_void);

/// A threadsafe function's state, as Node's ThreadSafeFunction::State.
#[derive(Clone, Copy, PartialEq)]
enum TsfnState {
    Open,
    Closing,
    Closed,
}

/// What every thread touches, under the threadsafe function's lock.
struct TsfnShared {
    queue: VecDeque<SendPtr<c_void>>,
    thread_count: usize,
    state: TsfnState,
    /// A Dispatch task is posted and has not run yet.
    dispatch_posted: bool,
}

/// A napi_threadsafe_function, as Node's v8impl::ThreadSafeFunction: calls queued from any
/// thread run one by one on the JS thread (`call_js`), until every thread released it (or one
/// aborted it); then its handles close and its finalizer runs.
pub struct Tsfn {
    env: napi_env,
    native: Arc<NativeLoop>,
    shared: Mutex<TsfnShared>,
    /// Blocking calls wait here while the queue is full.
    space: Condvar,
    max_queue_size: usize,
    context: *mut c_void,
    // the JS thread's
    func: Option<v8::Global<v8::Function>>,
    call_js: Option<CallJsFn>,
    finalize: Option<napi_finalize>,
    finalize_data: *mut c_void,
    refed: bool,
    handles_closing: bool,
}

impl Tsfn {
    fn lock(&self) -> std::sync::MutexGuard<'_, TsfnShared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ask the JS thread to dispatch (once until it does).
    fn send(&self, shared: &mut TsfnShared) {
        if !shared.dispatch_posted {
            shared.dispatch_posted = true;
            let me = self as *const Tsfn as *mut Tsfn;
            self.native.post(NativeTask::Dispatch(SendPtr(me)));
        }
    }
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn napi_create_threadsafe_function(
    env: napi_env,
    func: napi_value,
    async_resource: napi_value,
    async_resource_name: napi_value,
    max_queue_size: usize,
    initial_thread_count: usize,
    thread_finalize_data: *mut c_void,
    thread_finalize_cb: Option<napi_finalize>,
    context: *mut c_void,
    call_js_cb: Option<CallJsFn>,
    result: *mut *mut Tsfn,
) -> napi_status {
    basic(env, || {
        if async_resource_name.is_null() || initial_thread_count == 0 {
            return Err(napi_invalid_arg);
        }
        let result = out_arg(result)?;
        let s = scope(env);
        let func = if func.is_null() {
            call_js_cb.ok_or(napi_invalid_arg)?;
            None
        } else {
            let f: v8::Local<v8::Function> =
                to_local(func).try_into().map_err(|_| napi_invalid_arg)?;
            Some(v8::Global::new(s, f))
        };
        resource_and_name(env, async_resource, async_resource_name)?;
        let tsfn = Box::into_raw(Box::new(Tsfn {
            env,
            native: shared_env(env).native.clone(),
            shared: Mutex::new(TsfnShared {
                queue: VecDeque::new(),
                thread_count: initial_thread_count,
                state: TsfnState::Open,
                dispatch_posted: false,
            }),
            space: Condvar::new(),
            max_queue_size,
            context,
            func,
            call_js: call_js_cb,
            finalize: thread_finalize_cb,
            finalize_data: thread_finalize_data,
            refed: true,
            handles_closing: false,
        }));
        shared_env(env).tsfns.borrow_mut().push(tsfn);
        // as Node: an env cleanup hook closes it at teardown
        teardown_of(env).borrow_mut().push(Teardown::Tsfn(tsfn));
        *result = tsfn;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_threadsafe_function_context(
    func: *mut Tsfn,
    result: *mut *mut c_void,
) -> napi_status {
    // (NULL arguments: Node aborts)
    if func.is_null() || result.is_null() {
        return napi_invalid_arg;
    }
    *result = (*func).context;
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_threadsafe_function(
    func: *mut Tsfn,
    data: *mut c_void,
    is_blocking: i32,
) -> napi_status {
    if func.is_null() {
        return napi_invalid_arg;
    }
    let t = &*func;
    let mut shared = t.lock();
    // a full queue: napi_queue_full, or (napi_tsfn_blocking) wait for space
    while t.max_queue_size > 0
        && shared.queue.len() >= t.max_queue_size
        && shared.state == TsfnState::Open
    {
        if is_blocking == 0 {
            return napi_queue_full;
        }
        shared = t.space.wait(shared).unwrap_or_else(|e| e.into_inner());
    }
    if shared.state == TsfnState::Open {
        shared.queue.push_back(SendPtr(data));
        t.send(&mut shared);
        return napi_ok;
    }
    // closing: the calling thread lets go of it
    if shared.thread_count == 0 {
        return napi_invalid_arg;
    }
    shared.thread_count -= 1;
    let last = shared.state == TsfnState::Closed && shared.thread_count == 0;
    drop(shared);
    if last {
        drop(Box::from_raw(func));
    }
    napi_closing
}

#[no_mangle]
pub unsafe extern "C" fn napi_acquire_threadsafe_function(func: *mut Tsfn) -> napi_status {
    if func.is_null() {
        return napi_invalid_arg;
    }
    let mut shared = (*func).lock();
    if shared.state == TsfnState::Open {
        shared.thread_count += 1;
        return napi_ok;
    }
    napi_closing
}

#[no_mangle]
pub unsafe extern "C" fn napi_release_threadsafe_function(
    func: *mut Tsfn,
    mode: i32,
) -> napi_status {
    if func.is_null() {
        return napi_invalid_arg;
    }
    let t = &*func;
    let mut shared = t.lock();
    if shared.thread_count == 0 {
        return napi_invalid_arg;
    }
    shared.thread_count -= 1;
    // the last thread, or napi_tsfn_abort: no more calls; the JS thread closes it
    let abort = mode == 1;
    if (shared.thread_count == 0 || abort) && shared.state == TsfnState::Open {
        if abort {
            shared.state = TsfnState::Closing;
            t.space.notify_all();
        }
        t.send(&mut shared);
    }
    let last = shared.state == TsfnState::Closed && shared.thread_count == 0;
    drop(shared);
    if last {
        drop(Box::from_raw(func));
    }
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_ref_threadsafe_function(
    _env: napi_env,
    func: *mut Tsfn,
) -> napi_status {
    if func.is_null() {
        return napi_invalid_arg;
    }
    (*func).refed = true;
    napi_ok
}

#[no_mangle]
pub unsafe extern "C" fn napi_unref_threadsafe_function(
    _env: napi_env,
    func: *mut Tsfn,
) -> napi_status {
    if func.is_null() {
        return napi_invalid_arg;
    }
    (*func).refed = false;
    napi_ok
}

/// Dispatch one queued call on the JS thread, as Node's DispatchOne: once the queue is empty
/// and no thread holds it any more, its handles close.
unsafe fn dispatch_one(scope: &mut v8::PinScope, tsfn: *mut Tsfn) {
    let t = &mut *tsfn;
    let (data, more) = {
        let mut shared = t.lock();
        shared.dispatch_posted = false;
        let mut data = None;
        if shared.state == TsfnState::Open {
            data = shared.queue.pop_front();
            if data.is_some() && t.max_queue_size > 0 {
                t.space.notify_all();
            }
            if shared.queue.is_empty() && shared.thread_count == 0 {
                shared.state = TsfnState::Closing;
                t.space.notify_all();
                close_handles(tsfn);
            }
        } else {
            close_handles(tsfn);
        }
        let more = !shared.queue.is_empty() && shared.state == TsfnState::Open;
        if more {
            t.send(&mut shared);
        }
        (data, more)
    };
    let _ = more;
    if let Some(data) = data {
        let env = t.env;
        in_env_scope(scope, env, || {
            let s = scope_of(env);
            let js_callback = t.func.as_ref().map_or(std::ptr::null_mut(), |f| {
                to_napi(v8::Local::new(s, f).into())
            });
            let call_js = t.call_js.unwrap_or(default_call_js);
            let _ = call_addon(|| call_js(env, js_callback, t.context, data.0));
            callback_exception(env, false);
        });
    }
}

/// The env's current scope.
unsafe fn scope_of<'a>(env: napi_env) -> &'a mut v8::PinScope<'static, 'static> {
    scope(env)
}

/// Node's default call_js (a threadsafe function made without one): call the function with no
/// arguments; a failure other than a thrown exception throws ERR_NAPI_TSFN_CALL_JS.
unsafe extern "C" fn default_call_js(
    env: napi_env,
    cb: napi_value,
    _context: *mut c_void,
    _data: *mut c_void,
) {
    if env.is_null() || cb.is_null() {
        return;
    }
    let mut recv: napi_value = std::ptr::null_mut();
    napi_get_undefined(env, &mut recv);
    let status = napi_call_function(env, recv, cb, 0, std::ptr::null(), std::ptr::null_mut());
    // (Node's guard: short of a termination, a call fails only by throwing)
    let (code, message) = (c"ERR_NAPI_TSFN_CALL_JS", c"Failed to call JS callback");
    let failed = status != napi_ok && status != napi_pending_exception;
    let _ = failed.then(|| napi_throw_error(env, code.as_ptr(), message.as_ptr()));
}

/// Close the threadsafe function's handles (once): it no longer keeps the loop alive, and its
/// finalization is posted (Node's uv_close callback).
unsafe fn close_handles(t: *mut Tsfn) {
    if !std::mem::replace(&mut (*t).handles_closing, true) {
        (&*t).native.post(NativeTask::Finalize(SendPtr(t)));
    }
}

/// Node's ThreadSafeFunction::Finalize: what is still queued is handed to call_js with a NULL
/// env (to free it), the finalizer runs, and the threadsafe function is freed unless threads
/// still hold it (they free it when the last one lets go).
unsafe fn finalize_tsfn(tsfn: *mut Tsfn) {
    let t = &mut *tsfn;
    let queued: Vec<SendPtr<c_void>> = t.lock().queue.drain(..).collect();
    let call_js = t.call_js.unwrap_or(default_call_js);
    for data in queued {
        let _ = call_addon(|| {
            call_js(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                t.context,
                data.0,
            )
        });
    }
    if let Some(finalize) = t.finalize {
        let env = t.env;
        let _ = call_addon(|| finalize(env, t.finalize_data, t.context));
        callback_exception(env, true);
    }
    // ReleaseResources: closed, its function and teardown hook dropped
    t.func = None;
    let env = t.env;
    shared_env(env).tsfns.borrow_mut().retain(|&p| p != tsfn);
    teardown_of(env)
        .borrow_mut()
        .retain(|item| !matches!(*item, Teardown::Tsfn(p) if p == tsfn));
    let mut shared = t.lock();
    shared.state = TsfnState::Closed;
    t.space.notify_all();
    let held = shared.thread_count > 0;
    drop(shared);
    if !held {
        drop(Box::from_raw(tsfn));
    }
}

/// At env teardown, a threadsafe function's cleanup hook (Node's Cleanup): it is closed now;
/// its finalization runs once the hooks have (`finalize_for_teardown`).
pub(super) unsafe fn close_for_teardown(tsfn: *mut Tsfn) -> *mut Tsfn {
    let t = &mut *tsfn;
    let mut shared = t.lock();
    shared.state = TsfnState::Closing;
    t.space.notify_all();
    drop(shared);
    t.handles_closing = true;
    tsfn
}

/// At env teardown, finalize a threadsafe function closed by `close_for_teardown`: its env.
pub(super) unsafe fn finalize_for_teardown(tsfn: *mut Tsfn) -> napi_env {
    let env = (*tsfn).env;
    in_teardown_scope(env, || finalize_tsfn(tsfn));
    env
}

/// Whether `env` has threadsafe functions open (they hold it, as Node's env references).
pub(super) fn holds_env(shared: &Shared, env: napi_env) -> bool {
    shared
        .tsfns
        .borrow()
        .iter()
        .any(|&t| unsafe { (*t).env } == env)
}
