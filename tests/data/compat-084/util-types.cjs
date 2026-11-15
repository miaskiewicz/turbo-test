const test = globalThis.it || require('node:test');
const assert = require('node:assert/strict');
const types = require('util/types');

test('util/types aliases preserve util.types object identity', () => {
  assert.equal(types, require('node:util/types'));
  assert.equal(types, require('util').types);
  assert.equal(types, require('node:util').types);
});
test('isDate tests the real internal Date brand including invalid dates', () => {
  assert.equal(types.isDate(new Date(0)), true);
  assert.equal(types.isDate(new Date(NaN)), true);
  class SubDate extends Date {}
  assert.equal(types.isDate(new SubDate(0)), true);
  for (const value of [null, undefined, 0, 'date', {}, Object.create(Date.prototype), { [Symbol.toStringTag]: 'Date' }, new Proxy(new Date(), {})]) {
    assert.equal(types.isDate(value), false);
  }
});
