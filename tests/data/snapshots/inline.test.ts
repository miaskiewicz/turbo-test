import { describe, test, expect } from 'vitest'
describe('inline', () => {
  test('new ones', () => {
    expect({ a: 1, b: [1, 2] }).toMatchInlineSnapshot()
    expect('one').toMatchInlineSnapshot()
    expect(() => { throw new Error('boom') }).toThrowErrorMatchingInlineSnapshot()
    expect({ id: 3, n: 'x' }).toMatchInlineSnapshot({ id: expect.any(Number) })
		expect('tab `x` ${y} \\ z').toMatchInlineSnapshot()
  })
  test('changed', () => {
    expect(2).toMatchInlineSnapshot(`1`)
    expect({ c: 3 }).toMatchInlineSnapshot(`
      {
        "c": 2,
      }
    `)
  })
})
