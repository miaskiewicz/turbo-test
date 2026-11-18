const assert = require('node:assert/strict');
const tls = require('node:tls');

test('TLS server construction reports its unsupported transport without calling listeners', () => {
  let called = false;
  const listener = () => { called = true; };
  const unsupported = { code: 'ERR_TLS_UNSUPPORTED_SERVER', message: /TLS server transport is not supported/ };
  assert.throws(() => new tls.Server({}, listener), unsupported);
  assert.throws(() => tls.Server({}, listener), unsupported);
  assert.throws(() => tls.createServer({}, listener), unsupported);
  assert.equal(called, false);
});

test('subclassing a TLS Server does not make its transport available', () => {
  class Derived extends tls.Server {}
  assert.throws(() => new Derived(), { code: 'ERR_TLS_UNSUPPORTED_SERVER' });
});
