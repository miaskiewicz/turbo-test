import { test, vi, expect } from 'vitest'
// without the options nothing is undone; vi.setConfig({ clearMocks }) turns it on for later tests
const f = vi.fn()
test('a', () => { f(); vi.stubGlobal('__k8n', 1) })
test('b', () => { expect([f.mock.calls.length, globalThis.__k8n]).toEqual([1, 1]); vi.setConfig({ clearMocks: true }) })
test('c', () => { expect(f.mock.calls.length).toBe(0) })
