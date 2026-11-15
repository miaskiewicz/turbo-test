const test = globalThis.it || require('node:test');
const after = globalThis.afterAll || require('node:test').after;
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const base = path.join(process.cwd(), 'realpath-fixture');
fs.mkdirSync(path.join(base, 'target'), { recursive: true });
fs.writeFileSync(path.join(base, 'target', 'é.txt'), 'real bytes');
fs.symlinkSync('target', path.join(base, 'alias'));
after(() => fs.rmSync(base, { recursive: true, force: true }));
const input = path.join(base, 'alias', 'é.txt');
const expected = path.join(base, 'target', 'é.txt');

test('realpathSync and native resolve symlinks to the real absolute path', () => {
  for (const realpath of [fs.realpathSync, fs.realpathSync.native]) {
    assert.equal(realpath(input), expected);
    assert.equal(realpath(path.relative(process.cwd(), input)), expected);
    assert.equal(fs.readFileSync(realpath(input), 'utf8'), 'real bytes');
  }
});
test('realpath accepts Buffer and file URL paths and encodes output', () => {
  for (const realpath of [fs.realpathSync, fs.realpathSync.native]) {
    assert.equal(realpath(Buffer.from(input)), expected);
    assert.equal(realpath(pathToFileURL(input)), expected);
    assert.deepEqual(realpath(input, { encoding: 'buffer' }), Buffer.from(expected));
    assert.equal(realpath(input, 'hex'), Buffer.from(expected).toString('hex'));
  }
});
test('realpath errors retain a real errno, operation, and original path', () => {
  const missing = path.join(base, 'missing');
  for (const realpath of [fs.realpathSync, fs.realpathSync.native]) {
    assert.throws(() => realpath(missing), { code: 'ENOENT', path: missing });
    assert.throws(() => realpath(input + '/child'), { code: 'ENOTDIR' });
  }
});
test('realpath rejects invalid paths and invalid encodings before filesystem use', () => {
  for (const realpath of [fs.realpathSync, fs.realpathSync.native]) {
    assert.throws(() => realpath(42), { code: 'ERR_INVALID_ARG_TYPE' });
    assert.throws(() => realpath('bad\0path'), { code: 'ERR_INVALID_ARG_VALUE' });
    assert.throws(() => realpath(input, 'nonsense'), { code: 'ERR_INVALID_ARG_VALUE' });
  }
});
