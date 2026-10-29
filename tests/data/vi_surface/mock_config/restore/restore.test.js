import { test, vi, expect, beforeEach } from 'vitest'
// test.clearMocks / restoreMocks / unstubEnvs / unstubGlobals: before each test (and before its
// beforeEach hooks) vitest clears the mocks, restores the spies and undoes the stubs
const o = { m: () => 1 }
const o2 = { m: () => 1 }
const f = vi.fn(() => 'impl')
beforeEach(() => { vi.spyOn(o2, 'm').mockReturnValue(9) })
test('stubs and spies', () => {
  vi.spyOn(o, 'm').mockReturnValue(2)
  f()
  f.mockReturnValue('set')
  vi.stubGlobal('__k8g', 1)
  vi.stubEnv('K8_ENV', 'x')
  expect([o.m(), f.mock.calls.length, globalThis.__k8g, process.env.K8_ENV, o2.m()]).toEqual([2, 1, 1, 'x', 9])
})
test('are undone before the next test, before its beforeEach', () => {
  // clearMocks keeps f's implementation (mockReset would drop it)
  expect([o.m(), f.mock.calls.length, f(), '__k8g' in globalThis, 'K8_ENV' in process.env, o2.m()]).toEqual([1, 0, 'set', false, false, 9])
})
