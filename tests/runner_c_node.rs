//! Node built-in module shims through the real CLI: the `fs` shim's append semantics under
//! parallel workers, and the builtins that used to import as an empty module (named + default
//! exports with Node's behaviour, the same on the ESM-graph and module-runner paths).
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

#[test]
fn append_file_sync_from_parallel_workers_keeps_every_line() {
    // 4 files appending 200 lines each to ONE log from 4 workers: a read-modify-write shim
    // loses most of them; a real O_APPEND append keeps all 800.
    let p = Project::new("rc-append");
    p.file("package.json", r#"{"name":"rc-append","type":"module"}"#);
    let log = p.path("log.txt");
    let mut files = Vec::new();
    for n in 1..=4 {
        let name = format!("f{n}.test.js");
        p.file(
            &name,
            &format!(
                "import fs from 'node:fs';\ntest('append {n}', () => {{\n  for (let i = 0; i < 200; i++) fs.appendFileSync({log:?}, 'file{n}-line' + i + '\\n');\n}});\n"
            ),
        );
        files.push(name);
    }
    let args: Vec<&str> = files.iter().map(String::as_str).collect();
    let r = turbo_test(&p.dir, &args, &[("TURBO_JOBS", "4")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let text = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 800, "{text}");
    for n in 1..=4 {
        let prefix = format!("file{n}-line");
        let mine: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| l.starts_with(&prefix))
            .collect();
        let want: Vec<String> = (0..200).map(|i| format!("{prefix}{i}")).collect();
        assert_eq!(mine, want, "file{n}'s lines, in order");
    }
}

#[test]
fn append_file_variants_extend_and_report_errors() {
    let p = Project::new("rc-append-variants");
    p.file(
        "package.json",
        r#"{"name":"rc-append-variants","type":"module"}"#,
    );
    p.file(
        "a.test.js",
        r#"import fs from 'node:fs';
test('appendFileSync and promises.appendFile', async () => {
  const root = globalThis.__cwd;
  const f = root + '/log.txt';
  fs.appendFileSync(f, 'a');
  fs.appendFileSync(f, 'b');
  await fs.promises.appendFile(f, 'c\n');
  expect(fs.readFileSync(f, 'utf8')).toBe('abc\n');
  expect(() => fs.appendFileSync(root + '/package.json/x', 'a')).toThrow('appendFileSync ' + root + '/package.json/x');
  await expect(fs.promises.appendFile(root + '/package.json/x', 'a')).rejects.toThrow('appendFileSync ' + root + '/package.json/x');
});
"#,
    );
    let r = turbo_test(&p.dir, &["a.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(std::fs::read_to_string(p.path("log.txt")).unwrap(), "abc\n");
}

/// One test per builtin that had no shim (or only an empty object): named + default imports
/// with Node's behaviour. Written as ESM so the same file runs on both loader paths (the
/// module runner turns the imports into `require`s of the same shims).
const BUILTINS_TEST: &str = r#"import bufferMod, { Buffer, atob, btoa, Blob } from 'node:buffer';
import fsp, { readFile, writeFile, appendFile } from 'fs/promises';
import fs from 'node:fs';
import proc, { env, cwd } from 'node:process';
import timers, { setTimeout as st, clearTimeout as ct } from 'timers';
import tp, { setTimeout as sleep, setImmediate as tick, setInterval as every, scheduler } from 'node:timers/promises';
import vm, { runInThisContext, Script, compileFunction, isContext, createContext } from 'vm';
import wt, { isMainThread, parentPort, threadId, workerData, Worker, MessageChannel, getEnvironmentData, setEnvironmentData } from 'node:worker_threads';
import qs, { parse, stringify, escape, unescape } from 'querystring';
import sdMod, { StringDecoder } from 'node:string_decoder';
import constants from 'constants';
import os from 'node:os';
import inspector, { url as inspectorUrl, Session } from 'node:inspector';
import assertStrict from 'node:assert/strict';
import assert, { strict } from 'assert';

test('buffer', () => {
  expect(Buffer.from('a').length).toBe(1);
  expect(Buffer).toBe(globalThis.Buffer);
  expect(bufferMod.Buffer).toBe(Buffer);
  expect(btoa('hi')).toBe('aGk=');
  expect(atob('aGk=')).toBe('hi');
  expect(Blob).toBe(globalThis.Blob);
});
test('fs/promises', async () => {
  expect(fsp).toBe(fs.promises);
  const f = globalThis.__cwd + '/p.txt';
  await writeFile(f, 'x');
  await appendFile(f, 'y');
  expect(await readFile(f, 'utf8')).toBe('xy');
  expect(fsp.constants).toBe(fs.constants);
});
test('process', () => {
  expect(proc).toBe(globalThis.process);
  expect(env).toBe(process.env);
  expect(cwd()).toBe(globalThis.__cwd);
});
test('timers', async () => {
  expect(timers.setTimeout).toBe(globalThis.setTimeout);
  expect(timers.setImmediate).toBe(globalThis.setImmediate);
  expect(timers.promises).toBe(tp);
  let cancelled = true;
  ct(st(() => { cancelled = false; }, 1));
  expect(await new Promise((r) => st(r, 5, 'fired'))).toBe('fired');
  expect(cancelled).toBe(true);
});
test('timers/promises', async () => {
  expect(await sleep(10, 'v')).toBe('v');
  expect(await tick('i')).toBe('i');
  expect(await scheduler.wait(5)).toBe(undefined);
  expect(await scheduler.yield()).toBe(undefined);
  const ac = new AbortController();
  const p = sleep(1000, 0, { signal: ac.signal });
  ac.abort();
  await expect(p).rejects.toMatchObject({ name: 'AbortError', code: 'ABORT_ERR' });
  await expect(tick(0, { signal: ac.signal })).rejects.toMatchObject({ name: 'AbortError' });
  const seen = [];
  for await (const v of every(5, 'tick')) { seen.push(v); if (seen.length === 3) break; }
  expect(seen).toEqual(['tick', 'tick', 'tick']);
  const ac2 = new AbortController();
  const it = every(5, 1, { signal: ac2.signal });
  expect(await it.next()).toEqual({ done: false, value: 1 });
  const pending = it.next();
  ac2.abort();
  await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
  expect(await it.next()).toEqual({ done: true, value: undefined });
});
test('vm', () => {
  expect(runInThisContext('1 + 2')).toBe(3);
  runInThisContext('var __vmGlobal = 5');
  expect(globalThis.__vmGlobal).toBe(5);
  expect(new Script('6 * 7').runInThisContext()).toBe(42);
  expect(compileFunction('return a + b', ['a', 'b'])(1, 2)).toBe(3);
  expect(vm.runInThisContext).toBe(runInThisContext);
  expect(isContext({})).toBe(false);
  expect(() => isContext(1)).toThrow(TypeError);
  expect(() => createContext({})).toThrow('vm.createContext is not available');
  expect(() => vm.runInNewContext('1')).toThrow('vm.runInNewContext is not available');
  expect(() => new Script('1').runInContext({})).toThrow('vm.Script.runInContext is not available');
});
test('worker_threads', () => {
  expect([isMainThread, parentPort, threadId, workerData]).toEqual([true, null, 0, null]);
  expect(wt.resourceLimits).toEqual({});
  expect(() => new Worker('./w.js')).toThrow('worker_threads.Worker is not available');
  expect(MessageChannel).toBe(globalThis.MessageChannel);
  setEnvironmentData('k', 1);
  expect(getEnvironmentData('k')).toBe(1);
  const o = {};
  expect(wt.isMarkedAsUntransferable(o)).toBe(false);
  wt.markAsUntransferable(o);
  expect(wt.isMarkedAsUntransferable(o)).toBe(true);
});
test('querystring', () => {
  expect({ ...parse('a=1&a=2&b=x+y&c') }).toEqual({ a: ['1', '2'], b: 'x y', c: '' });
  expect(Object.getPrototypeOf(parse('a=1'))).toBe(null);
  expect({ ...parse('a:1;b:2', ';', ':') }).toEqual({ a: '1', b: '2' });
  expect(Object.keys(parse('a=1&b=2&c=3', null, null, { maxKeys: 2 }))).toEqual(['a', 'b']);
  expect(stringify({ a: [1, 2], b: 'x y', c: true, d: null, e: NaN })).toBe('a=1&a=2&b=x%20y&c=true&d=&e=');
  expect(escape('a b/ü')).toBe('a%20b%2F%C3%BC');
  expect(unescape('%E4%BD%A0')).toBe('你');
  expect(unescape('%E4%BD%zz')).toBe('�%zz');
  expect(qs.decode).toBe(parse);
  expect(qs.encode).toBe(stringify);
});
test('string_decoder', () => {
  const d = new StringDecoder('utf8');
  const euro = Buffer.from('€');
  expect(d.write(euro.subarray(0, 1))).toBe('');
  expect(d.write(euro.subarray(1))).toBe('€');
  expect(d.end(Buffer.from([0xe2]))).toBe('�');
  expect(new StringDecoder().write(Buffer.from([0xff, 0x41]))).toBe('�A');
  expect(new StringDecoder('hex').write(Buffer.from([255, 1]))).toBe('ff01');
  const b = new StringDecoder('base64');
  expect([b.write(Buffer.from([1, 2])), b.write(Buffer.from([3, 4])), b.end()]).toEqual(['', 'AQID', 'BA==']);
  const u = new StringDecoder('utf16le');
  expect([u.write(Buffer.from([0x3d, 0xd8])), u.write(Buffer.from([0x00, 0xde])), u.end()]).toEqual(['', '😀', '']);
  expect(new StringDecoder('latin1').end(Buffer.from([0xe9]))).toBe('é');
  expect(new StringDecoder('ascii').end(Buffer.from([0xe9]))).toBe('i');
  expect(new StringDecoder('UCS2').encoding).toBe('utf16le');
  expect(() => new StringDecoder('nope')).toThrow('Unknown encoding: nope');
  expect(sdMod.StringDecoder).toBe(StringDecoder);
});
test('constants', () => {
  expect([constants.F_OK, constants.R_OK, constants.W_OK, constants.X_OK]).toEqual([0, 4, 2, 1]);
  expect(constants.O_RDWR).toBe(fs.constants.O_RDWR);
  expect(constants.RSA_PKCS1_PADDING).toBe(1);
  expect(os.constants.priority.PRIORITY_HIGH).toBe(-14);
  if (process.platform !== 'win32') {
    expect([constants.SIGINT, constants.SIGTERM, os.constants.signals.SIGKILL]).toEqual([2, 15, 9]);
    expect([constants.ENOENT, os.constants.errno.EACCES]).toEqual([2, 13]);
  }
});
test('inspector', () => {
  expect(inspectorUrl()).toBe(undefined);
  expect(() => inspector.waitForDebugger()).toThrow('Inspector is not active');
  const s = new Session();
  expect(() => s.post('Runtime.evaluate')).toThrow('Session is not connected');
  expect(() => s.connect()).toThrow('inspector.Session.connect is not available');
  expect(inspector.console).toBe(globalThis.console);
});
test('assert and assert/strict', async () => {
  expect(strict).toBe(assertStrict);
  expect(assert.strict).toBe(assertStrict);
  assertStrict.equal(1, 1);
  expect(() => assertStrict.equal(1, '1')).toThrow(assertStrict.AssertionError);
  assert.equal(1, '1');
  assert.deepEqual({ a: 1 }, { a: '1' });
  expect(() => assertStrict.deepEqual({ a: 1 }, { a: '1' })).toThrow();
  assertStrict.deepStrictEqual(new Map([[1, { a: [1] }]]), new Map([[1, { a: [1] }]]));
  expect(() => assertStrict.deepEqual(new Date(1), new Date(2))).toThrow();
  assertStrict.throws(() => { throw new TypeError('bad thing'); }, TypeError);
  assertStrict.throws(() => { throw new TypeError('bad thing'); }, { name: 'TypeError', message: /bad/ });
  expect(() => assertStrict.throws(() => {})).toThrow('Missing expected exception.');
  await assertStrict.rejects(Promise.reject(new Error('r')), /r/);
  await assertStrict.doesNotReject(Promise.resolve(1));
  expect(() => assert(0)).toThrow('The expression evaluated to a falsy value');
  expect(() => assertStrict.ifError(new Error('e'))).toThrow('ifError got unwanted exception: e');
  assertStrict.match('abc', /b/);
  try { assertStrict.strictEqual(1, 2, 'custom'); } catch (e) {
    expect([e.name, e.code, e.message, e.actual, e.expected, e.operator, e.generatedMessage]).toEqual(['AssertionError', 'ERR_ASSERTION', 'custom', 1, 2, 'strictEqual', false]);
  }
});
"#;

#[test]
fn builtins_without_a_shim_import_with_node_exports_on_both_paths() {
    let p = Project::new("rc-builtins");
    p.file("package.json", r#"{"name":"rc-builtins","type":"module"}"#);
    p.file("b.test.js", BUILTINS_TEST);
    for esbuild in [false, true] {
        if esbuild {
            p.with_esbuild();
        }
        let r = turbo_test(&p.dir, &["b.test.js"], &[]);
        assert_eq!(r.code, Some(0), "esbuild={esbuild}\n{}", r.all());
        assert!(
            r.stdout.contains("Tests  12 passed (12)"),
            "esbuild={esbuild}\n{}",
            r.all()
        );
    }
}

/// turbo-test#83: `statSync` / `lstatSync` report the entry's real kind, size and times, throw
/// Node's ENOENT (or return undefined with `throwIfNoEntry: false`), `Dirent` is exported, and
/// the fs calls take a `file:` URL (Node 26 / vitest 4.1.12, environment node).
#[test]
fn stat_sync_reports_the_entry_and_fs_calls_take_urls() {
    let p = Project::new("rc-stat");
    p.file("package.json", r#"{"type":"module"}"#);
    p.file("sub/x.txt", "hi\n");
    p.file("sub/inner/.keep", "");
    std::os::unix::fs::symlink("x.txt", p.path("sub/link")).unwrap();
    std::fs::create_dir_all(p.path("sub/special")).unwrap();
    let fifo = std::process::Command::new("mkfifo")
        .arg(p.path("sub/special/fifo"))
        .status()
        .unwrap();
    assert!(fifo.success());
    let _socket = std::os::unix::net::UnixListener::bind(p.path("sub/special/sock")).unwrap();
    p.file(
        "a.test.ts",
        r#"import { expect, it } from 'vitest';
import * as fs from 'node:fs';
import { readdirSync, statSync, lstatSync, existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const dir = fileURLToPath(new URL('./sub', import.meta.url));
it('stat', () => {
  const f = statSync(new URL('./sub/x.txt', import.meta.url));
  expect([f.isFile(), f.isDirectory(), f.isSymbolicLink(), f.size]).toEqual([true, false, false, 3]);
  const d = statSync(dir + '/inner');
  expect([d.isFile(), d.isDirectory(), d.isSymbolicLink()]).toEqual([false, true, false]);
  expect([statSync(dir + '/link').isFile(), statSync(dir + '/link').isSymbolicLink()]).toEqual([true, false]);
  expect([lstatSync(dir + '/link').isSymbolicLink(), lstatSync(dir + '/link').isFile()]).toEqual([true, false]);
  expect([f.mtime instanceof Date, f.mtime.getTime() === Math.round(f.mtimeMs), f.mtimeMs > 1e12]).toEqual([true, true, true]);
  let err: any;
  try { statSync(dir + '/nope'); } catch (e) { err = e; }
  expect([err.message, err.code, err.errno, err.syscall, err.path]).toEqual([`ENOENT: no such file or directory, stat '${dir}/nope'`, 'ENOENT', -2, 'stat', `${dir}/nope`]);
  let lerr: any;
  try { lstatSync(dir + '/nope'); } catch (e) { lerr = e; }
  expect([lerr.message, lerr.syscall]).toEqual([`ENOENT: no such file or directory, lstat '${dir}/nope'`, 'lstat']);
  expect(statSync(dir + '/nope', { throwIfNoEntry: false })).toBeUndefined();
});
it('Dirent', () => {
  const [e] = readdirSync(dir, { withFileTypes: true });
  expect([e instanceof fs.Dirent, e.constructor.name, Object.keys(e)]).toEqual([true, 'Dirent', ['name', 'parentPath']]);
});
it('special files', () => {
  const kinds = (x) => ['isFile', 'isDirectory', 'isSymbolicLink', 'isFIFO', 'isSocket', 'isCharacterDevice', 'isBlockDevice'].filter((k) => x[k]());
  expect(readdirSync(dir + '/special', { withFileTypes: true }).map((e) => [e.name, kinds(e)])).toEqual([['fifo', ['isFIFO']], ['sock', ['isSocket']]]);
  expect([kinds(statSync(dir + '/special/fifo')), kinds(statSync(dir + '/special/sock')), kinds(statSync('/dev/null'))]).toEqual([['isFIFO'], ['isSocket'], ['isCharacterDevice']]);
  const s = statSync(dir + '/x.txt');
  expect([s instanceof fs.Stats, s.constructor.name, Object.keys(s).join()]).toEqual([true, 'Stats', 'dev,mode,nlink,uid,gid,rdev,blksize,ino,size,blocks,atimeMs,mtimeMs,ctimeMs,birthtimeMs']);
  expect([s.mode & 0o170000, s.nlink, s.blocks >= 0, s.atime.getTime() === Math.round(s.atimeMs), s.ctime.getTime() === Math.round(s.ctimeMs), s.birthtime.getTime() === Math.round(s.birthtimeMs)]).toEqual([0o100000, 1, true, true, true, true]);
});
it('fs/promises stat', async () => {
  const p = await import('node:fs/promises');
  expect((await p.stat(dir + '/inner')).isDirectory()).toBe(true);
  expect((await p.lstat(dir + '/link')).isSymbolicLink()).toBe(true);
  await expect(p.stat(dir + '/nope')).rejects.toMatchObject({ code: 'ENOENT', syscall: 'stat' });
});
it('URL arguments', () => {
  expect(existsSync(new URL('./sub/x.txt', import.meta.url))).toBe(true);
  expect(existsSync(new URL('./sub/no.txt', import.meta.url))).toBe(false);
  expect(readFileSync(new URL('./sub/x.txt', import.meta.url), 'utf8')).toBe('hi\n');
});
"#,
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("Tests  5 passed (5)"), "{}", r.all());
}
