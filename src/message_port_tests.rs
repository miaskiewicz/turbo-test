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
fn invalid_native_receivers_throw_type_errors_and_do_not_create_ports() {
    with_scope(|scope| {
        bind(scope, "create", native_create);
        bind(scope, "id", native_id);
        bind(scope, "detached", native_detached);
        bind(scope, "detach", native_detach);
        let result = run(scope, r#"(() => {
          const calls = [() => create(-1, {}, () => {}), () => id(0), () => detached({}), () => detach({})];
          return calls.every(call => { try { call(); return false; } catch (error) { return error instanceof TypeError && /MessagePort/.test(error.message); } });
        })()"#).unwrap();
        assert!(result.is_true());
    });
}
struct Delegate;
impl v8::ValueSerializerImpl for Delegate {
    fn throw_data_clone_error<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        message: v8::Local<'s, v8::String>,
    ) {
        let error = v8::Exception::error(scope, message);
        scope.throw_exception(error);
    }
}
impl v8::ValueDeserializerImpl for Delegate {}
#[test]
fn transfer_rejects_invalid_host_objects_detached_sources_and_truncated_indices() {
    with_scope(|scope| {
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut tc.init();
        let transfers = PortTransfers::default();
        let ordinary = v8::Object::new(scope);
        let serializer = v8::ValueSerializer::new(scope, Box::new(Delegate));
        assert_eq!(transfers.write(scope, ordinary, &serializer), None);
        assert!(scope
            .exception()
            .unwrap()
            .to_rust_string_lossy(scope)
            .contains("could not be cloned"));
        scope.reset();
        let id = v8::Integer::new(scope, 1);
        let prototype = v8::Object::new(scope);
        let callback = run(scope, "(function(){})").unwrap();
        let port = make_port(scope, id.into(), prototype.into(), callback).unwrap();
        assert_eq!(transfers.add(scope, port.into()), Some(()));
        let detached = v8::Boolean::new(scope, true);
        port.set_internal_field(2, detached.into());
        assert_eq!(transfers.write(scope, port, &serializer), None);
        assert!(scope.has_caught());
        scope.reset();
        assert_eq!(transfers.validate_sources(scope), None);
        assert!(scope.has_caught());
        scope.reset();
        let reader = v8::ValueDeserializer::new(scope, Box::new(Delegate), &[]);
        assert!(transfers.read(scope, &reader).is_none());
    });
}
#[test]
fn failed_transfer_commit_preserves_other_transferables() {
    with_scope(|scope| {
        bind(scope, "create", native_create);
        bind(scope, "detach", native_detach);
        bind(
            scope,
            "clone",
            crate::structured_clone::native_structured_clone,
        );
        let result = run(scope, r#"(() => {
          const buffer = new ArrayBuffer(8);
          const port = create(1, {}, () => { throw new Error('commit denied'); });
          try { clone({port, buffer}, [port, buffer]); } catch (error) { return error.message === 'commit denied' && buffer.byteLength === 8; }
          return false;
        })()"#).unwrap();
        assert!(result.is_true());
    });
}
