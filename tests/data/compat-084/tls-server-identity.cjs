const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const tls = require('node:tls');
const net = require('node:net');
const http = require('node:http');

testCase('TLS Server constructor aliases support supertest type discrimination', () => {
  assert.equal(typeof tls.Server, 'function');
  assert.equal(require('tls').Server, tls.Server);
  const app = () => {};
  const server = http.createServer(app);
  assert.equal(app instanceof tls.Server, false);
  assert.equal(server instanceof tls.Server, false);
  assert.equal(server instanceof http.Server, true);
});

testCase('TLS Server has its own extendable identity over net Server', () => {
  class Derived extends tls.Server {}
  assert.equal(Object.getPrototypeOf(tls.Server.prototype), net.Server.prototype);
  assert.equal(tls.Server.prototype.constructor, tls.Server);
  assert.equal(Object.getPrototypeOf(Derived.prototype), tls.Server.prototype);
  assert.notEqual(tls.Server, http.Server);
});

testCase('TLS default and named imports retain the same Server constructor', async () => {
  const bare = await import('tls');
  const prefixed = await import('node:tls');
  assert.equal(bare.Server, tls.Server);
  assert.equal(prefixed.Server, tls.Server);
  assert.equal(bare.default.Server, tls.Server);
});
