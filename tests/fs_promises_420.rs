//! fs.promises.access / chmod / rename / realpath / open and its FileHandle (turbo-test#420):
//! real syscalls with Node's results and errors (code, errno, syscall, path, dest, message), so
//! mongodb-memory-server's `fs.promises.access` probe of its binary directory works.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const CASES: &str = r#"
const fs = require('fs');
const p = require('fs').promises;
const path = require('path');
const os = require('os');
let dir;
beforeEach(() => { dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fsp-')); });
afterEach(() => { fs.rmSync(dir, { recursive: true, force: true }); });
const fail = async (promise) => { try { await promise; } catch (e) { return { name: e.name, code: e.code, errno: e.errno, syscall: e.syscall, path: e.path, dest: e.dest, message: e.message }; } return 'resolved'; };
const failSync = (f) => { try { f(); } catch (e) { return e.name + ' ' + e.code; } return 'returned'; };
const mode = (f) => fs.statSync(f).mode & 0o777;
test('access resolves for a permitted mode and rejects with Node\'s errors', async () => {
 const f = path.join(dir, 'f'); fs.writeFileSync(f, 'hello');
 expect(p.access).toBe(fs.promises.access);
 expect(require('fs/promises').access).toBe(p.access);
 await expect(p.access(f)).resolves.toBeUndefined();
 await expect(p.access(f, fs.constants.R_OK | fs.constants.W_OK)).resolves.toBeUndefined();
 const missing = path.join(dir, 'nope');
 expect(await fail(p.access(missing))).toEqual({ name: 'Error', code: 'ENOENT', errno: -2, syscall: 'access', path: missing, dest: undefined, message: `ENOENT: no such file or directory, access '${missing}'` });
 expect(await fail(p.access(f, fs.constants.X_OK))).toEqual({ name: 'Error', code: 'EACCES', errno: -13, syscall: 'access', path: f, dest: undefined, message: `EACCES: permission denied, access '${f}'` });
 expect((await fail(p.access(f, 99))).code).toBe('ERR_OUT_OF_RANGE');
 expect(fs.accessSync(f, fs.constants.R_OK)).toBeUndefined();
 expect(failSync(() => fs.accessSync(missing))).toBe('Error ENOENT');
});
test('chmod sets the mode from a number or an octal string', async () => {
 const f = path.join(dir, 'f'); fs.writeFileSync(f, 'x');
 await expect(p.chmod(f, 0o755)).resolves.toBeUndefined();
 expect(mode(f)).toBe(0o755);
 await p.access(f, fs.constants.X_OK);
 await p.chmod(f, '640');
 expect(mode(f)).toBe(0o640);
 fs.chmodSync(f, 0o600);
 expect(mode(f)).toBe(0o600);
 const missing = path.join(dir, 'nope');
 expect(await fail(p.chmod(missing, 0o644))).toEqual({ name: 'Error', code: 'ENOENT', errno: -2, syscall: 'chmod', path: missing, dest: undefined, message: `ENOENT: no such file or directory, chmod '${missing}'` });
 expect((await fail(p.chmod(f, '9'))).code).toBe('ERR_INVALID_ARG_VALUE');
});
test('rename moves a file and reports both paths on failure', async () => {
 const f = path.join(dir, 'f'), g = path.join(dir, 'g'); fs.writeFileSync(f, 'hello');
 await expect(p.rename(f, g)).resolves.toBeUndefined();
 expect([fs.existsSync(f), fs.readFileSync(g, 'utf8')]).toEqual([false, 'hello']);
 fs.renameSync(g, f);
 expect(fs.readFileSync(f, 'utf8')).toBe('hello');
 const missing = path.join(dir, 'nope'), to = path.join(dir, 'x');
 expect(await fail(p.rename(missing, to))).toEqual({ name: 'Error', code: 'ENOENT', errno: -2, syscall: 'rename', path: missing, dest: to, message: `ENOENT: no such file or directory, rename '${missing}' -> '${to}'` });
});
test('realpath resolves through the real directory', async () => {
 const f = path.join(dir, 'f'); fs.writeFileSync(f, '');
 const real = fs.realpathSync(dir);
 expect(await p.realpath(path.join(dir, '..', path.basename(dir), 'f'))).toBe(path.join(real, 'f'));
 expect(Buffer.isBuffer(await p.realpath(f, 'buffer'))).toBe(true);
 const missing = path.join(dir, 'nope');
 expect(await fail(p.realpath(missing))).toEqual({ name: 'Error', code: 'ENOENT', errno: -2, syscall: 'realpath', path: missing, dest: undefined, message: `ENOENT: no such file or directory, realpath '${missing}'` });
});
test('open returns a FileHandle that reads and writes at positions', async () => {
 const f = path.join(dir, 'h');
 const h = await p.open(f, 'w+');
 expect([typeof h.fd, h.fd > 2, h.constructor.name]).toEqual(['number', true, 'FileHandle']);
 expect(await h.write('abcdef')).toEqual({ __proto__: null, bytesWritten: 6, buffer: 'abcdef' });
 const xy = Buffer.from('XY');
 const w = await h.write(xy, 0, 2, 1);
 expect([w.bytesWritten, w.buffer === xy]).toEqual([2, true]);
 const b = Buffer.alloc(10);
 const r = await h.read(b, 0, 10, 0);
 expect([r.bytesRead, r.buffer === b, b.toString('utf8', 0, 6)]).toEqual([6, true, 'aXYdef']);
 expect(await h.readFile({ encoding: 'utf8' })).toBe('');
 expect((await h.write(Buffer.alloc(0))).bytesWritten).toBe(0);
 expect((await h.read(Buffer.alloc(0), 0, 0, 0)).bytesRead).toBe(0);
 expect((await h.stat()).size).toBe(6);
 expect((await h.stat()).isFile()).toBe(true);
 await h.truncate(3);
 expect(fs.readFileSync(f, 'utf8')).toBe('aXY');
 await h.chmod(0o640);
 expect(mode(f)).toBe(0o640);
 await h.sync(); await h.datasync();
 await expect(h.close()).resolves.toBeUndefined();
 expect(h.fd).toBe(-1);
 await expect(h.close()).resolves.toBeUndefined();
 const closed = await fail(h.read(b, 0, 1, 0));
 expect([closed.code, closed.syscall, closed.message]).toEqual(['EBADF', 'read', 'file closed']);
});
test('a FileHandle reads with options, the whole file and appends', async () => {
 const f = path.join(dir, 'g'); fs.writeFileSync(f, 'hello');
 const r = await p.open(f);
 expect(await r.readFile('utf8')).toBe('hello');
 const rest = await r.read();
 expect([rest.bytesRead, rest.buffer.length]).toEqual([0, 16384]);
 const b = Buffer.alloc(5);
 expect((await r.read({ buffer: b, position: 1 })).bytesRead).toBe(4);
 expect(b.toString('utf8', 0, 4)).toBe('ello');
 expect((await r.read(b, { offset: 1, length: 2, position: 0 })).bytesRead).toBe(2);
 expect(b.toString('utf8', 0, 4)).toBe('eheo');
 const ro = await fail(r.write('x'));
 expect([ro.code, ro.errno, ro.syscall, ro.message]).toEqual(['EBADF', -9, 'write', 'EBADF: bad file descriptor, write']);
 await r.close();
 const a = await p.open(f, 'a');
 await a.appendFile('!'); await a.writeFile(Buffer.from('?'));
 await a[Symbol.asyncDispose]();
 expect(a.fd).toBe(-1);
 expect(fs.readFileSync(f, 'utf8')).toBe('hello!?');
 const latin = await p.open(path.join(dir, 'l'), 'w', 0o600);
 expect((await latin.write('héllo', 0, 'latin1')).bytesWritten).toBe(5);
 await latin.close();
 expect(mode(path.join(dir, 'l'))).toBe(0o600);
 expect(fs.readFileSync(path.join(dir, 'l')).length).toBe(5);
});
test('open reports Node\'s errors for missing, existing and directory paths', async () => {
 const missing = path.join(dir, 'nope');
 expect(await fail(p.open(missing, 'r'))).toEqual({ name: 'Error', code: 'ENOENT', errno: -2, syscall: 'open', path: missing, dest: undefined, message: `ENOENT: no such file or directory, open '${missing}'` });
 const f = path.join(dir, 'f'); fs.writeFileSync(f, '');
 expect(await fail(p.open(f, 'wx'))).toEqual({ name: 'Error', code: 'EEXIST', errno: -17, syscall: 'open', path: f, dest: undefined, message: `EEXIST: file already exists, open '${f}'` });
 const bad = await fail(p.open(f, 'zz'));
 expect([bad.name, bad.code, bad.message]).toEqual(['TypeError', 'ERR_INVALID_ARG_VALUE', "The argument 'flags' is invalid. Received 'zz'"]);
 const d = await p.open(dir, fs.constants.O_RDONLY);
 expect((await d.stat()).isDirectory()).toBe(true);
 const isDir = await fail(d.readFile());
 expect([isDir.code, isDir.errno, isDir.syscall, isDir.message]).toEqual(['EISDIR', -21, 'read', 'EISDIR: illegal operation on a directory, read']);
 await d.close();
 const wo = await p.open(f, 'w');
 const wr = await fail(wo.read(Buffer.alloc(2), 0, 2, 0));
 expect([wr.code, wr.syscall]).toEqual(['EBADF', 'read']);
 await wo.close();
});
"#;
#[test]
fn fs_promises_access_chmod_rename_realpath_open_match_node() {
    let p = Project::new("fs-promises-420");
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("fs.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 7, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
