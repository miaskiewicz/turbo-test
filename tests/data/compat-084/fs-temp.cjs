const test = globalThis.it || require('node:test');
const after = globalThis.afterAll || require('node:test').after;
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const base = path.join(process.cwd(), 'fs-temp-fixture');
fs.mkdirSync(base, { recursive: true });
after(() => fs.rmSync(base, { recursive: true, force: true }));

test('mkdtempSync creates distinct private directories with a six-character suffix', () => {
  const prefix = path.join(base, 'unique-');
  const a = fs.mkdtempSync(prefix), b = fs.mkdtempSync(prefix);
  assert.notEqual(a, b);
  for (const dir of [a, b]) {
    assert.equal(dir.startsWith(prefix), true);
    assert.match(dir.slice(prefix.length), /^[A-Za-z0-9]{6}$/);
    assert.equal(fs.statSync(dir).isDirectory(), true);
    if (process.platform !== 'win32') assert.equal(fs.statSync(dir).mode & 0o777, 0o700);
  }
});

test('mkdtempSync accepts Buffer and file URL prefixes and Buffer output', () => {
  const prefix = path.join(base, 'é-buffer-');
  const dir = fs.mkdtempSync(Buffer.from(prefix), { encoding: 'buffer' });
  assert.equal(Buffer.isBuffer(dir), true);
  assert.equal(dir.toString().startsWith(prefix), true);
  assert.equal(fs.statSync(dir).isDirectory(), true);
  const urlDir = fs.mkdtempSync(pathToFileURL(path.join(base, 'url-')), null);
  assert.equal(fs.statSync(urlDir).isDirectory(), true);
});

test('mkdtempSync returns the requested encoding and preserves relative prefixes', () => {
  const encoded = fs.mkdtempSync(path.join(base, 'hex-'), 'hex');
  const decoded = Buffer.from(encoded, 'hex').toString();
  assert.equal(fs.statSync(decoded).isDirectory(), true);
  const relative = fs.mkdtempSync('relative-fs-temp-');
  try { assert.equal(path.isAbsolute(relative), false); assert.equal(fs.statSync(relative).isDirectory(), true); }
  finally { fs.rmSync(relative, { recursive: true, force: true }); }
});

test('mkdtempSync validates input and keeps the OS error template path', () => {
  assert.throws(() => fs.mkdtempSync(42), { code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => fs.mkdtempSync('bad\0prefix'), { code: 'ERR_INVALID_ARG_VALUE' });
  assert.throws(() => fs.mkdtempSync('x-', { encoding: 'not-an-encoding' }), { code: 'ERR_INVALID_ARG_VALUE' });
  assert.throws(() => fs.mkdtempSync(path.join(base, 'invalid-'), 42), { code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => fs.mkdtempSync(path.join(base, 'invalid-'), true), { code: 'ERR_INVALID_ARG_TYPE' });
  assert.throws(() => fs.mkdtempSync(path.join(base, 'invalid-'), { encoding: 42 }), { code: 'ERR_INVALID_ARG_VALUE' });
  const prefix = path.join(base, 'missing-parent', 'prefix-');
  assert.throws(() => fs.mkdtempSync(prefix), { code: 'ENOENT', syscall: 'mkdtemp', path: prefix + 'XXXXXX' });
});

test('promises.mkdtemp creates real directories and rejects OS failures', async () => {
  const dir = await fs.promises.mkdtemp(path.join(base, 'promise-'), { encoding: 'buffer' });
  assert.equal(Buffer.isBuffer(dir), true);
  assert.equal(fs.statSync(dir).isDirectory(), true);
  const missing = path.join(base, 'missing-parent', 'promise-');
  await assert.rejects(fs.promises.mkdtemp(missing), (error) => error.code === 'ENOENT' && error.syscall === 'mkdtemp' && error.path.startsWith(missing) && /^[A-Za-z0-9]{6}$/.test(error.path.slice(missing.length)));
});

test('callback mkdtemp delivers asynchronous success and OS errors', async () => {
  let returned = false;
  await new Promise((resolve, reject) => {
    const result = fs.mkdtemp(path.join(base, 'callback-'), (error, dir) => {
      try { assert.equal(returned, true); assert.equal(error, null); assert.equal(fs.statSync(dir).isDirectory(), true); resolve(); }
      catch (failure) { reject(failure); }
    });
    assert.equal(result, undefined); returned = true;
  });
  const missing = path.join(base, 'missing-parent', 'callback-');
  await new Promise((resolve, reject) => fs.mkdtemp(missing, (error, dir) => {
    try { assert.equal(error.code, 'ENOENT'); assert.equal(error.path.startsWith(missing), true); assert.match(error.path.slice(missing.length), /^[A-Za-z0-9]{6}$/); assert.equal(dir, undefined); resolve(); }
    catch (failure) { reject(failure); }
  }));
  assert.throws(() => fs.mkdtemp(path.join(base, 'no-callback-')), { code: 'ERR_INVALID_ARG_TYPE' });
});
