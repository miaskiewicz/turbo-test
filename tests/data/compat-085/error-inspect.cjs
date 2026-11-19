// util.inspect / util.format('%s') of errors whose name or stack differ from the default, as
// Node v26.11.0 prints them (improveStack: `Ctor [name]` when a stack-header name ending in
// "Error" differs from the constructor; brackets around an error without stack frames).
// Also passes under node --test.
const test = globalThis.test || require('node:test');
const assert = require('node:assert/strict');
const util = require('node:util');

const first = (value) => util.inspect(value).split('\n')[0];
const formatted = (value) => util.format('%s', value).split('\n')[0];
const cases = [
  ['a name set after construction', () => { const e = new Error('duplicate'); e.name = 'SequelizeUniqueConstraintError'; return e; }, 'Error [SequelizeUniqueConstraintError]: duplicate'],
  ['a subclass renaming itself in its constructor', () => { class X extends Error { constructor(m) { super(m); this.name = 'XError'; } } return new X('m'); }, 'X [XError]: m'],
  ['a subclass keeping the name Error', () => { class Y extends Error {} return new Y('m'); }, 'Y [Error]: m'],
  ['a renamed TypeError whose name does not end in Error', () => { const e = new TypeError('t'); e.name = 'Custom'; return e; }, 'Custom: t'],
  ['an Error keeping its name', () => { const e = new Error('dup'); e.name = 'Error'; return e; }, 'Error: dup'],
  ['an error without a stack', () => { const e = new Error('ns'); e.name = 'Z'; e.stack = undefined; return e; }, '[Z: ns]'],
  ['a replaced stack string', () => { const e = new Error('sr'); e.name = 'W'; e.stack = 'W: sr\n    at x'; return e; }, 'W: sr'],
  ['an empty message', () => { const e = new Error(); e.name = 'Q'; return e; }, 'Q'],
  ['a name on the subclass prototype', () => { class P extends Error {} P.prototype.name = 'PName'; return new P('m'); }, 'PName: m'],
];
for (const [label, make, expected] of cases) {
  test(`inspect and %s of ${label} match Node`, () => {
    assert.equal(first(make()), expected);
    assert.equal(formatted(make()), expected);
  });
}
