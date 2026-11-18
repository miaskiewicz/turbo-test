use super::*;
fn with_scope(f: impl FnOnce(&mut v8::PinScope)) {
    crate::runner::init_v8();
    let isolate = &mut v8::Isolate::new(crate::runner::framework_snapshot().create_params());
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    f(&mut v8::ContextScope::new(scope, context));
}
fn bind(scope: &mut v8::PinScope, name: &str, callback: impl v8::MapFnTo<v8::FunctionCallback>) {
    let key = v8::String::new(scope, name).unwrap();
    let function = v8::Function::new(scope, callback).unwrap();
    scope
        .get_current_context()
        .global(scope)
        .set(scope, key.into(), function.into());
}
fn run<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> Option<v8::Local<'s, v8::Value>> {
    let code = v8::String::new(scope, code).unwrap();
    v8::Script::compile(scope, code, None)?.run(scope)
}
#[test]
fn native_vm_rejects_invalid_anchors_and_code_conversion_exceptions() {
    with_scope(|scope| {
        bind(scope, "execute", native_run_context);
        bind(scope, "compile", native_compile_script);
        let result = run(scope, r#"(() => {
          let anchor = false, conversion = false;
          try { execute('1', null, {}, 'invalid.js', 0); } catch (error) { anchor = /Invalid VM context anchor/.test(error.message); }
          const sentinel = new Error('conversion denied');
          try { compile({ toString() { throw sentinel; } }, 'convert.js'); } catch (error) { conversion = error === sentinel; }
          return anchor && conversion;
        })()"#).unwrap();
        assert!(result.is_true());
    });
}
#[test]
fn sandbox_binding_failure_preserves_the_original_exception() {
    with_scope(|scope| {
        bind(scope, "execute", native_run_context);
        let result = run(scope, r#"(() => {
          const sentinel = new Error('binding denied');
          function anchor() { throw sentinel; }
          try { execute('1', anchor, {}, 'bindings.js', 0); } catch (error) { return error === sentinel; }
          return false;
        })()"#).unwrap();
        assert!(result.is_true());
    });
}
#[test]
fn dropped_watchdog_cancels_and_joins_without_terminating_later_execution() {
    with_scope(|scope| {
        {
            let _watchdog = Watchdog::start(scope, 60000);
        }
        assert!(!scope.is_execution_terminating());
        assert_eq!(run(scope, "6 * 7").unwrap().integer_value(scope), Some(42));
    });
}
#[test]
fn external_termination_is_not_misreported_as_a_vm_timeout() {
    with_scope(|scope| {
        bind(scope, "execute", native_run_context);
        bind(
            scope,
            "stop",
            |scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue| {
                scope.terminate_execution();
            },
        );
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut tc.init();
        assert!(run(
            scope,
            "execute('stop()', function(){}, {}, 'stopped.js', 0)"
        )
        .is_none());
        // The nested native TryCatch may consume the termination marker; it must
        // never manufacture the watchdog-specific timeout exception.
        if let Some(error) = scope.exception() {
            assert!(!error.to_rust_string_lossy(scope).contains("timed out"));
        }
        scope.cancel_terminate_execution();
        scope.reset();
        assert_eq!(run(scope, "42").unwrap().integer_value(scope), Some(42));
    });
}

#[test]
fn unavailable_vm_realms_throw_recoverable_capability_errors() {
    with_scope(|scope| {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut tc.init();
        assert!(pristine_context(scope, false).is_none());
        assert!(scope
            .exception()
            .unwrap()
            .to_rust_string_lossy(scope)
            .contains("Node framework snapshot"));
        scope.reset();
        assert!(required_context(scope, None, "VM context is unavailable").is_none());
        assert_eq!(
            scope.exception().unwrap().to_rust_string_lossy(scope),
            "Error: VM context is unavailable"
        );
        scope.reset();
        assert!(required_context(
            scope,
            None,
            "The pristine VM context snapshot is unavailable"
        )
        .is_none());
        assert!(scope
            .exception()
            .unwrap()
            .to_rust_string_lossy(scope)
            .contains("pristine VM context snapshot"));
        scope.reset();
        assert!(pristine_context(scope, true).is_some());
        assert_eq!(run(scope, "40 + 2").unwrap().integer_value(scope), Some(42));
    });
}
#[test]
fn external_watchdog_interrupts_vm_javascript_without_claiming_vm_timeout() {
    with_scope(|scope| {
        bind(scope, "execute", native_run_context);
        bind(
            scope,
            "arm",
            |scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue| {
                let watchdog = Watchdog::start(scope, 10);
                scope.set_slot(watchdog);
            },
        );
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut tc.init();
        assert!(run(
            scope,
            "execute('arm(); for (;;) {}', function(){}, {}, 'interrupted.js', 0)"
        )
        .is_none());
        assert!(
            scope.remove_slot::<Watchdog>().unwrap().finish(),
            "external watchdog actually terminated execution"
        );
        scope.cancel_terminate_execution();
        scope.reset();
        assert_eq!(run(scope, "42").unwrap().integer_value(scope), Some(42));
    });
}
