const test = globalThis.it || require('node:test');
const after = globalThis.afterAll || require('node:test').after;
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const base = path.join(process.cwd(), 'fs-copy-fixture');
fs.mkdirSync(base, { recursive: true });
after(() => fs.rmSync(base, { recursive: true, force: true }));
const source = path.join(base, 'source.json');
const bytes = Buffer.from([0, 1, 127, 128, 255, 10]);
fs.writeFileSync(source, bytes);

test('copyFileSync preserves binary bytes and overwrites existing destinations', () => {
  const dest = path.join(base, 'copy.bin');
  assert.equal(fs.copyFileSync(source, dest), undefined);
  assert.deepEqual(fs.readFileSync(dest), bytes);
  fs.writeFileSync(dest, 'a longer old destination');
  fs.copyFileSync(source, dest, fs.constants.COPYFILE_FICLONE);
  assert.deepEqual(fs.readFileSync(dest), bytes);
});

test('copyFileSync accepts Buffer and file URL paths', () => {
  const dest = path.join(base, 'é-url.bin');
  fs.copyFileSync(Buffer.from(source), pathToFileURL(dest));
  assert.deepEqual(fs.readFileSync(dest), bytes);
});

test('COPYFILE_EXCL preserves the destination and reports both paths', () => {
  const dest = path.join(base, 'existing.bin'); fs.writeFileSync(dest, 'keep');
  assert.throws(() => fs.copyFileSync(source, dest, fs.constants.COPYFILE_EXCL), { code: 'EEXIST', syscall: 'copyfile', path: source, dest });
  assert.equal(fs.readFileSync(dest, 'utf8'), 'keep');
  const missing = path.join(base, 'missing-source');
  assert.throws(() => fs.copyFileSync(missing, dest, fs.constants.COPYFILE_EXCL), { code: 'ENOENT', syscall: 'copyfile', path: missing, dest });
});

test('copying the same file is a no-op unless COPYFILE_EXCL is requested', () => {
  fs.copyFileSync(source, source);
  assert.deepEqual(fs.readFileSync(source), bytes);
  assert.throws(() => fs.copyFileSync(source, source, fs.constants.COPYFILE_EXCL), { code: 'EEXIST' });
  assert.deepEqual(fs.readFileSync(source), bytes);
});

test('copyFileSync preserves OS errors and validates modes and path types', () => {
  const dest = path.join(base, 'missing-parent', 'target.bin');
  assert.throws(() => fs.copyFileSync(source, dest), { code: 'ENOENT', syscall: 'copyfile', path: source, dest });
  const directoryCopy = path.join(base, 'directory-copy-target');
  const directoryCode = process.platform === 'darwin' ? 'ENOTSUP' : 'EISDIR';
  assert.throws(() => fs.copyFileSync(base, directoryCopy), { code: directoryCode, syscall: 'copyfile', path: base, dest: directoryCopy });
  assert.throws(() => fs.copyFileSync(source, base), { code: 'EISDIR', syscall: 'copyfile', path: source, dest: base });
  assert.throws(() => fs.copyFileSync(source, 'no-file', 8), { code: 'ERR_OUT_OF_RANGE' });
  assert.throws(() => fs.copyFileSync(source, 'no-file', true), { code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => fs.copyFileSync(42, 'no-file'), { code: 'ERR_INVALID_ARG_TYPE' });
});

test('promise and callback copyFile perform real copies and asynchronous errors', async () => {
  const dest = path.join(base, 'promise.bin');
  await fs.promises.copyFile(source, dest);
  assert.deepEqual(fs.readFileSync(dest), bytes);
  await assert.rejects(fs.promises.copyFile(source, dest, fs.constants.COPYFILE_EXCL), { code: 'EEXIST' });
  let returned = false;
  await new Promise((resolve, reject) => {
    const result = fs.copyFile(source, path.join(base, 'callback.bin'), (error) => {
      try { assert.equal(returned, true); assert.equal(error, null); resolve(); } catch (failure) { reject(failure); }
    });
    assert.equal(result, undefined); returned = true;
  });
  await new Promise((resolve, reject) => fs.copyFile(source, dest, fs.constants.COPYFILE_EXCL, (error) => {
    try { assert.equal(error.code, 'EEXIST'); assert.equal(error.path, source); assert.equal(error.dest, dest); resolve(); } catch (failure) { reject(failure); }
  }));
  assert.throws(() => fs.copyFile(source, dest), { code: 'ERR_INVALID_ARG_TYPE' });
});
