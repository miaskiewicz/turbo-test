import { test, expect } from 'vitest'
import * as fs from 'node:fs'
const out = []
const rec = (label, fn) => { try { fn(); out.push({ label, passed: true }) } catch (e) { out.push({ label, name: e && e.name, message: e && e.message }) } }
expect.extend({
  toBeFoo(received) { return { pass: received === 'foo', message: () => `expected ${received} to be foo` } },
  toBeBar(received) { return { pass: received === 'bar', message: 'plain bar message' } },
})
test('snap', () => {
  rec('snapshot', () => expect({ a: 2 }).toMatchSnapshot())
  rec('inline', () => expect({ a: 2 }).toMatchInlineSnapshot(`
    {
      "a": 1,
    }
  `))
  rec('throw inline', () => expect(() => { throw new Error('boom') }).toThrowErrorMatchingInlineSnapshot(`[Error: other]`))
  rec('extend fn', () => expect('x').toBeFoo())
  rec('extend not', () => expect('foo').not.toBeFoo())
  rec('extend str', () => expect('x').toBeBar())
  fs.writeFileSync('out3.json', JSON.stringify(out, null, 1))
})
