//! Real per-environment payload activation and reuse boundaries (#390).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const NODE: &str = r#"
const assert=require('node:assert/strict');
test('Node initializes backend without DOM payload',()=>{
 assert.equal(globalThis.__ttRuntimeBackendInstalled,true);
 for(const key of ['document','DOMPoint','DOMMatrix','FileReader','MutationObserver','localStorage']) assert.equal(typeof globalThis[key],'undefined',key);
 assert.equal(typeof globalThis.__ttAesGcm,'function'); assert.equal(typeof globalThis.__ttTcp,'function');
 const cipher=require('node:crypto').createCipheriv('aes-256-gcm',Buffer.alloc(32),Buffer.alloc(12));
 const ciphertext=Buffer.concat([cipher.update(Buffer.alloc(16)),cipher.final()]);
 assert.equal(ciphertext.toString('hex'),'cea7403d4d606b6e074ec5d3baf39d18');
 assert.equal(cipher.getAuthTag().toString('hex'),'d0d1c8a799996bf0265b98b5d48ab919');
 assert.equal(require('node:crypto').createHash('sha256').update('abc').digest('hex'),'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
 assert.equal(typeof globalThis.__otherProfileSentinel,'undefined');globalThis.__otherProfileSentinel='Node';
});
"#;
const DOM: &str = r#"
const assert=require('node:assert/strict');
test('DOM initializes browser without backend payload or callbacks',async()=>{
 assert.equal(globalThis.__ttRuntimeBackendInstalled,false);assert.equal(globalThis.__ttRuntimeDomInstalled,true);
 for(const key of ['__ttAesGcm','__ttRsa','__ttEc','__ttX509','__ttSpawnSync','__ttAsyncProcess','__ttTcp','__ttHttp','__ttSetPromiseHooks','__ttAsyncFrameGet']) assert.equal(typeof globalThis[key],'undefined',key);
 assert.equal(typeof document.createElement,'function');assert.equal(typeof DOMPoint,'function');
 assert.equal(eval('typeof __nodeKeys'),'undefined');assert.equal(eval('typeof __nodeAes'),'undefined');assert.equal(eval('typeof __tcpSockets'),'undefined');
 assert.equal(typeof globalThis.__otherProfileSentinel,'undefined');globalThis.__otherProfileSentinel='DOM';
 assert.equal(require('node:crypto').createHmac('sha256',Buffer.alloc(20,11)).update('Hi There').digest('hex'),'b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7');
 assert.throws(()=>require('node:crypto').createCipheriv(),{code:'ERR_TURBO_BACKEND_CAPABILITY'});
 const element=document.createElement('p');element.textContent='profile DOM';document.body.appendChild(element);
 const channel=new MessageChannel();
 await new Promise((resolve,reject)=>{channel.port2.onmessage=event=>{try{assert.equal(event.data,'profile task');resolve();}catch(error){reject(error);}finally{channel.port1.close();channel.port2.close();}};channel.port1.postMessage('profile task');});
 assert.equal(element.textContent,'profile DOM');
});
"#;
fn project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{"name":"runtime-profiles"}"#);
    p
}
fn passed(p: &Project, count: usize, args: &[&str], env: &[(&str, &str)]) {
    let run = turbo_test(&p.dir, args, env);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], count, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
#[test]
fn profile_selection_happens_before_payload_and_respects_explicit_environments() {
    let p = project("runtime-profile-priority");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file(
        "node.test.tsx",
        &(String::from("/* @vitest-environment node */\n// document window.ReactDOM\n") + NODE),
    );
    p.file(
        "dom.test.cjs",
        &(String::from("/* @vitest-environment jsdom */\n") + DOM),
    );
    passed(&p, 2, &["--jobs", "1", "--reporter", "json"], &[]);
}
#[test]
fn profile_changes_recycle_reused_workers_in_both_directions() {
    for (name, first, second) in [("node-dom-node", NODE, DOM), ("dom-node-dom", DOM, NODE)] {
        let p = project(name);
        for (index, source) in [(0, first), (1, second), (2, first)] {
            let env = if source == NODE { "node" } else { "happy-dom" };
            p.file(
                &format!("0{index}.test.cjs"),
                &format!("/* @vitest-environment {env} */\n{source}"),
            );
        }
        passed(
            &p,
            3,
            &["--jobs", "1", "--no-isolate", "--reporter", "json"],
            &[],
        );
    }
}
#[test]
fn window_snapshot_plain_node_context_and_browser_context_coexist() {
    let p = project("runtime-window-profiles");
    for index in 0..8 {
        p.file(
            &format!("dom-{index}.test.cjs"),
            &format!("/* @vitest-environment jsdom */\n{DOM}"),
        );
    }
    p.file(
        "node.test.cjs",
        &format!("/* @vitest-environment node */\n{NODE}"),
    );
    passed(&p, 9, &["--jobs", "2", "--reporter", "json"], &[]);
}
#[test]
fn configured_and_runlevel_environments_win_over_content_detection() {
    let p = project("runtime-profile-config");
    p.file(
        "vitest.config.js",
        "export default {test:{environment:'node'}};",
    );
    p.file(
        "profile.test.tsx",
        &(String::from("// document window.ReactDOM\n") + NODE),
    );
    passed(&p, 1, &["--jobs", "1", "--reporter", "json"], &[]);
    p.file("profile.test.tsx", DOM);
    passed(
        &p,
        1,
        &["--jobs", "1", "--reporter", "json"],
        &[("TURBO_ENV", "jsdom")],
    );
}
