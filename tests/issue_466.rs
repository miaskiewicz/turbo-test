#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

fn check(name: &str, code: &str, environment: &str) {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", "{\"name\":\"issue-466\"}");
    p.file("case.test.cjs", code);
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json", "--environment", environment],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
}

#[test]
fn fetches_from_a_local_http_server() {
    check(
        "466-fetch",
        r#"
const http = require('node:http');
test('fetch loopback', async () => {
  const server = http.createServer((req, res) => { let body = ''; req.on('data', b => body += b); req.on('end', () => { res.writeHead(201, {'x-test': 'ok'}); res.end(req.method === 'POST' ? body : 'hello'); }); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const response = await fetch(`http://127.0.0.1:${server.address().port}/`);
    expect(response.status).toBe(201); expect(response.headers.get('x-test')).toBe('ok');
    expect(await response.text()).toBe('hello'); const post = await fetch(`http://127.0.0.1:${server.address().port}/`, {method:'POST', body:'posted'}); expect(await post.text()).toBe('posted'); const request = new Request(`http://127.0.0.1:${server.address().port}/`, {method:'POST', body:'from-request'}); expect(await (await fetch(request)).text()).toBe('from-request');
  } finally { await new Promise(resolve => server.close(resolve)); }
});
"#,
        "node",
    );
}

#[test]
fn fetches_from_a_local_http_server_in_dom_environment() {
    check(
        "466-fetch-dom",
        r#"
const http = require('node:http');
test('fetch loopback in DOM', async () => {
 const server = http.createServer((_req, res) => res.end('ok'));
 await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
 try { expect(await (await fetch(`http://127.0.0.1:${server.address().port}/`)).text()).toBe('ok'); }
 finally { await new Promise(resolve => server.close(resolve)); }
});
"#,
        "jsdom",
    );
}

#[test]
fn spawn_sync_supports_ignored_stdio() {
    check(
        "466-spawn-ignore",
        r#"
const {spawnSync} = require('node:child_process');
test('ignore stdio', () => {
 const result = spawnSync('/bin/sh', ['-c', 'printf hello; printf err >&2'], {stdio:'ignore'});
 expect(result.status).toBe(0); expect(result.stdout).toBeNull(); expect(result.stderr).toBeNull(); expect(result.output).toEqual([null, null, null]); const noisy = spawnSync('/bin/sh', ['-c', 'printf 123456789'], {stdio:'ignore', maxBuffer:1}); expect(noisy.status).toBe(0); expect(noisy.error).toBeUndefined();
});
"#,
        "node",
    );
}

#[test]
fn accepts_p256_curve_alias() {
    check(
        "466-ec-alias",
        r#"
const {generateKeyPairSync, sign, verify} = require('node:crypto');
test('P-256 alias', () => {
 const {privateKey, publicKey} = generateKeyPairSync('ec', {namedCurve:'P-256'});
 const message = Buffer.from('hello'); const signature = sign('sha256', message, privateKey);
 expect(verify('sha256', message, publicKey, signature)).toBe(true);
});
"#,
        "node",
    );
}

#[test]
fn rejects_special_urls_without_a_host() {
    check(
        "466-url",
        r#"
test('invalid special URLs', () => {
 for (const value of ['https://','http://','https://:443']) { try { new URL(value); throw Error('expected failure'); } catch (error) { expect(error.code).toBe('ERR_INVALID_URL'); } }
 expect(new URL('file:///tmp').hostname).toBe('');
});
"#,
        "node",
    );
}

#[test]
fn response_array_buffer_preserves_binary() {
    check(
        "466-response",
        r#"
test('response bytes', async () => {
 const body = new Uint8Array([0, 127, 128, 255]);
 const response = new Response(body); expect(response.bodyUsed).toBe(false); expect(Array.from(new Uint8Array(await response.arrayBuffer()))).toEqual([0, 127, 128, 255]); expect(response.bodyUsed).toBe(true); await expect(response.arrayBuffer()).rejects.toThrow(); const empty = new Response(null); await empty.arrayBuffer(); expect(empty.bodyUsed).toBe(false); await empty.arrayBuffer();
});
"#,
        "node",
    );
}

#[test]
fn process_chdir_changes_working_directory() {
    check(
        "466-chdir",
        r#"
const fs = require('node:fs'); const path = require('node:path');
test('chdir', () => {
 const before = process.cwd(); fs.mkdirSync('child');
 try { process.chdir('child'); expect(process.cwd()).toBe(path.join(before, 'child')); fs.writeFileSync('inside.txt', 'ok'); expect(fs.readFileSync('inside.txt', 'utf8')).toBe('ok'); expect(fs.existsSync(path.join(before, 'inside.txt'))).toBe(false); expect(() => process.chdir('missing')).toThrow(); }
 finally { process.chdir(before); }
});
"#,
        "node",
    );
}

#[test]
fn filesystem_copy_and_glob() {
    check(
        "466-fs",
        r#"
const fs = require('node:fs');
test('recursive copy and glob', () => {
 fs.mkdirSync('source'); fs.writeFileSync('source/a.txt', 'hello'); fs.symlinkSync('a.txt', 'source/link');
 fs.cpSync('source', 'copy', {recursive:true});
 expect(fs.readFileSync('copy/a.txt', 'utf8')).toBe('hello'); expect(fs.lstatSync('copy/link').isSymbolicLink()).toBe(true); expect(fs.readFileSync('copy/link', 'utf8')).toBe('hello');
 expect(fs.globSync('copy/*.txt')).toEqual(['copy/a.txt']); expect(() => fs.cpSync('source', 'source/nested', {recursive:true})).toThrow();
});
"#,
        "node",
    );
}

#[test]
#[ignore = "waiting for turbo-dom#586; tracked by turbo-test#467"]
fn document_open_exists_on_created_documents() {
    check(
        "466-document-open",
        r#"
test('document.open', () => {
 const doc = document.implementation.createHTMLDocument('');
 expect(typeof doc.open).toBe('function');
 expect(doc.open()).toBe(doc);
});
"#,
        "jsdom",
    );
}

#[test]
fn filesystem_glob_supports_recursive_patterns() {
    check(
        "466-glob",
        r#"
const fs = require('node:fs');
test('glob nested files', () => {
 fs.mkdirSync('src/deep', {recursive:true}); fs.writeFileSync('src/deep/a.js', ''); fs.writeFileSync('src/b.js', ''); fs.writeFileSync('src/.hidden.js', '');
 expect(fs.globSync('src/**/*.js').sort()).toEqual(['src/b.js', 'src/deep/a.js']); expect(fs.globSync('src/[ab].js')).toEqual(['src/b.js']);
});
"#,
        "node",
    );
}

#[test]
fn filesystem_watch_observes_changes_and_can_close() {
    check(
        "466-watch",
        r#"
const fs = require('node:fs');
test('watch file', async () => {
 fs.writeFileSync('watch.txt', 'first');
 await new Promise((resolve, reject) => {
   const timer = setTimeout(() => { watcher.close(); reject(new Error('watch timed out')); }, 500);
   const watcher = fs.watch('watch.txt', (event, name) => {
     try { expect(['change', 'rename']).toContain(event); expect(String(name)).toBe('watch.txt'); }
     catch (error) { clearTimeout(timer); watcher.close(); reject(error); return; }
     clearTimeout(timer); watcher.close(); resolve();
   });
   fs.writeFileSync('watch.txt', 'second');
 });
});
"#,
        "node",
    );
}

#[test]
fn filesystem_watch_directory_reports_the_changed_name() {
    check(
        "466-watch-directory",
        r#"
const fs = require('node:fs');
test('watch directory', async () => {
 fs.mkdirSync('watched');
 await new Promise((resolve, reject) => {
  const timer = setTimeout(() => { watcher.close(); reject(new Error('watch timed out')); }, 1000);
  const watcher = fs.watch('watched', (event, name) => {
   if (name === null) return;
   try { expect(['rename', 'change']).toContain(event); expect(String(name)).toBe('new.txt'); }
   catch (error) { clearTimeout(timer); watcher.close(); reject(error); return; }
   clearTimeout(timer); watcher.close(); resolve();
  });
  fs.writeFileSync('watched/new.txt', 'created');
 });
});
"#,
        "node",
    );
}

#[test]
fn chdir_is_isolated_between_parallel_workers() {
    let p = Project::new("466-chdir-parallel");
    p.with_esbuild();
    p.file("package.json", "{\"name\":\"issue-466-parallel\"}");
    for name in ["a", "b"] {
        p.file(
            &format!("{name}.test.cjs"),
            &format!(
                r#"
const fs = require('node:fs');
test('worker {name}', async () => {{
 const before = process.cwd(); fs.mkdirSync('{name}'); process.chdir('{name}');
 await new Promise(resolve => setTimeout(resolve, 10));
 fs.writeFileSync('result.txt', '{name}');
 expect(fs.readFileSync('result.txt', 'utf8')).toBe('{name}');
 process.chdir(before);
}});
"#
            ),
        );
    }
    let run = turbo_test(
        &p.dir,
        &["--reporter", "json", "--environment", "node", "--jobs", "2"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 2, "{}", run.all());
    assert_eq!(
        std::fs::read_to_string(p.path("a/result.txt")).unwrap(),
        "a"
    );
    assert_eq!(
        std::fs::read_to_string(p.path("b/result.txt")).unwrap(),
        "b"
    );
}

#[test]
fn vm_contexts_work_in_a_mixed_dom_project() {
    let p = Project::new("466-mixed-vm");
    p.with_esbuild();
    p.file("package.json", "{\"name\":\"issue-466-mixed\"}");
    for index in 0..8 {
        p.file(
            &format!("dom{index}.test.js"),
            "test('dom', () => expect(typeof document).toBe('object'));",
        );
    }
    p.file("node.test.cjs", "// @vitest-environment node\nconst vm = require('node:vm'); test('vm', () => expect(vm.runInNewContext('value + 1', { value: 2 })).toBe(3));");
    let run = turbo_test(&p.dir, &["--reporter", "json", "--jobs", "2"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 9, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

#[test]
fn glob_does_not_descend_into_unmatched_directories() {
    check(
        "466-glob-pruning",
        r#"
const fs = require('node:fs');
test('nonrecursive glob', () => {
 fs.mkdirSync('hidden-dir'); fs.chmodSync('hidden-dir', 0);
 try { expect(fs.globSync('*')).toContain('hidden-dir'); }
 finally { fs.chmodSync('hidden-dir', 0o700); }
});
"#,
        "node",
    );
}

#[test]
fn recursive_copy_rejects_dereferenced_symlink_cycles() {
    check(
        "466-copy-cycle",
        r#"
const fs = require('node:fs');
test('recursive copy detects cycles', () => {
 fs.mkdirSync('source'); fs.symlinkSync('.', 'source/loop');
 try { fs.cpSync('source', 'copy', {recursive:true, dereference:true}); throw Error('expected error'); }
 catch (error) { expect(error.code).toBe('ERR_FS_CP_EINVAL'); }
});
"#,
        "node",
    );
}

#[test]
fn recursive_watch_detects_nested_changes_and_abort_closes_it() {
    check(
        "466-watch-recursive",
        r#"
const fs = require('node:fs');
test('recursive watch and abort', async () => {
 fs.mkdirSync('watched/nested', {recursive:true});
 const controller = new AbortController();
 await new Promise((resolve, reject) => {
  const timer = setTimeout(() => { watcher.close(); reject(new Error('watch timed out')); }, 2000);
  const watcher = fs.watch('watched', {recursive:true, signal:controller.signal}, (event, name) => {
   if (name === null) return;
   try { expect(String(name)).toBe('nested/new.txt'); expect(['rename', 'change']).toContain(event); }
   catch (error) { clearTimeout(timer); watcher.close(); reject(error); return; }
   watcher.on('close', () => { clearTimeout(timer); resolve(); });
   controller.abort();
  });
  fs.writeFileSync('watched/nested/new.txt', 'created');
 });
});
"#,
        "node",
    );
}
