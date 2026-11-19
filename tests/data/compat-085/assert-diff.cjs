// A failing node:assert deep-equality reports its message and diff (turbo-test#403's
// diff-sequences loop exhausted the heap on these reordered lines and aborted the file).
// The test fails by design, under node --test too: the harness checks how it is reported.
const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');

const actual = ['_write a buffer', 'write a -> true', 'write b -> true', 'destroyed true', 'm:micro', 'event close',
  'm:tick', 'release a', 'cb a ok', 'cb b ERR_STREAM_DESTROYED', 'released', 'n:micro', 'n:tick'];
const expected = ['_write a buffer', 'write a -> true', 'write b -> true', 'destroyed true', 'event close', 'm:tick',
  'm:micro', 'release a', 'cb a ok', 'cb b ERR_STREAM_DESTROYED', 'released', 'n:tick', 'n:micro'];

testCase('reordered lines fail with a diff', () => {
  assert.deepStrictEqual(actual, expected);
});

testCase('the file keeps running after the failure', () => {
  assert.equal(actual.length, expected.length);
});
