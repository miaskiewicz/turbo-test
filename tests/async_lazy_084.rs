//! Unused async context incurs no native capture calls while preserving activation semantics.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn fixture_project(name: &str, environment: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{"name":"async-lazy-084"}"#);
    p.file(
        "jest.config.js",
        &format!("module.exports = {{ testEnvironment: {environment:?} }};\n"),
    );
    p
}
fn assert_passes(run: &common::Run, count: u64) {
    let report = json_report(run);
    assert_eq!(report["numPassedTests"], count, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
#[test]
fn unused_frame_capture_is_lazy_in_the_node_backend() {
    let p = fixture_project("async-lazy-node", "node");
    p.file(
        "lazy.test.cjs",
        include_str!("data/compat-084/async-lazy.cjs"),
    );
    assert_passes(&turbo_test(&p.dir, &["--reporter", "json"], &[]), 4);
}
#[test]
fn reuse_reset_makes_native_capture_lazy_again_without_observing_prior_frames() {
    let p = fixture_project("async-lazy-reuse", "node");
    p.file("00-first.test.cjs", r#"
const assert = require('node:assert/strict');
const { AsyncLocalStorage } = require('node:async_hooks');
globalThis.__lazyReuseStorage = new AsyncLocalStorage();
globalThis.__lazyReuseStorage.enterWith('prior-file');
test('prior file context is active', () => assert.equal(__lazyReuseStorage.getStore(), 'prior-file'));
"#);
    p.file("01-second.test.cjs", r#"
const assert = require('node:assert/strict');
const nativeGet = globalThis.__ttAsyncFrameGet; let reads = 0;
globalThis.__ttAsyncFrameGet = () => { reads++; return nativeGet(); };
test('next file clears prior frame without capture overhead until reactivated', async () => {
  assert.equal(__lazyReuseStorage.getStore(), undefined);
  await new Promise(resolve => setTimeout(resolve, 1));
  assert.equal(reads, 0);
  const value = await __lazyReuseStorage.run('new-file', async () => { await Promise.resolve(); return __lazyReuseStorage.getStore(); });
  assert.equal(value, 'new-file'); assert.ok(reads > 0);
});
"#);
    assert_passes(
        &turbo_test(
            &p.dir,
            &["--reporter", "json", "--jobs", "1", "--no-isolate"],
            &[],
        ),
        2,
    );
}

#[test]
fn inactive_resource_dispatch_avoids_native_transitions_and_preserves_later_activation() {
    let p = fixture_project("async-resource-lazy-node", "node");
    p.file(
        "resource.test.cjs",
        include_str!("data/compat-084/async-resource-lazy.cjs"),
    );
    assert_passes(&turbo_test(&p.dir, &["--reporter", "json"], &[]), 1);
}
#[test]
fn dom_tasks_never_initialize_node_async_context_or_native_hooks() {
    let p = fixture_project("async-lazy-dom", "jsdom");
    p.file("dom.test.cjs",r#"
const assert=require('node:assert/strict');
function absent() {
  assert.equal(globalThis.__ttRuntimeBackendInstalled,false);
  for(const name of ['__ttAsyncFrameGet','__ttAsyncFrameSet','__ttSetPromiseHooks','__ttDrainAsyncHookTasks']) assert.equal(typeof globalThis[name],'undefined',name);
  assert.equal(eval('typeof __asyncFrameActivated'),'undefined');
  // (requiring a backend module would activate the backend, as vitest's jsdom has all of Node)
}
test('DOM work timers promises and nextTicks remain independent of Node context',async()=>{
  absent();const tasks=[];
  for(let i=0;i<16;i++) {
    tasks.push(new Promise(resolve=>setTimeout(resolve,1)));
    tasks.push(new Promise(resolve=>setImmediate(resolve)));
    tasks.push(new Promise(resolve=>process.nextTick(resolve)));
  }
  await Promise.all(tasks);
  const element=document.createElement('div');element.textContent='shared task';document.body.appendChild(element);
  assert.equal(element.textContent,'shared task');element.remove();absent();
});
test('DOM MessagePort resource dispatch keeps its receiver without Node activation',async()=>{
  absent();const channel=new MessageChannel();
  const received=new Promise(resolve=>channel.port1.on('message',function(value){assert.equal(this,channel.port1);resolve(value);}));
  channel.port2.postMessage('delivered');assert.equal(await received,'delivered');
  channel.port1.close();channel.port2.close();await new Promise(resolve=>setImmediate(resolve));absent();
});
"#);
    assert_passes(&turbo_test(&p.dir, &["--reporter", "json"], &[]), 2);
}
