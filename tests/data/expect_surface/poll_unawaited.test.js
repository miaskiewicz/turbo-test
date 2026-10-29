import { test, expect } from 'vitest'
test('unawaited', () => {
  let n = 0
  expect.poll(() => { n++; return 1 }).toBe(1)
  expect.poll(() => 1).not.toBe(2)
  globalThis.__n = n
})
test('ran', () => { expect(globalThis.__n).toBe(0) })
