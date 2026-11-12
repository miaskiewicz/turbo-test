import { test, expect } from 'vitest'
test('config serializers', () => {
  expect({ n: 1, cur: 'EUR' }).toMatchInlineSnapshot()
  expect([{ n: 2, cur: 'USD', note: { a: 1 } }, { tag: 'T', body: { n: 3, cur: 'PLN' } }]).toMatchInlineSnapshot()
  expect.addSnapshotSerializer({ test: (v) => v && v.cur === 'GBP', serialize: () => 'pounds' })
  expect({ n: 4, cur: 'GBP' }).toMatchSnapshot()
})
