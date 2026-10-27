import { describe, expect, it, test } from 'vitest';
test('passes', () => { expect(1).toBe(1); });
test('numbers', () => { expect(1).toBe(2); });
test('objects', () => { expect({ a: 1, b: [1, 'x'] }).toEqual({ a: 2, b: [1, 'y'] }); });
test('strings "q"', () => { expect('line1\nline2').toBe('line1\nLINE2'); });
test('thrown', () => { throw new TypeError('bad "thing"\nsecond'); });
test.skip('skipped', () => {});
test.todo('later');
test('name # with \\ backslash', () => {});
test('annotated', async ({ annotate }) => { await annotate('a note'); await annotate('warn me', 'warning'); });
describe('outer', () => {
  it('inner pass', () => {});
  describe('deeper', () => {
    it('deep fail', () => { expect([1, 2]).toStrictEqual([1, 3]); });
  });
  describe.skip('skipped suite', () => {
    it('never', () => {});
  });
});
describe('all pass', () => {
  it('one', () => {});
});
test('truthy', () => { expect(0).toBeTruthy(); });
test('non-error', () => { throw 'plain string'; });
