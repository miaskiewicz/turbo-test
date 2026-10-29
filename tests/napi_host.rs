//! The Node-API host (src/napi_host.rs, src/napi_host_async.rs) end to end: C addons compiled
//! at test time are `require()`d from specs run by the real CLI, and the specs assert what the
//! `napi_*` / `node_api_*` functions produced (values, statuses, thrown errors, pending
//! exceptions, last-error info), plus the module-init paths (exports vs returned value,
//! throwing init, an import the host does not provide, a module registered with
//! napi_module_register, a declared module API version, missing init symbol, a file that
//! fails to dlopen, a nested require() during init, init serialized across workers),
//! re-entrant callbacks, several addons loaded side by side, references, napi_wrap and
//! finalizers (on collection and at env teardown), cleanup hooks, async work and threadsafe
//! functions on the event loop. The expected values are Node-API's: the same specs pass against
//! Node 26 with the same fixtures, except for host-only behaviour (reference ids, host load
//! messages, cases where Node aborts, crashes or deadlocks). docs/node-api.md has the inventory.
#![cfg(unix)]

mod common;
mod napi_support;

use common::{json_report, turbo_test, Project};
use napi_support::{compile_addon, compile_addon_as, spec};

/// A throwaway project with esbuild linked in (the module runner's CJS path is what gives specs
/// a `require()`, as in a real vitest project).
fn project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p
}

/// Run the project's specs and assert every test passed (`expected` of them).
fn assert_all_pass(p: &Project, expected: u64) {
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], expected, "{}", r.all());
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn every_implemented_entrypoint_behaves() {
    let p = project("napi-full");
    compile_addon(&p, "full");
    p.file("full.test.ts", spec("full.test.ts"));
    assert_all_pass(&p, 41);
}

#[test]
fn strings_over_v8_max_length_fail_cleanly() {
    let p = project("napi-huge");
    compile_addon(&p, "full");
    p.file("huge.test.ts", spec("huge.test.ts"));
    assert_all_pass(&p, 1);
}

#[test]
fn module_init_outcomes() {
    let p = project("napi-init");
    for stem in [
        "init_custom",
        "init_throw",
        "noreg",
        "init_nested",
        "second",
        "unknown",
    ] {
        compile_addon(&p, stem);
    }
    p.file("garbage.node", "this is not a shared library\n");
    p.file("init.test.ts", spec("init.test.ts"));
    assert_all_pass(&p, 6);
}

/// Two workers require() the same addon at about the same time: its module init runs under the
/// addon lock like every other entry into addon code, one thread at a time.
#[test]
fn module_init_is_serialized_across_workers() {
    let p = project("napi-init-lock");
    compile_addon(&p, "init_lock");
    p.file("a.test.ts", spec("init_lock.test.ts"));
    p.file("b.test.ts", spec("init_lock.test.ts"));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_JOBS", "2")]);
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], 2, "{}", r.all());
}

#[test]
fn reentrant_callbacks_work() {
    let p = project("napi-reentry");
    compile_addon(&p, "full");
    compile_addon(&p, "second");
    p.file("reentry.test.ts", spec("reentry.test.ts"));
    assert_all_pass(&p, 3);
}

#[test]
fn each_addon_keeps_its_own_env() {
    let p = project("napi-addons");
    compile_addon(&p, "full");
    compile_addon(&p, "second");
    p.file("addons.test.ts", spec("addons.test.ts"));
    assert_all_pass(&p, 2);
}

#[test]
fn objects_wrap_native_pointers() {
    let p = project("napi-wrap");
    compile_addon(&p, "wrap");
    p.file("wrap.test.ts", spec("wrap.test.ts"));
    // --expose-gc: the spec collects wrapped objects to run their finalizers
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_V8_FLAGS", "--expose-gc")],
    );
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], 5, "{}", r.all());
}

/// A node-addon-api-style ObjectWrap class: napi_define_class builds the prototype (instance
/// methods with the class as their signature, accessors, values) and the static members from its
/// property descriptors, with Node's attributes; napi_get_new_target tells `new` from a call;
/// napi_define_properties defines the same descriptors on any object (an accessor missing one
/// half leaves that half out of the definition, turbo-test#43 item 4).
#[test]
fn define_class_builds_an_object_wrap_class() {
    let p = project("napi-objectwrap");
    compile_addon(&p, "objectwrap");
    p.file("objectwrap.test.ts", spec("objectwrap.test.ts"));
    assert_all_pass(&p, 10);
}

/// Type tags (napi_type_tag_object / napi_check_object_type_tag: one tag per object, compared
/// as two 64-bit words) and napi_add_finalizer (any number per object, run on collection, an
/// optional weak result reference), as Node; an addon importing them no longer crashes the run.
#[test]
fn objects_carry_type_tags_and_finalizers() {
    let p = project("napi-typetag");
    compile_addon(&p, "typetag");
    p.file("typetag.test.ts", spec("typetag.test.ts"));
    // --expose-gc: the spec collects objects to run their finalizers
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_V8_FLAGS", "--expose-gc")],
    );
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], 7, "{}", r.all());
}

/// Objects still wrapped (or with a napi_add_finalizer finalizer) when the addon's env is torn
/// down (the file's isolate is disposed) have their finalizers run then, newest first and
/// interleaved, as Node does at exit: in a handle scope, so value calls work (a strong reference
/// still has its value, the record's own reference has none, values can be created), while JS
/// cannot run (napi_pending_exception). The expected lines are Node 26's.
#[test]
fn wrap_finalizers_run_at_env_teardown() {
    let p = project("napi-wrap-teardown");
    compile_addon(&p, "wrap");
    p.file("wrap_teardown.test.ts", spec("wrap_teardown.test.ts"));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    assert_eq!(j["numPassedTests"], 1, "{}", r.all());
    let lines: Vec<&str> = r
        .stderr
        .lines()
        .filter(|l| l.starts_with("wrap teardown finalizer:"))
        .collect();
    let line = |id: u32| {
        format!(
            "wrap teardown finalizer: id={id} hint={} own_ref=0:-1 strong_ref=0:6 \
             create_object=0:6 get_named_property=10 delete_reference=0 call_function=10",
            id + 100
        )
    };
    assert_eq!(lines, [line(42), line(43), line(41)], "{}", r.all());
}

/// Reference counts, as Node (turbo-test#43 item 3): napi_create_reference honours its initial
/// count, napi_reference_ref / napi_reference_unref move it up and down, a reference at 0 is
/// weak (its value goes when the object is collected; only objects, functions and symbols can be
/// referenced) and above 0 strong, including a wrap's own reference.
#[test]
fn references_count_up_and_down() {
    let p = project("napi-refs");
    compile_addon(&p, "refs");
    p.file("refs.test.ts", spec("refs.test.ts"));
    // --expose-gc: the spec collects referenced objects
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_V8_FLAGS", "--expose-gc")],
    );
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], 4, "{}", r.all());
}

/// The Node-API functions beyond full.c (turbo-test#44), one section per family; the same spec
/// passes against Node 26: errors (napi_throw_* / napi_create_* of each kind,
/// napi_get_last_error_info, napi_fatal_exception), numbers, bigints, dates, strings (Latin-1,
/// UTF-16, property keys, external strings), symbols, coercions, arrays, properties (by key
/// value, own, named, elements, all names, prototype, freeze / seal), comparisons and type
/// checks, arraybuffers / typed arrays / dataviews / buffers (external ones alias the addon's
/// memory and run their finalizer on collection), promises, externals (with type tags),
/// napi_new_instance, napi_run_script, handle scopes, napi_adjust_external_memory,
/// node_api_get_module_file_name, and a NULL env.
#[test]
fn every_node_api_family_behaves() {
    let p = project("napi-api");
    compile_addon(&p, "api");
    p.file("api.test.ts", spec("api.test.ts"));
    // --expose-gc: the spec collects values to run their finalizers
    let r = turbo_test(
        &p.dir,
        &["--reporter", "json"],
        &[("TURBO_V8_FLAGS", "--expose-gc")],
    );
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], 46, "{}", r.all());
    // napi_fatal_exception reported the error as uncaught (what a throwing timer callback
    // gets), and so is what a finalizer threw (turbo-test#53)
    for line in [
        "uncaught reported as uncaught",
        "uncaught thrown by a finalizer",
    ] {
        assert!(r.stderr.contains(line), "{line}\n{}", r.all());
    }
}

/// napi_fatal_error prints `FATAL ERROR: <location> <message>` and aborts the process, as
/// Node does.
#[test]
fn fatal_error_aborts_the_process() {
    let p = project("napi-fatal");
    compile_addon(&p, "fatal");
    p.file(
        "fatal.test.ts",
        "import { test } from 'vitest';\ntest('dies', () => { require('./fatal.node').die(); });\n",
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, None, "killed by SIGABRT\n{}", r.all());
    assert!(
        r.stderr
            .contains("FATAL ERROR: tt_fatal_location the message\n"),
        "{}",
        r.all()
    );
}

/// At env teardown, as Node at exit: the env cleanup hooks (napi_add_env_cleanup_hook, a removed
/// one skipped) and async cleanup hooks run newest first, then an open threadsafe function is
/// finalized and, the env no longer held, the env's finalizers run (a wrap, the instance
/// data's; data replaced by napi_set_instance_data never gets its finalizer), and an external
/// string still alive gets its finalizer with a NULL env once the env is gone. The expected
/// lines are Node 26's.
#[test]
fn cleanup_hooks_and_instance_data_run_at_env_teardown() {
    let p = project("napi-cleanup");
    compile_addon(&p, "cleanup");
    p.file("cleanup.test.ts", spec("cleanup.test.ts"));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    assert_eq!(j["numPassedTests"], 1, "{}", r.all());
    let lines: Vec<&str> = r
        .stderr
        .lines()
        .filter_map(|l| l.strip_prefix("cleanup: "))
        .collect();
    assert_eq!(
        lines,
        [
            "instance data at init 0",
            "async hook 101",
            "async hook removed 0",
            "hook 4",
            "async hook 3",
            "async hook removed 0",
            "hook 1",
            "threadsafe function finalizer 9",
            "wrap finalizer 7",
            "instance data finalizer 6",
            "external string finalizer, env is NULL: 1",
            "external string finalizer hint 8",
        ],
        "{}",
        r.all()
    );
}

/// An addon registering itself with napi_module_register from a static constructor (the
/// NAPI_MODULE macro) loads; loaded again by a second file, its constructors do not run again
/// and its saved module's init does.
#[test]
fn self_registered_modules_load() {
    let p = project("napi-selfreg");
    compile_addon(&p, "selfreg");
    p.file("a.test.ts", spec("selfreg.test.ts"));
    p.file("b.test.ts", spec("selfreg.test.ts"));
    // dropped from require.cache, it is loaded again (turbo-test#47)
    p.file("reload.test.ts", spec("reload.test.ts"));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[("TURBO_JOBS", "1")]);
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], 3, "{}", r.all());
}

/// napi_get_version (10) and napi_get_node_version (the Node.js version process.versions.node
/// names), and an addon's declared module API version (node_api_module_get_api_version_v1) as
/// Node: primitives can be referenced from 10, below 8 is 8, above 10 fails require() (but
/// NAPI_VERSION_EXPERIMENTAL), and at env teardown a call that would run JS is
/// napi_cannot_run_js from 10 (napi_pending_exception before). The teardown lines are Node 26's.
#[test]
fn addons_declare_their_node_api_version() {
    let p = project("napi-apiversion");
    for (version, name) in [
        ("10", "v10"),
        ("3", "v3"),
        ("2147483647", "vexp"),
        ("11", "v11"),
    ] {
        compile_addon_as(
            &p,
            "apiversion",
            name,
            &[&format!("-DTT_VERSION={version}")],
        );
    }
    p.file("apiversion.test.ts", spec("apiversion.test.ts"));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], 4, "{}", r.all());
    let lines: Vec<&str> = r
        .stderr
        .lines()
        .filter(|l| l.starts_with("apiversion "))
        .collect();
    assert_eq!(
        lines,
        [
            "apiversion 3 teardown: napi_call_function=10",
            "apiversion 10 teardown: napi_call_function=23",
        ],
        "{}",
        r.all()
    );
}

/// Async Node-API, as Node (the same spec passes against Node 26): async work on a pool of
/// four threads (complete on the JS thread; queued work can be cancelled, running work cannot),
/// threadsafe functions called from other threads (blocking calls on a full queue, release /
/// abort, the default call_js, finalization, napi_closing), napi_make_callback, callback scopes
/// and napi_get_uv_event_loop. In-flight work and an open, referenced threadsafe function keep
/// the event loop alive. An exception left by a complete callback is uncaught; one left by a
/// threadsafe function's call_js (module API version 8) only warns, once.
#[test]
fn async_work_and_threadsafe_functions_run_on_the_event_loop() {
    let p = project("napi-async");
    compile_addon(&p, "async");
    p.file("async.test.ts", spec("async.test.ts"));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    assert_eq!(j["numFailedTests"], 0, "{}", r.all());
    assert_eq!(j["numPassedTests"], 17, "{}", r.all());
    assert!(
        r.stderr.contains("uncaught thrown by a complete callback"),
        "{}",
        r.all()
    );
    assert_eq!(
        r.stderr
            .matches("[DEP0168] DeprecationWarning: Uncaught Node-API callback exception detected")
            .count(),
        1,
        "{}",
        r.all()
    );
}
