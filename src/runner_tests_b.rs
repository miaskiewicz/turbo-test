//! Unit tests for runner.rs helpers in the module-loading/mock-registry/natives/run region.

use super::*;

/// A per-test scratch dir under the system temp dir (unique per test name + pid).
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join("turbo-test-unit-b")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

/// Run `f` in a fresh snapshot-booted context with the natives installed (as run_test_file
/// does), then drop every registry Global while the isolate is still alive.
fn with_context<R>(f: impl FnOnce(&mut v8::PinScope, v8::Local<v8::Object>) -> R) -> R {
    init_v8();
    let params = crate::runner::framework_snapshot().create_params();
    let isolate = &mut v8::Isolate::new(params);
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    install_natives(scope, global);
    let r = f(scope, global);
    clear_registry();
    r
}

/// Evaluate `src` as a classic script and return its completion value as a string.
fn eval(scope: &mut v8::PinScope, src: &str) -> String {
    let code = v8::String::new(scope, src).unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let v = script.run(scope).unwrap();
    v.to_rust_string_lossy(scope)
}

fn eval_value<'s>(scope: &mut v8::PinScope<'s, '_>, src: &str) -> v8::Local<'s, v8::Value> {
    let code = v8::String::new(scope, src).unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    script.run(scope).unwrap()
}

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn extract_mocks_finds_vi_and_jest_calls_in_source_order() {
    let src = r#"
jest.mock('./j', () => ({ j: 1 }));
vi.mock("./v", () => ({ f: (x) => (x + 1) }));
vi.mock(`./tpl`, factory);
"#;
    assert_eq!(
        extract_mocks(src),
        pairs(&[
            ("./j", "() => ({ j: 1 })"),
            ("./v", "() => ({ f: (x) => (x + 1) })"),
            ("./tpl", "factory"),
        ])
    );
    // vi before jest, then jest only
    assert_eq!(
        extract_mocks("vi.mock('a', f); jest.mock('b', g);"),
        pairs(&[("a", "f"), ("b", "g")])
    );
    assert_eq!(
        extract_mocks("jest.mock('only', g)"),
        pairs(&[("only", "g")])
    );
}

#[test]
fn extract_mocks_skips_dynamic_specifiers_and_stops_on_truncated_input() {
    // a non-literal specifier is skipped; the next literal one is still found
    assert_eq!(
        extract_mocks("vi.mock(name, f);\nvi.mock('./x', g);"),
        pairs(&[("./x", "g")])
    );
    // a factory-less mock as the last call yields an empty factory
    assert_eq!(
        extract_mocks("vi.mock('./auto');"),
        pairs(&[("./auto", "")])
    );
    // unterminated quote / nothing after `vi.mock(` -> stop
    assert_eq!(extract_mocks("vi.mock('./never"), vec![]);
    assert_eq!(extract_mocks("vi.mock(   "), vec![]);
    assert_eq!(extract_mocks("no mocks here"), vec![]);
    // unbalanced factory runs to end of input
    assert_eq!(
        extract_mocks("vi.mock('./u', () => (1"),
        pairs(&[("./u", "() => (1")])
    );
}

#[test]
fn extract_mocks_decodes_escaped_quotes_in_the_specifier() {
    // turbo-test#42 item 4: an escaped quote does not end the specifier; esbuild prints one
    // holding both quote kinds as a template literal
    let src = r#"vi.mock("./w\"x.js", () => 1);
vi.mock('./q"a\'b.js', () => 2);
vi.mock(`./t"a'b.js`);
vi.mock("./open\""#;
    assert_eq!(
        extract_mocks(src),
        pairs(&[
            ("./w\"x.js", "() => 1"),
            ("./q\"a'b.js", "() => 2"),
            ("./t\"a'b.js", ""),
        ])
    );
}

#[test]
fn lowered_imports_wait_for_pending_async_do_mocks() {
    // turbo-test#42 item 6: a literal (esbuild, esm_cjs) or esm_cjs's evaluated `__tt_spec`
    // asks __ttImportReady; any other computed specifier keeps Promise.resolve()
    let code = r#"const a = Promise.resolve().then(() => __toESM(require("./a\")b")));
const b = ((__tt_spec) => Promise.resolve().then(() => __toESM(require(__tt_spec))))(x);
const c = Promise.resolve().then(() => __toESM(require(dir + "/c")));
const d = Promise.resolve().then(() => __toESM(require('./d')));"#;
    assert_eq!(
        await_do_mocks_on_import(code.to_string()),
        r#"const a = __ttImportReady(__dirname, "./a\")b").then(() => __toESM(require("./a\")b")));
const b = ((__tt_spec) => __ttImportReady(__dirname, __tt_spec).then(() => __toESM(require(__tt_spec))))(x);
const c = Promise.resolve().then(() => __toESM(require(dir + "/c")));
const d = __ttImportReady(__dirname, './d').then(() => __toESM(require('./d')));"#
    );
    // nothing lowered: unchanged
    let plain = "const e = require('./e');".to_string();
    assert_eq!(await_do_mocks_on_import(plain.clone()), plain);
}

#[test]
fn extract_do_mocks_reads_only_do_mock_calls() {
    let src = "vi.mock('./a', () => 1);\nvi.doMock('./b', () => 2);\njest.doMock(\"./c\");\nvi.doMock(dyn, f);";
    assert_eq!(
        extract_do_mocks(src),
        pairs(&[("./b", "() => 2"), ("./c", "")])
    );
    // vi.doMock is not a hoisted vi.mock
    assert_eq!(extract_mocks(src), pairs(&[("./a", "() => 1")]));
}

#[test]
fn extract_mocks_factory_less_call_does_not_take_a_later_factory() {
    // turbo-test#31 item 4: the factory search ran to the next ',' anywhere in the source, so a
    // factory-less mock took the next call's factory and that call was dropped
    assert_eq!(
        extract_mocks("vi.mock('./a');\nvi.mock('./b', () => 1);"),
        pairs(&[("./a", ""), ("./b", "() => 1")])
    );
    // a test callback after a factory-less mock is not its factory either
    assert_eq!(
        extract_mocks("vi.mock('./m.js'); test('t', () => { expect(1).toBe(1) });"),
        pairs(&[("./m.js", "")])
    );
    // whitespace / a newline between the specifier and the comma still finds the factory
    assert_eq!(
        extract_mocks("jest.mock('./c'\n  , () => ({ c: 1 }));"),
        pairs(&[("./c", "() => ({ c: 1 })")])
    );
    // a trailing comma with no factory is factory-less
    assert_eq!(extract_mocks("vi.mock('./t', );"), pairs(&[("./t", "")]));
}

#[test]
fn collect_named_imports_stays_inside_the_import_statement() {
    // turbo-test#31 item 13: a default-only import took the previous import's `{...}` names
    assert!(collect_named_imports(
        "import { x } from \"./x\";\nimport d from \"./d\";\n",
        "./d"
    )
    .is_empty());
    assert!(collect_named_imports("import{x}from\"./x\";import d from\"./d\";", "./d").is_empty());
    // a name that contains `import` does not end the look-back early
    assert_eq!(
        collect_named_imports(r#"import { importer, reimport as r } from "/p";"#, "/p"),
        vec!["importer", "r"]
    );
    // a re-export from the path is a statement of its own (with or without an import before it)
    assert_eq!(
        collect_named_imports(r#"export { a, b as c } from "/p";"#, "/p"),
        vec!["a", "c"]
    );
    assert_eq!(
        collect_named_imports("import { x } from \"/x\";\nexport { a } from \"/p\";", "/p"),
        vec!["a"]
    );
    // the statement's own clause is still read, also after a default binding
    assert_eq!(
        collect_named_imports(
            "import { x } from \"./x\";\nimport d, { e as f } from \"./d\";",
            "./d"
        ),
        vec!["f"]
    );
}

#[test]
fn mock_externalizable_skips_partial_and_react_factories_only() {
    // externalized (the registry intercepts the import)
    for f in [
        "",
        "() => ({ v: 1 })",
        "() => ({ fn: (x) => (x + 1) })",
        "factory",
    ] {
        assert!(mock_externalizable(f), "{f}");
    }
    // partial mocks and React-building factories stay bundled with the test's react
    for f in [
        "async (importOriginal) => ({ ...(await importOriginal()), x: 1 })",
        "async () => ({ ...(await vi.importActual('./m')) })",
        "() => ({ C: () => <div /> })",
        "() => ({ C: () => (\n  <div />\n) })",
        "() => ({ C: () => React.createElement('div') })",
        "() => { const { jsx } = require('react/jsx-runtime'); return {}; }",
    ] {
        assert!(!mock_externalizable(f), "{f}");
    }
}

#[test]
fn collect_named_imports_reads_brace_clauses_for_the_path() {
    let text = r#"import { a, b as c, $d, _e } from "/abs/m.js";
import{x}from"/abs/m.js";
import { other } from "/abs/other.js";
"#;
    // the compact (`from"..."`) spelling is scanned first, then the spaced one
    assert_eq!(
        collect_named_imports(text, "/abs/m.js"),
        vec!["x", "a", "c", "$d", "_e"]
    );
    assert_eq!(collect_named_imports(text, "/abs/other.js"), vec!["other"]);
    assert!(collect_named_imports(text, "/abs/none.js").is_empty());
    // a default-only import with no preceding brace clause contributes nothing; non-identifier
    // parts are dropped
    assert!(collect_named_imports(r#"import D from "/p";"#, "/p").is_empty());
    assert_eq!(
        collect_named_imports(r#"import { a, , b-c, } from "/p";"#, "/p"),
        vec!["a"]
    );
}

#[test]
fn json_string_escapes_for_a_js_literal() {
    assert_eq!(json_string("plain"), "\"plain\"");
    assert_eq!(json_string("q\"b\\n\nr\rt\t"), "\"q\\\"b\\\\n\\nr\\rt\\t\"");
    assert_eq!(json_string("\u{1}\u{1f}"), "\"\\u0001\\u001f\"");
    assert_eq!(json_string("ünï ✓"), "\"ünï ✓\"");
}

#[test]
fn process_ids_define_pid_and_only_a_known_ppid() {
    let both = process_ids_js(7, Some(3));
    assert!(
        both.contains("pid: { value: 7, writable: false, enumerable: true"),
        "{both}"
    );
    assert!(
        both.contains("ppid: { value: 3, writable: true, enumerable: true"),
        "{both}"
    );
    let pid_only = process_ids_js(7, None);
    assert!(
        pid_only.contains("pid: { value: 7, writable: false"),
        "{pid_only}"
    );
    assert!(!pid_only.contains("ppid"), "{pid_only}");
}

#[test]
fn builtin_and_test_api_module_names() {
    for s in [
        "fs",
        "node:fs",
        "fs/promises",
        "node:path",
        "worker_threads",
        "inspector",
    ] {
        assert!(is_node_builtin(s), "{s}");
    }
    for s in ["lodash", "node:nope", "fs/extra", "./fs"] {
        assert!(!is_node_builtin(s), "{s}");
    }
    assert!(is_test_api_module("vitest"));
    assert!(is_test_api_module("@jest/globals"));
    assert!(!is_test_api_module("vitest/config"));
    assert!(!is_test_api_module("jest"));
}

#[test]
fn every_node_builtin_has_a_non_empty_shim() {
    // each name (bare and `node:`-prefixed) resolves to a runtime.js shim with exports — no
    // builtin imports as an empty module
    with_context(|scope, _| {
        for name in NODE_BUILTINS {
            for spec in [name.to_string(), format!("node:{name}")] {
                assert_eq!(ensure_node_builtin(scope, &spec), Some(()), "{spec}");
                let key = PathBuf::from(format!("<node:{spec}>"));
                let names = REGISTRY.with(|r| r.borrow().cjs_export_names.get(&key).cloned());
                assert!(names.is_some_and(|n| !n.is_empty()), "{spec}: no exports");
            }
        }
        // a name without a shim is refused, not served as `{}`
        assert_eq!(ensure_node_builtin(scope, "missing-shim"), None);
    });
}

#[test]
fn test_report_ok_means_nothing_failed() {
    let r = |passed, failed| TestReport {
        passed,
        failed,
        ..Default::default()
    };
    assert!(r(1, 0).ok());
    // only skipped / filtered tests: nothing failed (turbo-test#32 item 7)
    assert!(r(0, 0).ok());
    assert!(!r(3, 1).ok());
    // a file-level failure entry (e.g. "No test suite found in file …") is not ok
    let empty = TestReport {
        failures: vec!["No test suite found in file /x.test.js".into()],
        ..r(0, 0)
    };
    assert!(!empty.ok());
}

#[test]
fn node_platform_names() {
    assert_eq!(node_platform("macos"), "darwin");
    assert_eq!(node_platform("windows"), "win32");
    assert_eq!(node_platform("linux"), "linux");
    assert_eq!(node_arch("x86_64"), "x64");
    assert_eq!(node_arch("aarch64"), "arm64");
    assert_eq!(node_arch("riscv64"), "riscv64");
    assert_eq!(node_os_type("macos"), "Darwin");
    assert_eq!(node_os_type("windows"), "Windows_NT");
    assert_eq!(node_os_type("linux"), "Linux");
    assert_eq!(node_os_type("freebsd"), "freebsd");
}

#[test]
fn load_error_messages_for_instantiate_and_evaluate() {
    assert_eq!(
        instantiate_error(Some("SyntaxError: x".into())),
        "instantiate: SyntaxError: x"
    );
    assert_eq!(instantiate_error(None), "instantiate: instantiate failed");
    let no_exc = || -> String { panic!("exception text not needed") };
    assert_eq!(
        evaluate_error(
            Some("boom".into()),
            false,
            v8::ModuleStatus::Errored,
            no_exc
        ),
        Some("evaluate threw: boom".into())
    );
    assert_eq!(
        evaluate_error(None, true, v8::ModuleStatus::Evaluated, no_exc),
        None
    );
    assert_eq!(
        evaluate_error(None, true, v8::ModuleStatus::Errored, || "E".into()),
        Some("module errored: E".into())
    );
    assert_eq!(
        evaluate_error(None, false, v8::ModuleStatus::Evaluated, no_exc),
        Some("evaluate failed: status=Evaluated".into())
    );
    assert_eq!(
        evaluate_error(None, true, v8::ModuleStatus::Instantiated, no_exc),
        Some("evaluate failed: status=Instantiated".into())
    );
}

#[test]
fn lazy_stub_source_embeds_the_default_name() {
    let s = lazy_stub_factory_src("\"Add\"");
    assert!(s.contains("var DEFAULT_NAME = \"Add\";"));
    assert!(lazy_stub_factory_src("null").contains("var DEFAULT_NAME = null;"));
}

#[test]
fn code_cache_paths_are_content_addressed() {
    let a = cc_path("(function(){ a })");
    let b = cc_path("(function(){ b })");
    assert_eq!(a, cc_path("(function(){ a })"));
    assert_ne!(a, b);
    let name = a.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("cc-") && name.ends_with(".bin") && name.len() == 3 + 16 + 4);
    assert_eq!(a.parent().unwrap(), cache_dir());
}

#[test]
fn write_atomic_bytes_replaces_whole_files() {
    let d = scratch("atomic");
    let p = d.join("blob.bin");
    write_atomic_bytes(&p, b"one");
    write_atomic_bytes(&p, b"second");
    assert_eq!(std::fs::read(&p).unwrap(), b"second");
    // no temp files are left behind
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    // an unwritable target is silently skipped
    let missing = d.join("no/such/dir/blob.bin");
    write_atomic_bytes(&missing, b"x");
    assert!(!missing.exists());
}

#[test]
fn coverage_accumulate_ignores_malformed_and_out_of_scope_scripts() {
    let d = scratch("covacc");
    let good = d.join("src.js");
    std::fs::write(&good, "exports.x = 1;\n").unwrap();
    let test_file = d.join("a.test.js");
    std::fs::write(&test_file, "1;\n").unwrap();
    let spec_file = d.join("a.spec.js");
    std::fs::write(&spec_file, "1;\n").unwrap();
    let nm = d.join("node_modules/p/i.js");
    std::fs::create_dir_all(nm.parent().unwrap()).unwrap();
    std::fs::write(&nm, "1;\n").unwrap();
    let bad_utf8 = d.join("bad.js");
    std::fs::write(&bad_utf8, b"exports.x = \"\xff\";\n").unwrap();
    let fns = r#""functions":[{"functionName":"f","ranges":[{"startOffset":0,"endOffset":5,"count":1}]}]"#;
    let script = |url: &str| format!(r#"{{"url":{},{fns}}}"#, serde_json::to_string(url).unwrap());
    for payload in [
        "not json".to_string(),
        r#"{"result":{}}"#.to_string(),
        format!(
            r#"{{"result":{{"result":{}}}}}"#,
            script(&good.to_string_lossy())
        ),
        format!(
            r#"{{"result":{{"result":[{{"nourl":1}},{},{},{},{},{},{},{}]}}}}"#,
            script(""),
            script("src.js"),
            script(&d.join("missing.js").to_string_lossy()),
            script(&nm.to_string_lossy()),
            script(&test_file.to_string_lossy()),
            script(&spec_file.to_string_lossy()),
            script(&bad_utf8.to_string_lossy()),
        ),
    ] {
        coverage_accumulate(&payload);
    }
    for f in [&good, &test_file, &spec_file, &nm, &bad_utf8] {
        assert!(!crate::coverage::has_meta(f), "{}", f.display());
    }
    // an in-scope script V8 reports without functions is registered, with nothing to map
    let empty = d.join("empty.js");
    std::fs::write(&empty, "exports.y = 2;\n").unwrap();
    coverage_accumulate(&format!(
        r#"{{"result":{{"result":[{{"url":{},"functions":[]}}]}}}}"#,
        serde_json::to_string(&empty.to_string_lossy()).unwrap()
    ));
    assert!(crate::coverage::has_meta(&empty));
}

#[test]
fn run_test_file_reports_a_missing_entry() {
    let missing = Path::new("/definitely/not/here.test.js");
    let e = run_test_file(missing).unwrap_err();
    assert_eq!(e, std::fs::canonicalize(missing).unwrap_err().to_string());
}

#[test]
fn fresh_retry_restores_the_reuse_registry_even_on_panic() {
    REGISTRY.with(|r| {
        r.borrow_mut()
            .mocks
            .insert(PathBuf::from("/sentinel"), "kept".into())
    });
    // inside the fresh run the reuse registry is swapped out and FORCE_FRESH is set
    let ok = run_fresh_with(Path::new("/x"), |p| {
        assert!(FORCE_FRESH.with(|f| f.get()));
        assert!(REGISTRY.with(|r| r.borrow().mocks.is_empty()));
        REGISTRY.with(|r| {
            r.borrow_mut()
                .mocks
                .insert(PathBuf::from("/fresh-only"), String::new())
        });
        Ok(TestReport {
            passed: p.to_string_lossy().len() as u32,
            ..Default::default()
        })
    });
    assert_eq!(ok.unwrap().passed, 2);
    let panicked = run_fresh_with(Path::new("/x"), |_| panic!("loader blew up"));
    assert_eq!(panicked.unwrap_err(), "panicked (fresh retry)");
    assert!(!FORCE_FRESH.with(|f| f.get()));
    REGISTRY.with(|r| {
        let reg = r.borrow();
        assert_eq!(
            reg.mocks.get(Path::new("/sentinel")).map(String::as_str),
            Some("kept")
        );
        assert!(!reg.mocks.contains_key(Path::new("/fresh-only")));
    });
    clear_registry();
}

#[test]
fn end_worker_reuse_and_forget_registry_drop_the_persistent_isolate() {
    // no persistent isolate: both are no-ops that leave the flags reset
    end_worker_reuse();
    forget_registry();
    assert!(REUSE_ISO.with(|i| i.borrow().is_none()));

    init_v8();
    for forget in [false, true] {
        let params = framework_snapshot().create_params();
        let mut isolate = v8::Isolate::new(params);
        {
            v8::scope!(let scope, &mut isolate);
            let ctx = v8::Context::new(scope, Default::default());
            let g = v8::Global::new(scope, ctx);
            REUSE_CTX.with(|c| *c.borrow_mut() = Some(g));
        }
        REUSE_ISO.with(|i| *i.borrow_mut() = Some(isolate));
        DOM_INSTALLED.with(|d| d.set(true));
        SETUP_MOCKS.with(|s| s.borrow_mut().insert(PathBuf::from("/setup")));
        if forget {
            forget_registry();
        } else {
            end_worker_reuse();
        }
        assert!(REUSE_ISO.with(|i| i.borrow().is_none()));
        assert!(REUSE_CTX.with(|c| c.borrow().is_none()));
        assert!(!DOM_INSTALLED.with(|d| d.get()));
        assert!(SETUP_MOCKS.with(|s| s.borrow().is_empty()));
    }
}

#[test]
fn drive_loop_gives_up_after_its_turn_budget() {
    let err = with_context(|scope, global| {
        eval(
            scope,
            "globalThis.__stepMacro = () => true; globalThis.__tt.run = () => new Promise(() => {}); 1",
        );
        drive_tests_bounded(scope, global, Path::new("/t.test.js"), 5).unwrap_err()
    });
    assert_eq!(err, "event loop did not settle (possible infinite timers)");
}

/// turbo-dom#112: on a large tree the DOM binding defers ownership work across mutating calls;
/// the drive loop settles it (`flush_ownership`) once the task's microtasks have drained, so a
/// subtree the task detached is collectable before the next task.
#[test]
fn drive_loop_settles_deferred_dom_ownership_after_each_task() {
    with_context(|scope, global| {
        turbo_dom_browser_env::browser_env::install(scope);
        eval(
            scope,
            "(() => {
               const host = document.createElement('div');
               document.body.appendChild(host);
               for (let i = 0; i < 1500; i++) host.appendChild(document.createElement('p'));
               const gone = document.createElement('section');
               gone.id = 'gone';
               host.appendChild(gone);
               globalThis.__stepMacro = () => false;
               globalThis.__tt.run = () => {
                 document.getElementById('gone').remove();
                 return new Promise(() => {});
               };
               return 1;
             })()",
        );
        let gone = {
            v8::scope!(let inner, scope);
            let gone = eval_value(inner, "document.getElementById('gone')");
            v8::Weak::new(inner, v8::Local::<v8::Object>::try_from(gone).unwrap())
        };
        // The task removes #gone (one call, deferred on a 1500-node tree) and never settles.
        let _ = drive_tests_bounded(scope, global, Path::new("/t.test.js"), 5);
        for _ in 0..3 {
            scope.low_memory_notification();
        }
        assert!(
            gone.is_empty(),
            "the detached wrapper was collected after the task"
        );
        turbo_dom_browser_env::browser_env::reset();
    });
}

#[test]
fn call_global_bool_tolerates_missing_and_throwing_hooks() {
    with_context(|scope, global| {
        eval(
            scope,
            "globalThis.__yes = () => 1; globalThis.__notFn = 3; globalThis.__throws = () => { throw new Error('x'); }; 1",
        );
        assert!(call_global_bool(scope, global, "__yes"));
        assert!(!call_global_bool(scope, global, "__notFn"));
        assert!(!call_global_bool(scope, global, "__absent"));
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        assert!(!call_global_bool(tc, global, "__throws"));
        assert!(tc.has_caught());
    });
}

#[test]
fn object_keys_falls_back_to_none_when_the_helper_is_broken() {
    with_context(|scope, global| {
        let obj = eval_value(scope, "({ a: 1, b: 2 })");
        assert_eq!(object_keys(scope, global, obj), vec!["a", "b"]);
        eval(
            scope,
            "globalThis.__realKeys = globalThis.__keys; globalThis.__keys = () => 5; 1",
        );
        assert!(object_keys(scope, global, obj).is_empty());
        eval(scope, "globalThis.__keys = undefined; 1");
        assert!(object_keys(scope, global, obj).is_empty());
        eval(
            scope,
            "globalThis.__keys = () => { throw new Error('k'); }; 1",
        );
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        assert!(object_keys(tc, global, obj).is_empty());
        assert!(tc.has_caught());
    });
}

#[test]
fn is_thenable_requires_a_callable_then() {
    with_context(|scope, _| {
        for (src, want) in [
            ("({ then() {} })", true),
            ("({ then: 1 })", false),
            ("({})", false),
            ("5", false),
            ("Promise.resolve(1)", true),
        ] {
            let v = eval_value(scope, src);
            assert_eq!(is_thenable(scope, v), want, "{src}");
        }
    });
}

#[test]
fn pending_mock_queue_keeps_unresolved_entries_and_registers_resolved_ones() {
    let d = scratch("drain");
    std::fs::write(d.join("real.cjs"), "exports.v = 1;\n").unwrap();
    let real = d.join("real.cjs");
    with_context(|scope, global| {
        eval(
            scope,
            "globalThis.__pendingMocks = [5, { spec: './real.cjs', exports: undefined }, \
             { spec: './p.cjs', exports: new Promise(() => {}) }, \
             { spec: './t.cjs', exports: { then() {} } }, \
             { spec: './unresolvable.cjs', exports: { v: 0 } }, \
             { spec: './real.cjs', exports: { v: 'mocked' } }]; 1",
        );
        drain_pending_mocks(scope, &d, false);
        assert_eq!(eval(scope, "globalThis.__pendingMocks.length"), "2");
        assert_eq!(
            eval(scope, "globalThis.__pendingMocks.map((m) => m.spec).join()"),
            "./p.cjs,./t.cjs"
        );
        assert!(REGISTRY.with(|r| r.borrow().mocks.contains_key(&real)));
        let g = REGISTRY
            .with(|r| r.borrow().cjs_exports.get(&real).cloned())
            .unwrap();
        let v = v8::Local::new(scope, &g);
        let o = v.to_object(scope).unwrap();
        let k = v8::String::new(scope, "v").unwrap();
        let got = o.get(scope, k.into()).unwrap().to_rust_string_lossy(scope);
        assert_eq!(got, "mocked");

        // a queue entry whose `spec` getter throws is skipped (the exception stays pending)
        eval(
            scope,
            "globalThis.__pendingMocks = [{ get spec() { throw new Error('spec'); } }]; 1",
        );
        {
            let tc = std::pin::pin!(v8::TryCatch::new(scope));
            let tc = &mut tc.init();
            drain_pending_mocks(tc, &d, false);
            assert!(tc.has_caught());
        }
        // no queue at all: nothing to do
        eval(scope, "globalThis.__pendingMocks = undefined; 1");
        drain_pending_mocks(scope, &d, false);
        assert_eq!(
            eval(scope, "String(globalThis.__pendingMocks)"),
            "undefined"
        );
        let _ = global;
    });
}

#[test]
fn resolve_pending_mocks_tolerates_a_missing_or_throwing_resolver() {
    with_context(|scope, _| {
        eval(scope, "globalThis.__resolvePendingMocks = undefined; 1");
        resolve_pending_mocks(scope);
        assert!(!RESOLVING_MOCKS.with(|f| f.get()));
        eval(
            scope,
            "globalThis.__resolvePendingMocks = () => { throw new Error('r'); }; 1",
        );
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        resolve_pending_mocks(tc);
        assert!(tc.has_caught());
        assert!(!RESOLVING_MOCKS.with(|f| f.get()));
    });
}

#[test]
fn a_synthetic_module_without_exports_fails_evaluation_instead_of_aborting() {
    // turbo-test#79: a synthetic module whose registry entry is gone (a reused isolate's reset)
    // errors with a message; empty evaluation steps without an exception abort V8.
    with_context(|scope, _| {
        let module = make_synthetic(scope, &["x".to_string()]).unwrap();
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        assert_eq!(module.instantiate_module(tc, resolve_callback), Some(true));
        let _ = module.evaluate(tc);
        assert_eq!(module.get_status(), v8::ModuleStatus::Errored);
        assert_eq!(
            module.get_exception().to_rust_string_lossy(tc),
            "Error: turbo-test: an imported module was unloaded before it was evaluated"
        );
    });
}

#[test]
fn the_esm_mock_prepass_falls_back_and_fails_loudly_when_nothing_compiles() {
    // turbo-test#79 item 1: a prepass that cannot compile was skipped silently, so the file ran
    // against the real modules.
    with_context(|scope, _| {
        assert_eq!(
            run_prepass(
                scope,
                "globalThis.__p = 'first';",
                "globalThis.__p = 'fallback';"
            ),
            Ok(())
        );
        assert_eq!(eval(scope, "globalThis.__p"), "first");
        // the first throws (or does not compile): the fallback runs
        assert_eq!(
            run_prepass(scope, "throw 1;", "globalThis.__p = 'after throw';"),
            Ok(())
        );
        assert_eq!(eval(scope, "globalThis.__p"), "after throw");
        assert_eq!(
            run_prepass(
                scope,
                "(n: string) => n",
                "globalThis.__p = 'after syntax';"
            ),
            Ok(())
        );
        assert_eq!(eval(scope, "globalThis.__p"), "after syntax");
        // a fallback that throws is the factories' business (their error stand-ins)
        assert_eq!(run_prepass(scope, "throw 1;", "throw 2;"), Ok(()));
        // nothing compiles: the file fails with the SyntaxError
        assert_eq!(
            run_prepass(
                scope,
                "(n: string) => n",
                "vi.mock('./d', (n: string) => n);"
            ),
            Err("SyntaxError: Unexpected token ':' (in the file's vi.mock factories)".to_string())
        );
    });
}

#[test]
fn extract_mocks_scans_the_text_of_source_that_does_not_parse() {
    // the AST is not available: the text scan still skips a non-literal specifier
    assert_eq!(
        extract_mocks("vi.mock(name, f);\nvi.mock('./x', g);\n("),
        pairs(&[("./x", "g")])
    );
}

/// turbo-test#207: the default transform cache is per build (`turbo-test-cache/<version>-<build
/// id>`), so an entry another turbo-test build wrote is never served as a hit.
#[test]
fn the_default_cache_dir_is_per_build() {
    let tmp = Path::new("/t");
    let d = default_cache_dir(tmp);
    assert_eq!(d.parent(), Some(Path::new("/t/turbo-test-cache")));
    let leaf = d.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        leaf,
        format!("{}-{}", env!("CARGO_PKG_VERSION"), env!("TT_BUILD_ID"))
    );
    assert_eq!(env!("TT_BUILD_ID").len(), 16);
}

/// A TURBO_CACHE_DIR is a root shared across builds (a persisted CI cache, an upgrade): this
/// build's artifacts, startup snapshots included, live in its own `<version>-<build id>` folder.
#[test]
fn a_custom_cache_root_is_per_build_too() {
    let d = build_cache_dir(Path::new("/ci/cache"));
    assert_eq!(d, Path::new("/ci/cache").join(BUILD_TAG));
    assert_eq!(
        BUILD_TAG,
        format!("{}-{}", env!("CARGO_PKG_VERSION"), env!("TT_BUILD_ID"))
    );
    assert_eq!(
        default_cache_dir(Path::new("/t")),
        build_cache_dir(Path::new("/t/turbo-test-cache"))
    );
    // every artifact path (here a startup snapshot's) is under the build's folder
    let blob = saved_blob_path("framework", 1);
    assert_eq!(
        blob.parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy(),
        BUILD_TAG
    );
}

/// turbo-test#209: a quote or backtick inside a comment does not open a string for the
/// shared-let rewrite. payroll-app's RegistrationFields.test.tsx has a JSX comment with
/// `undefined` in backticks above its `useCurrentEntity: () => currentEntity` factory; the
/// rewrite took the rest of the file for a template literal and left `currentEntity` alone
/// (`ReferenceError: currentEntity is not defined`; vitest 4.1.12: 21 passed).
#[test]
fn the_shared_let_rewrite_skips_comments() {
    let names = ["currentEntity".to_string()];
    let code = "{/* emits `undefined` here */}\n// don't\nconst f = () => currentEntity; /* it's */ g(currentEntity);\n";
    let out = rewrite_shared_lets(code, &names);
    assert!(
        out.ends_with("{/* emits `undefined` here */}\n// don't\nconst f = () => globalThis.__ms.currentEntity; /* it's */ g(globalThis.__ms.currentEntity);\n"),
        "{out}"
    );
    // a comment's own text is left as is
    let out = rewrite_shared_lets(
        "// currentEntity\n/* currentEntity */ currentEntity",
        &names,
    );
    assert!(
        out.ends_with("// currentEntity\n/* currentEntity */ globalThis.__ms.currentEntity"),
        "{out}"
    );
    // `/` alone (division) is not a comment
    let out = rewrite_shared_lets("a / currentEntity / 2", &names);
    assert!(
        out.ends_with("a / globalThis.__ms.currentEntity / 2"),
        "{out}"
    );
}

// turbo-test#228 at a worker's end: a file that fails to load after compiling a module leaves no
// pending bytecode cache behind (its scripts belong to the isolate it ran in, which is gone once
// run_test_file returns; the thread-local destructor would drop them after it).
#[test]
fn a_file_that_fails_to_load_leaves_no_pending_code_caches() {
    let d = scratch("pending-cc-load-fail");
    std::fs::write(d.join("package.json"), r#"{"name":"pendingcc"}"#).unwrap();
    std::fs::write(
        d.join("dep.js"),
        format!(
            "module.exports = function dep() {{ return {}; }};\n",
            std::process::id()
        ),
    )
    .unwrap();
    let entry = d.join("z.test.js");
    std::fs::write(
        &entry,
        "import './dep.js';\nthrow new Error('boom at load');\n",
    )
    .unwrap();
    let e = std::thread::spawn(move || {
        let r = run_test_file(&entry);
        let pending = PENDING_CODE_CACHES.with(|p| p.borrow().len());
        (r, pending)
    })
    .join()
    .expect("the worker thread exits cleanly");
    assert!(e.0.unwrap_err().contains("boom at load"));
    assert_eq!(e.1, 0);
}

/// turbo-dom#249: the worker blob carries the framework context (window-free, for node files)
/// and a window context a DOM file opens with the document already bootstrapped; its install
/// rehydrates the snapshotted window.
#[test]
fn framework_blob_opens_window_and_window_free_contexts() {
    init_v8();
    let snap = framework_snapshot();
    let mut isolate = v8::Isolate::new(snap.create_params());
    for (dom, expected) in [(false, "undefined,function"), (true, "object,function")] {
        v8::scope!(let scope, &mut isolate);
        let context = snap.new_context(scope, dom);
        let scope = &mut v8::ContextScope::new(scope, context);
        let got = eval_value(
            scope,
            "[typeof document, typeof globalThis.__ttApplyConfig].join()",
        )
        .to_rust_string_lossy(scope);
        assert_eq!(got, expected, "dom={dom}");
        if dom {
            turbo_dom_browser_env::browser_env::install(scope);
            let got = eval_value(
                scope,
                "document.body.appendChild(document.createElement('p')).tagName",
            )
            .to_rust_string_lossy(scope);
            assert_eq!(got, "P");
            turbo_dom_browser_env::browser_env::reset();
        }
    }
}

#[test]
fn the_window_snapshot_is_built_only_for_enough_dom_files() {
    assert!(!window_snapshot_wanted(0));
    assert!(!window_snapshot_wanted(WINDOW_SNAPSHOT_MIN_DOM_FILES - 1));
    assert!(window_snapshot_wanted(WINDOW_SNAPSHOT_MIN_DOM_FILES));
}

/// The window snapshot is the default; `TURBO_SNAPSHOT=0` turns it off, and so do custom V8
/// flags (`TURBO_V8_FLAGS`): code deserialized from a blob can conflict with them
/// (`--interpreted-frames-native-stack` aborts V8 on the window snapshot).
#[test]
fn custom_v8_flags_or_turbo_snapshot_0_keep_the_framework_only_blob() {
    assert!(window_snapshot_allowed(None, None));
    assert!(window_snapshot_allowed(Some("1"), None));
    assert!(!window_snapshot_allowed(Some("0"), None));
    assert!(!window_snapshot_allowed(
        None,
        Some("--interpreted-frames-native-stack")
    ));
    assert!(window_snapshot_allowed(None, Some("")));
}

/// A saved startup blob is served only intact: the magic, its length and its content hash
/// must match, so a truncated or corrupted file (never handed to V8, which would abort)
/// reads as absent and is rebuilt.
#[test]
fn saved_startup_blobs_are_served_only_intact() {
    let dir = std::env::temp_dir().join(format!("tt-blob-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("snapshot.bin");
    assert!(read_saved_blob(&path).is_none(), "missing");
    save_blob(&path, b"blob bytes");
    assert_eq!(read_saved_blob(&path), Some(&b"blob bytes"[..]));
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    assert!(read_saved_blob(&path).is_none(), "corrupted");
    bytes.truncate(last);
    std::fs::write(&path, &bytes).unwrap();
    assert!(read_saved_blob(&path).is_none(), "truncated");
    std::fs::write(&path, b"short").unwrap();
    assert!(read_saved_blob(&path).is_none(), "no header");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A saved blob is keyed on everything it was built from that the build id does not cover:
/// its kind, the reference table's fingerprint, chai's text and the V8 flags.
#[test]
fn saved_blob_keys_follow_their_inputs() {
    let base = blob_key_from("window", 1, "chai", "");
    assert_eq!(base, blob_key_from("window", 1, "chai", ""));
    assert_ne!(base, blob_key_from("framework", 1, "chai", ""));
    assert_ne!(base, blob_key_from("window", 2, "chai", ""));
    assert_ne!(base, blob_key_from("window", 1, "chai2", ""));
    assert_ne!(base, blob_key_from("window", 1, "chai", "--predictable"));
}

/// Every worker isolate runs single-threaded by default: with one isolate per core, V8's
/// background compile and GC helpers only contend with the other workers (payroll: about 9%
/// wall and CPU); and with a 32 MB young generation, so a test file's short-lived objects take
/// fewer scavenges (ui-design-components: about 8% wall). `TURBO_V8_FLAGS` replaces the
/// defaults; set empty, it runs none. The startup blob is keyed on the effective flags.
#[test]
fn v8_runs_single_threaded_unless_turbo_v8_flags_says_otherwise() {
    assert_eq!(v8_flags(None), "--single-threaded --min-semi-space-size=32");
    assert_eq!(v8_flags(Some("")), "");
    assert_eq!(v8_flags(Some("--max-lazy")), "--max-lazy");
}

#[test]
fn native_binary_boundaries_reject_non_views_and_malformed_clone_frames() {
    with_context(|scope, _| {
        assert_eq!(
            eval(
                scope,
                r#"JSON.stringify([__ttGzip, __ttV8Deserialize].map(fn => { try { fn({}); return false; } catch(e) { return e.message === 'Expected a byte view'; } }))"#
            ),
            "[true,true]"
        );
        assert_eq!(
            eval(
                scope,
                r#"JSON.stringify([[],[255],[255,15],[255,15,255],[0,0]].map(bytes=>{try{__ttV8Deserialize(new Uint8Array(bytes));return false;}catch(e){return e instanceof Error;}}))"#
            ),
            "[true,true,true,true,true]"
        );
    });
}

#[test]
fn native_zlib_handles_return_bytes_and_throw_node_errors() {
    with_context(|scope, _| {
        assert_eq!(
            eval(
                scope,
                r#"(() => {
                  const fail = (fn) => { try { fn(); return 'none'; } catch (e) { return [e.constructor.name, e.message, e.code, e.errno].join('|'); } };
                  const enc = __ttZlibOpen(5, 9), dec = __ttZlibOpen(6, 6);
                  const packed = __ttZlibWrite(enc, new TextEncoder().encode('native zlib'), 4);
                  const text = new TextDecoder().decode(__ttZlibWrite(dec, packed, 4));
                  const bad = __ttZlibOpen(2, 6), one = __ttZlibOpen(1, 6), all = __ttZlibOpen(3, 6);
                  const seen = [packed instanceof Uint8Array, text,
                    fail(() => __ttZlibOpen(42, 6)),
                    fail(() => __ttZlibOpen(9, 6, new Uint32Array([7, 0]))),
                    new TextDecoder().decode(__ttZlibWrite(__ttZlibOpen(8, 6), __ttZlibWrite(__ttZlibOpen(9, 6, new Uint32Array([1, 5, 2, 18])), new TextEncoder().encode('brotli'), 4), 4)),
                    fail(() => __ttZlibWrite(__ttZlibOpen(8, 6), new TextEncoder().encode('nope nope nope'), 0)),
                    fail(() => __ttZlibWrite(enc, {}, 0)),
                    fail(() => __ttZlibWrite(enc, new Uint8Array(1), 0)),
                    fail(() => __ttZlibWrite(bad, new Uint8Array([255, 255, 255, 255]), 0)),
                    fail(() => __ttZlibWrite(__ttZlibOpen(4, 6), new Uint8Array(0), 4))];
                  __ttZlibClose(one);
                  __ttZlibClose();
                  seen.push(fail(() => __ttZlibWrite(all, new Uint8Array(0), 0)));
                  return JSON.stringify(seen);
                })()"#
            ),
            r#"[true,"native zlib","Error|invalid zlib mode|ERR_INVALID_ARG_VALUE|","Error|Initialization failed|ERR_ZLIB_INITIALIZATION_FAILED|","brotli","Error|Decompression failed|ERR__ERROR_FORMAT_PADDING_2|-15","Error|Expected a byte view||","Error|zlib binding closed|ERR_ZLIB_BINDING_CLOSED|","Error|invalid compressed data|Z_DATA_ERROR|-3","Error|unexpected end of file|Z_BUF_ERROR|-5","Error|zlib binding closed|ERR_ZLIB_BINDING_CLOSED|"]"#
        );
    });
}

#[test]
fn poisoned_resolver_flags_preserve_original_exception_for_every_native_entry() {
    with_context(|scope, _| {
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const original=process.env; const error=new Error('env unavailable'); process.env=new Proxy({}, {get(){throw error;}}); const results=[__nativeRequire,__ttRegisterMock,__ttImportActual,__ttResolveSpec,__ttUnmock,__ttRequireResolve].map(fn=>{try{fn('unused');return false;}catch(e){return e===error;}}); process.env=original; return JSON.stringify(results);})()"#
            ),
            "[true,true,true,true,true,true]"
        );
    });
}

#[test]
fn absent_process_environment_uses_default_runtime_flags() {
    with_context(|scope, _| {
        eval(
            scope,
            "globalThis.__savedProcess = process; globalThis.process = null",
        );
        let flags = resolver_runtime_flags(scope);
        assert!(flags.is_some());
        assert!(!crate::pnpm_ancestor::enabled());
        assert!(!crate::vue::enabled(Path::new("/")));
        drop(flags);
        eval(
            scope,
            "globalThis.process = __savedProcess; delete globalThis.__savedProcess",
        );
    });
}

#[test]
fn invalid_prepass_never_runs_module_body_or_requires() {
    with_context(|scope, _| {
        eval(scope, "globalThis.__bodyRan = false");
        run_prepass_wrapper(
            scope,
            Path::new("/prepass.cjs"),
            Path::new("/"),
            "const = broken;",
        );
        assert_eq!(eval(scope, "__bodyRan"), "false");
        eval(scope, "globalThis.__mkRequire = undefined");
        run_prepass_wrapper(
            scope,
            Path::new("/prepass.cjs"),
            Path::new("/"),
            "globalThis.__bodyRan = true",
        );
        assert_eq!(eval(scope, "__bodyRan"), "false");
    });
}

#[test]
fn pending_mock_queue_rejects_nonobjects_and_propagates_spec_getter_errors() {
    with_context(|scope, _| {
        let arr = eval_value(
            scope,
            "[null, { get spec() { throw new Error('bad spec'); } }]",
        );
        let arr = v8::Local::<v8::Array>::try_from(arr).unwrap();
        let spec = v8::String::new(scope, "spec").unwrap();
        let exports = v8::String::new(scope, "exports").unwrap();
        let mut retained = Vec::new();
        drain_one_pending_mock(scope, arr, 0, spec, exports, Path::new("/"), &mut retained);
        assert!(retained.is_empty());
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        drain_one_pending_mock(tc, arr, 1, spec, exports, Path::new("/"), &mut retained);
        assert!(tc.has_caught());
        assert!(retained.is_empty());
    });
}

#[test]
fn primitive_null_mock_exports_do_not_overwrite_existing_cached_value() {
    with_context(|scope, _| {
        let path = Path::new("/mock-null.cjs");
        let object = eval_value(scope, "({sentinel: 7})");
        let cached = v8::Global::new(scope, object);
        REGISTRY.with(|r| {
            r.borrow_mut().cjs_exports.insert(path.into(), cached);
        });
        let null = v8::null(scope).into();
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let tc = &mut tc.init();
        assert!(!remock_in_place(tc, path, null, &[]));
        let cached = REGISTRY.with(|r| r.borrow().cjs_exports.get(path).unwrap().clone());
        assert!(v8::Local::new(tc, &cached).strict_equals(object));
    });
}

#[test]
fn module_variant_names_include_real_exports_and_node_default_only_when_required() {
    with_context(|scope, _| {
        let exports = eval_value(scope, "({answer:42,default:9})");
        let mut star = variant_names(scope, exports, Variant::Star).unwrap();
        star.sort();
        assert_eq!(star, vec!["answer"]);
        let mut node = variant_names(scope, exports, Variant::Node).unwrap();
        node.sort();
        assert!(node.contains(&"answer".to_owned()));
        assert!(node.contains(&"default".to_owned()));
    });
}

#[test]
fn forced_clone_failure_preserves_existing_source_and_destination() {
    let dir = scratch("force-copy");
    let source = dir.join("source");
    let dest = dir.join("dest");
    std::fs::write(&source, b"source bytes").unwrap();
    std::fs::write(&dest, b"keep destination").unwrap();
    let error = fs_copy_bytes(source.to_str().unwrap(), dest.to_str().unwrap(), 4).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ENOSYS));
    assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
    assert_eq!(std::fs::read(&dest).unwrap(), b"keep destination");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn copy_to_a_hardlink_does_not_truncate_either_alias() {
    let dir = scratch("hardlink-copy");
    let source = dir.join("source");
    let dest = dir.join("alias");
    std::fs::write(&source, b"preserved alias bytes").unwrap();
    std::fs::hard_link(&source, &dest).unwrap();
    fs_copy_bytes(source.to_str().unwrap(), dest.to_str().unwrap(), 0).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"preserved alias bytes");
    assert_eq!(std::fs::read(&dest).unwrap(), b"preserved alias bytes");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn actual_esm_cache_survives_mock_restoration_and_isolate_modules_snapshot() {
    let dir = scratch("esm-snapshot-coverage");
    let target = dir.join("actual.mjs");
    let dependency = dir.join("node_modules/external/index.mjs");
    std::fs::create_dir_all(dependency.parent().unwrap()).unwrap();
    std::fs::write(&target, "export const answer = 42;").unwrap();
    std::fs::write(&dependency, "export const external = true;").unwrap();
    with_context(|scope, _| {
        let original = load_graph(scope, &target).unwrap();
        let referrer = load_graph(scope, &dependency).unwrap();
        let original_hash = original.get_identity_hash();
        REGISTRY.with(|r| {
            r.borrow_mut()
                .mocks
                .insert(target.clone(), "mock marker".into());
        });
        let real = real_esm_for(scope, referrer, &target).unwrap();
        let cached = real_esm_for(scope, referrer, &target).unwrap();
        assert_eq!(real.get_identity_hash(), cached.get_identity_hash());
        let restored = REGISTRY.with(|r| r.borrow().esm_by_path[&target].clone());
        assert_eq!(
            v8::Local::new(scope, &restored).get_identity_hash(),
            original_hash
        );
        let undefined: v8::Local<v8::Value> = v8::undefined(scope).into();
        let exports = v8::Global::new(scope, undefined);
        REGISTRY.with(|r| {
            let mut registry = r.borrow_mut();
            registry.mocks.remove(&target);
            registry.cjs_exports.insert(target.clone(), exports);
        });
        eval(scope, "__ttSaveModules()");
        assert!(!REGISTRY.with(|r| r.borrow().esm_by_path.contains_key(&target)));
        eval(scope, "__ttRestoreModules()");
        let restored = REGISTRY.with(|r| r.borrow().esm_by_path[&target].clone());
        assert_eq!(
            v8::Local::new(scope, &restored).get_identity_hash(),
            original_hash
        );
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn temporary_directory_collisions_retry_without_modifying_existing_directories() {
    let dir = scratch("temp-collision");
    let prefix = dir.join("private-").to_str().unwrap().to_owned();
    let existing = format!("{prefix}AAAAAA");
    std::fs::create_dir(&existing).unwrap();
    let marker = Path::new(&existing).join("marker");
    std::fs::write(&marker, b"existing contents").unwrap();
    let mut calls = 0;
    let created = fs_temp_directory_with(&prefix, || {
        calls += 1;
        Ok(if calls == 1 { "AAAAAA" } else { "BBBBBB" }.into())
    })
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(created, format!("{prefix}BBBBBB"));
    assert!(Path::new(&created).is_dir());
    assert_eq!(std::fs::read(&marker).unwrap(), b"existing contents");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn temporary_directory_collision_exhaustion_and_entropy_failure_are_real_errors() {
    let dir = scratch("temp-exhaustion");
    let prefix = dir.join("private-").to_str().unwrap().to_owned();
    let existing = format!("{prefix}AAAAAA");
    std::fs::create_dir(&existing).unwrap();
    let mut calls = 0;
    let (error, path) = fs_temp_directory_with(&prefix, || {
        calls += 1;
        Ok("AAAAAA".into())
    })
    .unwrap_err();
    assert_eq!(calls, 128);
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(path, prefix);
    assert!(Path::new(&existing).is_dir());
    let (error, path) = fs_temp_directory_with(&prefix, || {
        Err(std::io::Error::other("entropy unavailable"))
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "entropy unavailable");
    assert_eq!(path, prefix);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn linking_an_externalized_cjs_dependency_loads_its_actual_exports_lazily() {
    let dir = scratch("lazy-cjs-link");
    let entry = dir.join("setup.mjs");
    let dependency = dir.join("dependency.cjs");
    std::fs::write(&dependency, "module.exports = { answer: 42 };").unwrap();
    with_context(|scope, _| {
        let code = v8::String::new(
            scope,
            "import value from './dependency.cjs'; export default value;",
        )
        .unwrap();
        let name = v8::String::new(scope, entry.to_str().unwrap()).unwrap();
        let origin = module_origin(scope, name);
        let mut source = v8::script_compiler::Source::new(code, Some(&origin));
        let module = compile_graph_module(scope, &entry, &mut source).unwrap();
        REGISTRY.with(|r| {
            r.borrow_mut()
                .path_by_hash
                .insert(module.get_identity_hash().get(), entry.clone());
        });
        assert_eq!(
            module.instantiate_module(scope, resolve_callback),
            Some(true)
        );
        let exports = REGISTRY
            .with(|r| r.borrow().cjs_exports.get(&dependency).cloned())
            .unwrap();
        let exports = v8::Local::new(scope, &exports).to_object(scope).unwrap();
        let key = v8::String::new(scope, "answer").unwrap();
        assert_eq!(
            exports.get(scope, key.into()).unwrap().integer_value(scope),
            Some(42)
        );
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unreferenced_tcp_listener_does_not_keep_an_idle_loop_polling() {
    with_context(|scope, global| {
        eval(scope, "globalThis.__idleServer = __mkRequire('/')('node:net').createServer(); __idleServer.listen(0); __idleServer.unref();");
        assert!(crate::tcp_compat::has_pending_io());
        assert!(!pace_tcp_io(scope, global));
        eval(
            scope,
            "__idleServer.close(); __tcpReset(); delete globalThis.__idleServer;",
        );
        assert!(!crate::tcp_compat::has_pending_io());
    });
}
