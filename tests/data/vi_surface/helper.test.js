import { test, vi, expect } from 'vitest'
const check = vi.defineHelper((v) => {
  expect(v).toBe(2)
})
const checkAsync = vi.defineHelper(async (v) => {
  await null
  expect(v).toBe(3)
})
test('helper', () => {
  check(1)
})
test('async helper', async () => {
  await checkAsync(1)
})
