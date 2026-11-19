// A DOM-environment test file can use Node's backend built-ins, as under vitest's jsdom
// (regression since 0.8.4 / #390: they threw "requires the Node backend runtime"). Real packages
// such as @playwright/test require tty at load. Also passes under node --test.
const test = globalThis.test || require('node:test');
const assert = require('node:assert/strict');

test('tty, zlib and child_process load and work in a DOM file', () => {
  const tty = require('tty');
  assert.equal(typeof tty.isatty, 'function');
  assert.equal(typeof tty.isatty(1), 'boolean');
  const zlib = require('node:zlib');
  assert.equal(zlib.gunzipSync(zlib.gzipSync(Buffer.from('dom'))).toString(), 'dom');
  const { spawnSync } = require('child_process');
  assert.equal(spawnSync('echo', ['hi']).stdout.toString(), 'hi\n');
});

test('a crypto function captured before first use works', () => {
  const { createCipheriv } = require('node:crypto');
  const cipher = createCipheriv('aes-256-gcm', Buffer.alloc(32), Buffer.alloc(12));
  const ciphertext = Buffer.concat([cipher.update(Buffer.alloc(16)), cipher.final()]);
  assert.equal(ciphertext.toString('hex'), 'cea7403d4d606b6e074ec5d3baf39d18');
});

test('the DOM is still there', () => {
  if (typeof document === 'undefined') return; // node --test has no DOM
  const element = document.createElement('p');
  element.textContent = 'still DOM';
  assert.equal(element.textContent, 'still DOM');
});
