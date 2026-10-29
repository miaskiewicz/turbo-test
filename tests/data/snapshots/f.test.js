import { describe, test, expect, vi } from 'vitest'
describe('group', () => {
  test('values', () => {
    expect({ b: 1, a: [1, 'x', null, undefined], m: new Map([['k', { z: 1 }]]), s: new Set([2]), d: new Date(0), r: /x.y+/g, f() {}, n: -0, big: 10n, nested: { deep: { deeper: [true] } } }).toMatchSnapshot()
    expect('multi\nline "q" `t` ${x} \\ end').toMatchSnapshot()
    expect(42).toMatchSnapshot()
    expect('one line').toMatchSnapshot()
    expect([]).toMatchSnapshot()
    expect({}).toMatchSnapshot('hinted')
    expect({ id: 7, name: 'n' }).toMatchSnapshot({ id: expect.any(Number) })
  })
  test('errors and mocks', () => {
    const fn = vi.fn().mockName('named')
    fn(1, { a: 2 })
    expect(fn).toMatchSnapshot()
    expect(vi.fn()).toMatchSnapshot()
    expect(new Error('e')).toMatchSnapshot()
    expect(() => { throw new TypeError('boom') }).toThrowErrorMatchingSnapshot()
    expect(() => { throw new Error('inline') }).toThrowErrorMatchingInlineSnapshot(`[Error: inline]`)
    expect({ a: 1 }).toMatchInlineSnapshot(`
      {
        "a": 1,
      }
    `)
  })
})
test('key with `backtick` and ${dollar}', () => { expect(1).toMatchSnapshot() })
test('t 10', () => { expect(10).toMatchSnapshot() })
test('t 9', () => { expect(9).toMatchSnapshot() })
