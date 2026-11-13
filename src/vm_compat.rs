//! Node-mode VM contexts, retained by JS weak-map anchors rather than a Rust global registry.
//!
//! This bounded bridge preserves ordinary own bindings and configurable builtin fallbacks.
//! Immutable native global names accept only identical non-configurable, read-only data
//! bindings. Other shadows throw ERR_VM_UNSUPPORTED_GLOBAL: a plain V8 global cannot model
//! Node's contextifying proxy for those names. Window snapshots keep their DOM index zero
//! and report VM-context unavailability rather than accidentally instantiating a DOM realm.
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

// Capture reflection functions before guest code can replace their global properties.
const ANCHOR: &str = r#"(() => {
  const global = globalThis;
  const ownKeys = Reflect.ownKeys;
  const descriptor = Object.getOwnPropertyDescriptor;
  const define = Object.defineProperty;
  const remove = Reflect.deleteProperty;
  const same = Object.is;
  const has = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
  const CapabilityError = Error;
  const stringify = String;
  const includes = Function.prototype.call.bind(Array.prototype.includes);
  const baseline = Object.create(null);
  const initial = ownKeys(global);
  for (let i = 0; i < initial.length; i++) baseline[initial[i]] = descriptor(global, initial[i]);
  const differs = (a, b) => !b || !same(a.value, b.value) || a.get !== b.get || a.set !== b.set || a.writable !== b.writable || a.enumerable !== b.enumerable || a.configurable !== b.configurable;
  let bound = [];
  let skipped = [];
  const fallback = Object.create(null);
  function immutableBinding(desc, original, key) {
    if (has(desc, 'value') && !desc.writable && !desc.configurable && same(desc.value, original.value)) return;
    const error = new CapabilityError('vm context cannot shadow immutable global ' + stringify(key));
    define(error, 'code', { value: 'ERR_VM_UNSUPPORTED_GLOBAL', configurable: true });
    throw error;
  }
  function restore(key) {
    if (has(fallback, key)) {
      define(global, key, fallback[key]);
      baseline[key] = fallback[key];
      remove(fallback, key);
    } else remove(global, key);
  }
  function setBinding(sandbox, key) {
    const desc = descriptor(sandbox, key);
    const original = descriptor(global, key);
    if (original && !original.configurable && !original.writable) {
      immutableBinding(desc, original, key);
      skipped[skipped.length] = key;
      return;
    }
    if (baseline[key] && original && !has(fallback, key)) fallback[key] = original;
    define(global, key, desc);
  }
  function inject(sandbox, keys) {
    skipped = [];
    for (let i = 0; i < bound.length; i++) if (!includes(keys, bound[i])) restore(bound[i]);
    for (let i = 0; i < keys.length; i++) setBinding(sandbox, keys[i]);
  }
  function copyOut(sandbox, key) {
    if (includes(skipped, key)) return;
    const desc = descriptor(global, key);
    if (!includes(bound, key) && !differs(desc, baseline[key])) return;
    if (baseline[key] && !has(fallback, key)) fallback[key] = desc;
    define(sandbox, key, desc);
  }
  function sync(sandbox, keys) {
    const current = ownKeys(global);
    for (let i = 0; i < keys.length; i++) if (!includes(current, keys[i])) remove(sandbox, keys[i]);
    for (let i = 0; i < current.length; i++) copyOut(sandbox, current[i]);
  }
  return function(operation, sandbox) {
    const keys = ownKeys(sandbox);
    if (operation === 0) inject(sandbox, keys);
    else sync(sandbox, keys);
    bound = ownKeys(sandbox);
  };
})()"#;

/// Add this before setting the NORMAL framework snapshot's default context. Window blobs
/// reserve index zero for their DOM context and must not call this helper.
pub fn register_pristine_context(scope: &mut v8::PinScope<'_, '_, ()>) {
    let context = v8::Context::new(scope, Default::default());
    assert_eq!(scope.add_context(context), 0);
}

fn throw_error(scope: &mut v8::PinScope, message: &str) {
    let text = v8::String::new(scope, message).unwrap();
    let error = v8::Exception::error(scope, text);
    scope.throw_exception(error);
}

/// Return an inert function whose creation context retains one pristine VM realm.
pub fn native_create_context(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if !crate::runner::vm_context_snapshot_available() {
        return throw_error(scope, "vm contexts require the Node framework snapshot; use TURBO_SNAPSHOT=0 for mixed DOM projects");
    }
    let Some(context) = v8::Context::from_snapshot(scope, 0, Default::default()) else {
        return throw_error(scope, "The pristine VM context snapshot is unavailable");
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let code = v8::String::new(scope, ANCHOR).unwrap();
    if let Some(value) = v8::Script::compile(scope, code, None).and_then(|script| script.run(scope))
    {
        rv.set(value);
    }
}

/// A cancellation channel plus a joined watchdog prevents a successful evaluation's timeout
/// from terminating a later test. Only our own timeout clears V8's termination state.
struct Watchdog {
    cancel: Sender<()>,
    thread: Option<JoinHandle<bool>>,
}
impl Watchdog {
    fn start(scope: &v8::PinScope, timeout: u64) -> Self {
        let isolate = scope.thread_safe_handle();
        let (cancel, receive) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            if receive.recv_timeout(Duration::from_millis(timeout))
                == Err(RecvTimeoutError::Timeout)
            {
                isolate.terminate_execution();
                return true;
            }
            false
        });
        Self {
            cancel,
            thread: Some(thread),
        }
    }
    fn finish(mut self) -> bool {
        let _ = self.cancel.send(());
        self.thread.take().unwrap().join().unwrap_or(false)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn transfer_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    anchor: v8::Local<'s, v8::Function>,
    sandbox: v8::Local<'s, v8::Value>,
    operation: i32,
) -> Option<v8::Local<'s, v8::Value>> {
    let receiver = v8::undefined(scope);
    let operation = v8::Integer::new(scope, operation);
    anchor.call(scope, receiver.into(), &[operation.into(), sandbox])
}

fn compile_vm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: v8::Local<'s, v8::String>,
    filename: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Script>> {
    let origin = v8::ScriptOrigin::new(
        scope, filename, 0, 0, false, 0, None, false, false, false, None,
    );
    v8::Script::compile(scope, code, Some(&origin))
}

fn timeout_error<'s>(scope: &mut v8::PinScope<'s, '_>, timeout: u64) -> v8::Local<'s, v8::Value> {
    let message = v8::String::new(
        scope,
        &format!("Script execution timed out after {timeout}ms"),
    )
    .unwrap();
    let error = v8::Exception::error(scope, message);
    let object = error.to_object(scope).unwrap();
    let key = v8::String::new(scope, "code").unwrap();
    let code = v8::String::new(scope, "ERR_SCRIPT_EXECUTION_TIMEOUT").unwrap();
    object.set(scope, key.into(), code.into());
    error
}

fn run_code<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Value>> {
    let code = args.get(0).to_string(scope)?;
    compile_vm(scope, code, args.get(3)).and_then(|script| script.run(scope))
}

/// `__ttVmRun(code, anchor, sandbox, filename, timeout)`: the JS API validates arguments.
pub fn native_run_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let Ok(anchor) = v8::Local::<v8::Function>::try_from(args.get(1)) else {
        return throw_error(scope, "Invalid VM context anchor");
    };
    let Some(context) = anchor.get_creation_context(scope) else {
        return throw_error(scope, "VM context is unavailable");
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    evaluate(scope, &args, anchor, &mut rv);
}

fn evaluate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    anchor: v8::Local<'s, v8::Function>,
    rv: &mut v8::ReturnValue,
) {
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let tc = &mut tc.init();
    let sandbox = args.get(2);
    if transfer_bindings(tc, anchor, sandbox, 0).is_none() {
        tc.rethrow();
        return;
    }
    let timeout = args.get(4).integer_value(tc).unwrap_or(0) as u64;
    let watchdog = (timeout > 0).then(|| Watchdog::start(tc, timeout));
    let result = run_code(tc, args);
    let timed_out = watchdog.is_some_and(Watchdog::finish);
    if timed_out {
        tc.cancel_terminate_execution();
    } else if tc.has_terminated() {
        tc.rethrow();
        return;
    }
    let anchor = v8::Global::new(tc, anchor);
    let sandbox = v8::Global::new(tc, sandbox);
    let result = result.map(|value| v8::Global::new(tc, value));
    finish_evaluation(tc, &anchor, &sandbox, timeout, timed_out, result, rv);
}

fn finish_evaluation(
    scope: &mut v8::PinnedRef<'_, v8::TryCatch<'_, '_, v8::HandleScope<'_>>>,
    anchor: &v8::Global<v8::Function>,
    sandbox: &v8::Global<v8::Value>,
    timeout: u64,
    timed_out: bool,
    result: Option<v8::Global<v8::Value>>,
    rv: &mut v8::ReturnValue,
) {
    let mut exception = scope.exception().map(|error| v8::Global::new(scope, error));
    scope.reset();
    if timed_out {
        let error = timeout_error(scope, timeout);
        exception = Some(v8::Global::new(scope, error));
    }
    let anchor = v8::Local::new(scope, anchor);
    let sandbox = v8::Local::new(scope, sandbox);
    transfer_bindings(scope, anchor, sandbox, 1);
    exception = exception.or_else(|| scope.exception().map(|error| v8::Global::new(scope, error)));
    scope.reset();
    if let Some(exception) = exception {
        let error = v8::Local::new(scope, exception);
        scope.throw_exception(error);
        scope.rethrow();
    } else if let Some(result) = result {
        rv.set(v8::Local::new(scope, result));
    }
}
