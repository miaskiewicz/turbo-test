//! The callback fs API (turbo-test#433): fs.readFile / stat / open / read / write / ... with
//! Node's callback arguments and timing (after the current tick), errors in the callback,
//! argument errors thrown at the call, plus the descriptor *Sync forms they share.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const CASES: &str = r#"
const fs = require('fs');
const path = require('path');
const os = require('os');
let dir;
beforeEach(() => { dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fscb-')); });
afterEach(() => { fs.rmSync(dir, { recursive: true, force: true }); });
// resolves with the callback's arguments
const call = (fn, ...args) => new Promise((resolve) => fn(...args, (...result) => resolve(result)));
const codeOf = (f) => { try { f(); } catch (e) { return e.name + ' ' + e.code; } return 'returned'; };
test('callbacks run asynchronously, after the current tick and its promise jobs', async () => {
 const f = path.join(dir, 'f'); fs.writeFileSync(f, 'hi');
 const order = [];
 const done = new Promise((resolve) => fs.readFile(f, 'utf8', (err, data) => { order.push('callback:' + data); resolve(); }));
 process.nextTick(() => order.push('nextTick'));
 Promise.resolve().then(() => order.push('promise'));
 order.push('sync');
 await done;
 expect(order[0]).toBe('sync');
 expect(order.slice(1, 3).sort()).toEqual(['nextTick', 'promise']);
 expect(order[3]).toBe('callback:hi');
});
test('read-side callbacks pass (err, value) with Node\'s overloads', async () => {
 const f = path.join(dir, 'f'); fs.writeFileSync(f, 'hello');
 const [err, buf] = await call(fs.readFile, f);
 expect([err, Buffer.isBuffer(buf), String(buf)]).toEqual([null, true, 'hello']);
 expect(await call(fs.readFile, f, 'utf8')).toEqual([null, 'hello']);
 expect(await call(fs.readFile, f, { encoding: 'utf8' })).toEqual([null, 'hello']);
 const [serr, st] = await call(fs.stat, f);
 expect([serr, st.size, st.isFile()]).toEqual([null, 5, true]);
 const [, lst] = await call(fs.lstat, f, {});
 expect(lst.isFile()).toBe(true);
 expect(await call(fs.readdir, dir)).toEqual([null, ['f']]);
 const [, dirents] = await call(fs.readdir, dir, { withFileTypes: true });
 expect([dirents[0].name, dirents[0].isFile()]).toEqual(['f', true]);
 expect(await call(fs.realpath, f)).toEqual([null, fs.realpathSync(f)]);
 expect(await call(fs.realpath.native, f)).toEqual([null, fs.realpathSync(f)]);
 expect(await new Promise((resolve) => fs.exists(f, resolve))).toBe(true);
 expect(await new Promise((resolve) => fs.exists(path.join(dir, 'nope'), resolve))).toBe(false);
 expect(await call(fs.access, f)).toEqual([null]);
 expect(await call(fs.access, f, fs.constants.R_OK)).toEqual([null]);
});
test('write-side callbacks get a null error and change the file system', async () => {
 const f = path.join(dir, 'f');
 expect(await call(fs.writeFile, f, 'abc')).toEqual([null]);
 expect(await call(fs.appendFile, f, 'def', 'utf8')).toEqual([null]);
 expect(fs.readFileSync(f, 'utf8')).toBe('abcdef');
 expect(await call(fs.writeFile, f, Buffer.from('xyz'), { encoding: 'utf8' })).toEqual([null]);
 expect(fs.readFileSync(f, 'utf8')).toBe('xyz');
 expect(await call(fs.mkdir, path.join(dir, 'a'))).toEqual([null]);
 expect(await call(fs.mkdir, path.join(dir, 'b', 'c'), { recursive: true })).toEqual([null, path.join(dir, 'b')]);
 expect(await call(fs.chmod, f, 0o600)).toEqual([null]);
 expect(fs.statSync(f).mode & 0o777).toBe(0o600);
 expect(await call(fs.rename, f, path.join(dir, 'g'))).toEqual([null]);
 expect(await call(fs.symlink, path.join(dir, 'g'), path.join(dir, 'link'))).toEqual([null]);
 expect(fs.readFileSync(path.join(dir, 'link'), 'utf8')).toBe('xyz');
 expect(await call(fs.unlink, path.join(dir, 'link'))).toEqual([null]);
 expect(await call(fs.rmdir, path.join(dir, 'a'))).toEqual([null]);
 expect(await call(fs.rm, path.join(dir, 'b'), { recursive: true })).toEqual([null]);
 expect(fs.readdirSync(dir)).toEqual(['g']);
});
test('failures reach the callback with Node\'s error; bad arguments throw synchronously', async () => {
 const missing = path.join(dir, 'nope');
 const [err] = await call(fs.readFile, missing);
 expect([err.code, err.errno, err.syscall, err.path, err.message]).toEqual(['ENOENT', -2, 'open', missing, `ENOENT: no such file or directory, open '${missing}'`]);
 const [serr, st] = await call(fs.stat, missing);
 expect([serr.code, serr.syscall, st]).toEqual(['ENOENT', 'stat', undefined]);
 const [aerr] = await call(fs.access, missing);
 expect([aerr.code, aerr.syscall]).toEqual(['ENOENT', 'access']);
 const [rerr] = await call(fs.rename, missing, path.join(dir, 'x'));
 expect([rerr.code, rerr.dest]).toEqual(['ENOENT', path.join(dir, 'x')]);
 const [derr] = await call(fs.rm, dir);
 expect(derr.code).toBe('ERR_FS_EISDIR');
 expect(codeOf(() => fs.readFile(missing))).toBe('TypeError ERR_INVALID_ARG_TYPE');
 expect(codeOf(() => fs.stat(missing, {}))).toBe('TypeError ERR_INVALID_ARG_TYPE');
 expect(codeOf(() => fs.stat({}, () => {}))).toBe('TypeError ERR_INVALID_ARG_TYPE');
 expect(codeOf(() => fs.access({}, () => {}))).toBe('TypeError ERR_INVALID_ARG_TYPE');
 expect(codeOf(() => fs.exists(missing))).toBe('TypeError ERR_INVALID_ARG_TYPE');
 expect(codeOf(() => fs.access(missing, 99, () => {}))).toBe('RangeError ERR_OUT_OF_RANGE');
});
test('descriptor callbacks and their Sync forms read and write at positions', async () => {
 const f = path.join(dir, 'f');
 const [oerr, fd] = await call(fs.open, f, 'w+');
 expect([oerr, Number.isInteger(fd), fd > 2]).toEqual([null, true, true]);
 const [werr, written, str] = await call(fs.write, fd, 'abcdef');
 expect([werr, written, str]).toEqual([null, 6, 'abcdef']);
 const xy = Buffer.from('XY');
 const [, n2, b2] = await call(fs.write, fd, xy, 0, 2, 1);
 expect([n2, b2 === xy]).toEqual([2, true]);
 const buf = Buffer.alloc(10);
 const [rerr, bytesRead, rb] = await call(fs.read, fd, buf, 0, 10, 0);
 expect([rerr, bytesRead, rb === buf, buf.toString('utf8', 0, 6)]).toEqual([null, 6, true, 'aXYdef']);
 const [, n3] = await call(fs.read, fd, { buffer: buf, position: 2 });
 expect([n3, buf.toString('utf8', 0, 4)]).toEqual([4, 'Ydef']);
 const [, n4] = await call(fs.read, fd, buf, { position: 0, length: 1 });
 expect([n4, buf.toString('utf8', 0, 1)]).toEqual([1, 'a']);
 const [, st] = await call(fs.fstat, fd);
 expect(st.size).toBe(6);
 expect(await call(fs.ftruncate, fd, 3)).toEqual([null]);
 expect(await call(fs.fsync, fd)).toEqual([null]);
 expect(await call(fs.fdatasync, fd)).toEqual([null]);
 expect(await call(fs.fchmod, fd, 0o640)).toEqual([null]);
 expect(fs.statSync(f).mode & 0o777).toBe(0o640);
 expect(await call(fs.close, fd)).toEqual([null]);
 const [cerr] = await call(fs.fstat, fd);
 expect([cerr.code, cerr.syscall, cerr.message]).toEqual(['EBADF', 'fstat', 'EBADF: bad file descriptor, fstat']);
 expect(fs.readFileSync(f, 'utf8')).toBe('aXY');
 const sfd = fs.openSync(f, 'r+');
 expect(fs.writeSync(sfd, 'Z', 0)).toBe(1);
 expect(fs.writeSync(sfd, Buffer.from('QQ'), 0, 2, 1)).toBe(2);
 const sb = Buffer.alloc(3);
 expect(fs.readSync(sfd, sb, 0, 3, 0)).toBe(3);
 expect(sb.toString()).toBe('ZQQ');
 expect(fs.readSync(sfd, sb, { position: 1, length: 1 })).toBe(1);
 expect(fs.fstatSync(sfd).size).toBe(3);
 fs.ftruncateSync(sfd, 1); fs.fsyncSync(sfd); fs.fdatasyncSync(sfd); fs.fchmodSync(sfd, 0o600);
 fs.closeSync(sfd);
 expect([fs.readFileSync(f, 'utf8'), fs.statSync(f).mode & 0o777]).toEqual(['Z', 0o600]);
 const [eerr] = await call(fs.open, path.join(dir, 'nope'), 'r');
 expect([eerr.code, eerr.syscall]).toEqual(['ENOENT', 'open']);
 expect(codeOf(() => fs.openSync(f, 'zz'))).toBe('TypeError ERR_INVALID_ARG_VALUE');
 expect(codeOf(() => fs.fstatSync(-1))).toBe('RangeError ERR_OUT_OF_RANGE');
 expect(codeOf(() => fs.closeSync('1'))).toBe('TypeError ERR_INVALID_ARG_TYPE');
});
"#;
#[test]
fn fs_callback_api_matches_node() {
    let p = Project::new("fs-callbacks-433");
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("fs.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
