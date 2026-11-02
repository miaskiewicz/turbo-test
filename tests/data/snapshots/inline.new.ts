import { describe, test, expect } from 'vitest'
describe('inline', () => {
  test('new ones', () => {
    expect({ a: 1, b: [1, 2] }).toMatchInlineSnapshot(`
      {
        "a": 1,
        "b": [
          1,
          2,
        ],
      }
    `)
    expect('one').toMatchInlineSnapshot(`"one"`)
    expect(() => { throw new Error('boom') }).toThrowErrorMatchingInlineSnapshot(`[Error: boom]`)
    expect({ id: 3, n: 'x' }).toMatchInlineSnapshot({ id: expect.any(Number) }, `
      {
        "id": Any<Number>,
        "n": "x",
      }
    `)
		expect('tab `x` ${y} \\ z').toMatchInlineSnapshot(`"tab \`x\` \${y} \\ z"`)
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
