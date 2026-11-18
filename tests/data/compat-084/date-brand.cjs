const testCase = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
testCase('the active Date constructor retains the standard public name descriptor', () => {
  assert.equal(Date.name, 'Date');
  const descriptor = Object.getOwnPropertyDescriptor(Date, 'name');
  assert.equal(descriptor.writable, false); assert.equal(descriptor.enumerable, false); assert.equal(descriptor.configurable, true);
});
testCase('Date facade retains native date values argument overloads and call behavior', () => {
  const value = new Date('2024-03-01T12:00:00.000Z');
  assert.equal(Object.prototype.toString.call(value), '[object Date]'); assert.equal(value instanceof Date, true);
  assert.equal(value.toISOString(), '2024-03-01T12:00:00.000Z');
  assert.equal(Date.UTC(2024, 2, 1), Date.parse('2024-03-01T00:00:00.000Z'));
  assert.equal(typeof Date(), 'string'); assert.equal(new Date(NaN).toString(), 'Invalid Date');
});
