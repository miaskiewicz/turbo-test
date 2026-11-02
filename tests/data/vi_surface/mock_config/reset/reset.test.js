import { test, vi, expect } from 'vitest'
// test.mockReset: each mock goes back to its vi.fn(impl) implementation before each test
const f = vi.fn(() => 'impl')
const g = vi.fn()
test('set', () => { f.mockReturnValue('set'); g.mockReturnValue(1); f(); expect(f()).toBe('set') })
test('reset', () => { expect([f(), g(), f.mock.calls.length]).toEqual(['impl', undefined, 1]) })
