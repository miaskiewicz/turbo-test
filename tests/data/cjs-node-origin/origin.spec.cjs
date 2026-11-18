const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const consumer = require('native-caller');
testCase('backend dependency CallSites expose the real module filename', () => {
  assert.equal(consumer.caller, consumer.filename);
  assert.equal(require('node:path').isAbsolute(consumer.caller), true);
  assert.equal(consumer.caller.endsWith('/native-caller/index.cjs'), true);
});
