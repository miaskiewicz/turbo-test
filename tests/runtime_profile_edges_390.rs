//! Runtime-profile ownership edges: preserved same-profile caches and clean transitions.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{"name":"runtime-profile-edges"}"#);
    p
}
fn assert_passed(run: &common::Run, count: usize) {
    let report = json_report(run);
    assert_eq!(run.code, Some(0), "{}", run.all());
    assert_eq!(report["numPassedTests"], count, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
fn same_profile_fixture(p: &Project, environment: &str) {
    p.file("vitest.config.js", &format!("export default {{test:{{environment:{environment:?},isolate:false,setupFiles:['./setup.cjs']}}}};"));
    p.file("setup.cjs", "globalThis.__setupRuns=(globalThis.__setupRuns||0)+1; beforeEach(()=>{globalThis.__setupHookCalls=(globalThis.__setupHookCalls||0)+1;});");
    p.file(
        "node_modules/profile-singleton/package.json",
        r#"{"name":"profile-singleton","main":"index.cjs"}"#,
    );
    p.file("node_modules/profile-singleton/index.cjs", "globalThis.__singletonEvaluations=(globalThis.__singletonEvaluations||0)+1;module.exports={identity:{}};");
    let activate = if environment == "node" {
        "const {AsyncLocalStorage,createHook}=require('node:async_hooks');globalThis.__oldStorage=new AsyncLocalStorage();__oldStorage.enterWith('prior-file');globalThis.__staleAsyncCalls=0;globalThis.__oldTimerCalls=0;createHook({init(){if(globalThis.__secondFile)__staleAsyncCalls++;}}).enable();setInterval(()=>{if(globalThis.__secondFile)__oldTimerCalls++;},1);globalThis.__mutatedResource=function(){throw new Error('public AsyncResource must not drive private task resources');};require('node:async_hooks').AsyncResource=__mutatedResource;"
    } else {
        ""
    };
    p.file("00-first.test.cjs", &format!(r#"
const assert=require('node:assert/strict');
globalThis.__singleton=require('profile-singleton');globalThis.__oldHookCalls=0;
beforeEach(()=>{{globalThis.__oldHookCalls++;}});
{activate}
test('first file establishes same-profile module identity and local hooks',()=>{{
 assert.equal(__setupRuns,1);assert.equal(__setupHookCalls,1);assert.equal(__oldHookCalls,1);assert.equal(__singletonEvaluations,1);
}});
"#));
    let reset = if environment == "node" {
        "assert.equal(__oldStorage.getStore(),undefined);assert.equal(__staleAsyncCalls,0);assert.equal(__oldTimerCalls,0);assert.equal(require('node:async_hooks').AsyncResource,__mutatedResource);assert.equal(globalThis.__ttRuntimeBackendInstalled,true);assert.equal(typeof document,'undefined');await __oldStorage.run('private-context',async()=>{await Promise.resolve();const channel=new MessageChannel();const delivered=new Promise(resolve=>channel.port1.once('message',()=>resolve(__oldStorage.getStore())));channel.port2.postMessage('scope');assert.equal(await delivered,'private-context');channel.port1.close();channel.port2.close();});assert.equal(__oldStorage.getStore(),undefined);assert.equal(__staleAsyncCalls,0);assert.equal(__oldTimerCalls,0);"
    } else {
        "assert.equal(globalThis.__ttRuntimeBackendInstalled,false);assert.equal(typeof globalThis.__ttAsyncFrameGet,'undefined');assert.equal(typeof globalThis.__ttSetPromiseHooks,'undefined');assert.equal(typeof document.createElement,'function');"
    };
    p.file("01-next.test.cjs", &format!(r#"
const assert=require('node:assert/strict');globalThis.__secondFile=true;
const singleton=require('profile-singleton');
test('same-profile reuse keeps module identity but resets file hooks and async state',async()=>{{
 await new Promise(resolve=>setImmediate(resolve));
 assert.equal(singleton,__singleton);assert.equal(singleton.identity,__singleton.identity);
 assert.equal(__singletonEvaluations,1);assert.equal(__setupRuns,1);assert.equal(__setupHookCalls,2);assert.equal(__oldHookCalls,1);
 {reset}
}});
"#));
}
#[test]
fn same_profile_reuse_keeps_module_identity_and_resets_file_owned_hooks_and_async_context() {
    for environment in ["node", "jsdom"] {
        let p = project(&format!("profile-same-{environment}"));
        same_profile_fixture(&p, environment);
        assert_passed(
            &turbo_test(
                &p.dir,
                &["--jobs", "1", "--no-isolate", "--reporter", "json"],
                &[],
            ),
            2,
        );
    }
}
#[test]
fn failed_node_load_reaps_owned_children_before_dom_and_subsequent_node_files() {
    let p = project("profile-failed-load-cleanup");
    p.file(
        "00-failed.test.cjs",
        r#"/* @vitest-environment node */
const fs=require('node:fs');const {spawn}=require('node:child_process');
const child=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{env:{}});
fs.appendFileSync('owned-pids.txt',String(child.pid)+'\n');
throw new Error('PROFILE_OWNED_LOAD_FAILURE');
"#,
    );
    p.file("01-dom.test.cjs",r#"/* @vitest-environment jsdom */
const assert=require('node:assert/strict');
test('DOM after failed Node load receives no owned backend callbacks',()=>{
 assert.equal(globalThis.__ttRuntimeBackendInstalled,false);
 assert.equal(typeof globalThis.__ttAsyncProcess,'undefined');assert.equal(typeof globalThis.__ttSpawnSync,'undefined');
 assert.equal(typeof document.createElement,'function');assert.equal(require('node:fs').existsSync('owned-pids.txt'),true);
});
"#);
    p.file("02-node.test.cjs",r#"/* @vitest-environment node */
const assert=require('node:assert/strict');const fs=require('node:fs');const {spawnSync}=require('node:child_process');
test('subsequent Node profile proves every failed-load child was reaped',()=>{
 const pids=fs.readFileSync('owned-pids.txt','utf8').trim().split('\n').map(Number);
 assert.ok(pids.length>=1);assert.ok(pids.every(pid=>Number.isInteger(pid)&&pid>0));
 const code='const ids='+JSON.stringify(pids)+';for(const pid of ids){try{process.kill(pid,0);process.exit(7);}catch(error){if(error.code!=="ESRCH")process.exit(8);}}';
 const probe=spawnSync(process.execPath,['-e',code],{env:{}});assert.equal(probe.status,0);
 assert.equal(globalThis.__ttRuntimeBackendInstalled,true);assert.equal(typeof document,'undefined');
});
"#);
    let run = turbo_test(
        &p.dir,
        &["--jobs", "1", "--no-isolate", "--reporter", "json"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(run.code, Some(1), "{}", run.all());
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    assert_eq!(report["numFailedTestSuites"], 1, "{}", run.all());
    assert!(
        run.stdout.contains("PROFILE_OWNED_LOAD_FAILURE"),
        "{}",
        run.all()
    );
}
#[test]
fn mixed_profile_reuse_is_clean_when_window_snapshot_is_explicitly_disabled() {
    let p = project("profile-window-snapshot-off");
    let node = r#"/* @vitest-environment node */
const assert=require('node:assert/strict');
test('plain snapshot Node initializes only backend capabilities',()=>{
 assert.equal(globalThis.__ttRuntimeBackendInstalled,true);assert.equal(typeof document,'undefined');assert.equal(typeof DOMPoint,'undefined');
 assert.equal(typeof globalThis.__ttAesGcm,'function');assert.equal(typeof globalThis.__ttAsyncFrameGet,'function');
 assert.equal(require('node:crypto').createHash('sha256').update('abc').digest('hex'),'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
 assert.equal(typeof globalThis.__previousProfile,'undefined');globalThis.__previousProfile='node';
});
"#;
    p.file("00-node.test.cjs", node);
    p.file("01-dom.test.cjs",r#"/* @vitest-environment happy-dom */
const assert=require('node:assert/strict');
test('plain snapshot installs DOM without inheriting the preceding backend',async()=>{
 assert.equal(globalThis.__ttRuntimeBackendInstalled,false);assert.equal(globalThis.__ttRuntimeDomInstalled,true);
 assert.equal(typeof globalThis.__ttAesGcm,'undefined');assert.equal(typeof globalThis.__ttAsyncFrameGet,'undefined');
 assert.equal(typeof globalThis.__previousProfile,'undefined');globalThis.__previousProfile='dom';
 const element=document.createElement('span');element.textContent='fresh DOM';assert.equal(element.textContent,'fresh DOM');
 const channel=new MessageChannel();const received=new Promise(resolve=>channel.port1.once('message',resolve));
 channel.port2.postMessage('shared task');assert.equal(await received,'shared task');channel.port1.close();channel.port2.close();
});
"#);
    p.file("02-node.test.cjs", node);
    assert_passed(
        &turbo_test(
            &p.dir,
            &["--jobs", "1", "--no-isolate", "--reporter", "json"],
            &[("TURBO_SNAPSHOT", "0")],
        ),
        3,
    );
}
