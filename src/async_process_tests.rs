use super::*;

#[test]
fn native_requests_preserve_conversion_exceptions_and_reject_invalid_and_oversized_input() {
    crate::runner::init_v8();
    let isolate = &mut v8::Isolate::new(crate::runner::framework_snapshot().create_params());
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let key = v8::String::new(scope, "processNative").unwrap();
    let function = v8::Function::new(scope, native_async_process).unwrap();
    context
        .global(scope)
        .set(scope, key.into(), function.into());
    let source = v8::String::new(
        scope,
        r#"(() => {
        const sentinel = new Error('conversion');
        const bad = {toString() {throw sentinel}};
        let caught=0;
        try {processNative(bad,'{}')} catch(error) {if(error===sentinel)caught++}
        try {processNative('poll',bad)} catch(error) {if(error===sentinel)caught++}
        const code=(name,args)=>JSON.parse(processNative(name,args)).error.code;
        return caught===2 && code('poll','{')==='ERR_INVALID_ARG_VALUE' &&
          code('x'.repeat(33),'{}')==='ENOBUFS' &&
          code('poll',' '.repeat(2097153))==='ENOBUFS';
    })()"#,
    )
    .unwrap();
    let result = v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap();
    assert!(result.is_true());
}

#[cfg(unix)]
#[test]
fn descriptor_wrapper_exposes_the_active_native_pipe_set() {
    reset();
    assert!(descriptors().is_empty());
    let args = json!({"file":"/bin/cat","args":[],"env":{},"stdio":["pipe","pipe","ignore"]});
    let result = operation("spawn", &args);
    assert!(result["value"]["pid"].is_number());
    let descriptors = descriptors();
    assert_eq!(descriptors.len(), 1);
    assert!(descriptors[0].events & libc::POLLIN != 0);
    reset();
}
