import * as request from 'callable-package';
import defaultRequest from 'callable-package';
import * as cycle from './cycle-a';
const testCase = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const interop = require('./expected.json').interop;
testCase('namespace imports follow the configured CommonJS TypeScript interop', () => {
  assert.equal(typeof request, interop ? 'object' : 'function');
  const factory = interop ? request.default : request;
  assert.equal(factory, require('callable-package'));
  assert.equal(defaultRequest, interop ? factory : undefined);
  assert.deepEqual(factory('payload'), { value: 'payload', marker: 42 });
});
testCase('cyclic TypeScript imports retain late property reads and module identity', () => {
  assert.equal(cycle.readValue(), 42);
  assert.equal(interop ? cycle.request.default : cycle.request, interop ? request.default : request);
});
