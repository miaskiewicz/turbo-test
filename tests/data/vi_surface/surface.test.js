import { test, vi, afterAll } from 'vitest'
const realSetTimeout = globalThis.setTimeout
import * as fs from 'node:fs'
const rows = []
const show = (v) => {
  if (v === vi) return 'vi'
  if (v === undefined) return 'undefined'
  if (typeof v === 'function') return `fn:${v.name}:${vi.isMockFunction(v) ? 'mock' : 'plain'}`
  if (v instanceof Date) return `Date:${v.getTime()}`
  if (v instanceof Promise) return 'Promise'
  if (v instanceof Error) return `${v.name}: ${v.message}`
  if (v && typeof v === 'object') return JSON.stringify(v, (k, x) => (typeof x === 'function' ? show(x) : x === undefined ? 'undefined' : x))
  return JSON.stringify(v)
}
const row = async (name, f) => {
  let r
  try { r = ['value', show(await f())] } catch (e) { r = ['throws', show(e)] }
  rows.push([name, ...r])
}

test('vi surface', async () => {
  // #179: getRealSystemTime is the real clock, also under fake timers / a mocked time
  await row('getRealSystemTime real', () => typeof vi.getRealSystemTime() === 'number' && vi.getRealSystemTime() > 1.6e12)
  await row('getRealSystemTime under fake', () => { vi.useFakeTimers({ now: 5 }); vi.setSystemTime(0); const t = vi.getRealSystemTime(); vi.useRealTimers(); return t > 1.6e12 })
  // setTimerTickMode
  await row('setTimerTickMode without fake', () => vi.setTimerTickMode('manual'))
  await row('setTimerTickMode invalid', () => { vi.useFakeTimers(); try { return vi.setTimerTickMode('x') } finally { vi.useRealTimers() } })
  await row('setTimerTickMode manual returns', () => { vi.useFakeTimers(); try { return vi.setTimerTickMode('manual') } finally { vi.useRealTimers() } })
  await row('setTimerTickMode nextTimerAsync', async () => {
    vi.useFakeTimers({ now: 0 }); vi.setTimerTickMode('nextTimerAsync')
    const order = []
    setTimeout(() => order.push('a'), 1000)
    await new Promise((r) => setTimeout(r, 60_000))
    order.push(Date.now())
    vi.useRealTimers()
    return order
  })
  await row('setTimerTickMode nextTimerAsync then manual', async () => {
    vi.useFakeTimers({ now: 0 }); vi.setTimerTickMode('nextTimerAsync'); vi.setTimerTickMode('manual')
    let fired = false
    setTimeout(() => { fired = true }, 10)
    await new Promise((r) => globalThis.queueMicrotask(r))
    const c = vi.getTimerCount()
    vi.useRealTimers()
    return [fired, c]
  })
  await row('setTimerTickMode interval', async () => {
    vi.useFakeTimers({ now: 0 }); vi.setTimerTickMode('interval', 10)
    const t0 = Date.now()
    await new Promise((r) => realSetTimeout(r, 120))
    const moved = Date.now() - t0
    vi.useRealTimers()
    return moved >= 50 && moved % 10 === 0
  })
  // defineHelper
  const h = vi.defineHelper(function (a, b) { return [this && this.tag, a + b] })
  await row('defineHelper fn', () => h)
  await row('defineHelper sync', () => h.call({ tag: 't' }, 1, 2))
  const ap = Promise.resolve(7)
  const ha = vi.defineHelper(() => ap)
  await row('defineHelper async is new promise', () => { const p = ha(); return [p instanceof Promise, p === ap] })
  await row('defineHelper async value', () => ha())
  await row('defineHelper throws', () => vi.defineHelper(() => { throw new Error('boom') })())
  await row('defineHelper thenable', () => { const t = { then(r) { r(3) } }; return vi.defineHelper(() => t)() })
  // mockObject
  const src = { f: () => 1, n: 2, s: 'x', arr: [1, 2], o: { g() { return 3 } }, C: class { m() { return 4 } } }
  await row('mockObject automock', () => { const m = vi.mockObject(src); return [m === src, m.f, m.f(), m.n, m.s, m.arr, m.o.g, m.o.g(), new m.C().m(), vi.isMockFunction(m.C)] })
  await row('mockObject spy', () => { const m = vi.mockObject(src, { spy: true }); return [m.f, m.f(), m.o.g(), m.arr, new m.C().m(), m.f.mock.calls.length] })
  await row('mockObject spy is in place', () => { const o = { h: () => 5, arr: [1], in: { k: () => 6 } }; const m = vi.mockObject(o, { spy: true }); return [m === o, m.arr === o.arr, vi.isMockFunction(o.h), vi.isMockFunction(o.in.k), o.in.k()] })
  await row('mockObject automock is a copy', () => { const o = { h: () => 5, arr: [1], in: {} }; const m = vi.mockObject(o); return [m === o, m.in === o.in, vi.isMockFunction(o.h)] })
  // setConfig / resetConfig, observed through fakeTimers (#180)
  await row('setConfig returns', () => vi.setConfig({ fakeTimers: { now: 1000 } }))
  await row('setConfig fakeTimers now', () => { vi.useFakeTimers(); const t = Date.now(); vi.useRealTimers(); return t })
  await row('setConfig merged with arg', () => { vi.useFakeTimers({ loopLimit: 5 }); const t = Date.now(); vi.useRealTimers(); return t })
  await row('resetConfig returns', () => vi.resetConfig())
  await row('resetConfig restored', () => { vi.useFakeTimers(); const t = Date.now(); vi.useRealTimers(); return t > 1.6e12 })
  // chainable returns
  await row('stubGlobal returns', () => vi.stubGlobal('__k8', 1))
  await row('stubGlobal descriptor', () => Object.getOwnPropertyDescriptor(globalThis, '__k8'))
  await row('unstubAllGlobals returns', () => vi.unstubAllGlobals())
  await row('unstubAllGlobals removed', () => '__k8' in globalThis)
  await row('stubGlobal getter restored', () => { Object.defineProperty(globalThis, '__k8g', { get: () => 'got', configurable: true }); vi.stubGlobal('__k8g', 1); const a = globalThis.__k8g; vi.unstubAllGlobals(); const d = Object.getOwnPropertyDescriptor(globalThis, '__k8g'); delete globalThis.__k8g; return [a, typeof d.get, d.value] })
  await row('stubEnv returns', () => vi.stubEnv('K8_A', 'a'))
  await row('stubEnv number', () => { vi.stubEnv('K8_B', 1); return [process.env.K8_B, import.meta.env.K8_B] })
  await row('stubEnv undefined deletes', () => { process.env.K8_C = 'c'; vi.stubEnv('K8_C', undefined); return ['K8_C' in process.env] })
  await row('stubEnv PROD boolean', () => { vi.stubEnv('PROD', true); const a = import.meta.env.PROD; vi.stubEnv('PROD', false); return [a, import.meta.env.PROD] })
  await row('unstubAllEnvs returns', () => vi.unstubAllEnvs())
  await row('unstubAllEnvs restored', () => [process.env.K8_A, process.env.K8_B, process.env.K8_C])
  await row('clearAllMocks returns', () => vi.clearAllMocks())
  await row('resetAllMocks returns', () => vi.resetAllMocks())
  await row('restoreAllMocks returns', () => vi.restoreAllMocks())
  await row('resetModules returns', () => vi.resetModules())
  await row('doUnmock non-string', () => vi.doUnmock(1))
  await row('doMock non-string', () => vi.doMock(1))
  await row('doMock returns', () => { const r = vi.doMock('./nope-k8.js', () => ({})); vi.doUnmock('./nope-k8.js'); return [typeof r, typeof r[Symbol.dispose]] })
  await row('doUnmock returns', () => vi.doUnmock('./nope-k8.js'))
  await row('hoisted non-function', () => vi['hoisted'](1))
  await row('hoisted value', () => vi['hoisted'](() => 5))
  await row('getMockedSystemTime real', () => vi.getMockedSystemTime())
  await row('getMockedSystemTime date-only', () => { vi.setSystemTime(1234); const d = vi.getMockedSystemTime(); vi.useRealTimers(); return [d, vi.getMockedSystemTime()] })
  await row('getTimerCount without fake', () => vi.getTimerCount())
  await row('clearAllTimers without fake', () => vi.clearAllTimers())
  await row('isFakeTimers', () => vi.isFakeTimers())
  await row('dynamicImportSettled', () => { const p = vi.dynamicImportSettled(); return [p instanceof Promise] })
  // vi.mocked returns its argument untouched
  await row('mocked plain fn', () => { const f = () => 1; const m = vi.mocked(f, { partial: true }); return [m === f, 'mock' in f, Object.getOwnPropertyNames(f).sort()] })
  await row('mocked object', () => { const o = { a: 1 }; return vi.mocked(o) === o })
})

test('mock function surface', async () => {
  const impl = function (a) { return a * 2 }
  await row('fn name/length', () => { const f = vi.fn(); const g = vi.fn(impl); return [f.name, f.length, g.name, g.length] })
  await row('spy name', () => { const o = { meth(a, b) {} }; const s = vi.spyOn(o, 'meth'); return [s.name, s.length, s.getMockName()] })
  await row('getMockName', () => [vi.fn().getMockName(), vi.fn(impl).getMockName(), vi.fn().mockName('n').getMockName()])
  await row('getMockImplementation', () => { const f = vi.fn(); const g = vi.fn(impl); return [f.getMockImplementation(), g.getMockImplementation() === impl] })
  await row('getMockImplementation after mockImplementation', () => { const k = () => 9; const f = vi.fn(impl).mockImplementation(k); return f.getMockImplementation() === k })
  await row('getMockImplementation after mockReturnValue', () => { const f = vi.fn(impl).mockReturnValue(3); const i = f.getMockImplementation(); return [typeof i, i === impl, i && i()] })
  await row('getMockImplementation with once', () => { const k = () => 9; const f = vi.fn(impl).mockImplementationOnce(k); return f.getMockImplementation() === k })
  await row('mockThrow', () => { const f = vi.fn(impl).mockThrow(new Error('t')); let a, b; try { f(1) } catch (e) { a = e.message } try { f(1) } catch (e) { b = e.message } return [a, b, f.mock.results] })
  await row('mockThrow returns fn', () => { const f = vi.fn(); return f.mockThrow(1) === f && f.mockThrowOnce(2) === f })
  await row('mockThrowOnce', () => { const f = vi.fn(impl).mockThrowOnce('once'); let a; try { f(1) } catch (e) { a = e } return [a, f(2), f.mock.results] })
  await row('mockThrow non-error', () => { const f = vi.fn().mockThrow({ x: 1 }); try { f() } catch (e) { return e } })
  await row('withImplementation sync', () => { const f = vi.fn(impl); let inside; const r = f.withImplementation(() => 'w', () => { inside = f(1) }); return [r === f, r, inside, f(1)] })
  await row('withImplementation async', async () => { const f = vi.fn(impl); let inside; const r = f.withImplementation(() => 'w', async () => { await null; inside = f(1) }); const p = r instanceof Promise; const v = await r; return [p, v === f, inside, f(1)] })
  await row('withImplementation once queue', () => { const f = vi.fn(impl).mockReturnValueOnce('o'); let inside; f.withImplementation(() => 'w', () => { inside = f(1) }); return [inside, f(1), f(1)] })
  await row('mock.contexts', () => { const f = vi.fn(); const o = { f }; o.f(); f.call(5); new f(); return [f.mock.contexts.length, f.mock.contexts[0] === o, f.mock.contexts[1], typeof f.mock.contexts[2]] })
  await row('mock.lastCall', () => { const f = vi.fn(); const a = f.mock.lastCall; f(1); f(2, 3); const d = Object.getOwnPropertyDescriptor(f.mock, 'lastCall'); return [a, f.mock.lastCall, typeof d.get, Object.keys(f.mock)] })
  await row('mockReset to original', () => { const f = vi.fn(impl).mockReturnValue(5); f.mockReset(); return [f(2), f.getMockImplementation() === impl] })
  await row('mockReset no impl', () => { const f = vi.fn().mockReturnValue(5); f.mockReset(); return f(2) })
  await row('mockRestore fn', () => { const f = vi.fn(impl).mockImplementation(() => 0); f(1); f.mockRestore(); return [f(2), f.mock.calls.length] })
  await row('mockReturnValue undefined', () => vi.fn(impl).mockReturnValue(undefined)(4))
  await row('mockReturnValue then impl', () => vi.fn().mockReturnValue(1).mockImplementation(() => 2)())
  await row('spyOn getter', () => {
    const o = { _v: 1, get g() { return this._v } }
    const s = vi.spyOn(o, 'g', 'get')
    const a = o.g
    s.mockReturnValue(5)
    const b = o.g
    const calls = s.mock.calls.length
    s.mockRestore()
    return [a, b, calls, o.g, typeof Object.getOwnPropertyDescriptor(o, 'g').get]
  })
  await row('spyOn setter', () => {
    let seen
    const o = { set v(x) { seen = x } }
    const s = vi.spyOn(o, 'v', 'set')
    o.v = 3
    s.mockImplementation((x) => { seen = 'spy' + x })
    o.v = 4
    const calls = s.mock.calls
    s.mockRestore()
    o.v = 5
    return [seen, calls]
  })
  await row('spyOn missing getter', () => vi.spyOn({ a: 1 }, 'a', 'get'))
  await row('spyOn missing prop', () => vi.spyOn({}, 'nope'))
  await row('spyOn non-function', () => vi.spyOn({ a: 1 }, 'a'))
  await row('spyOn primitive', () => vi.spyOn(1, 'a'))
})

afterAll(() => fs.writeFileSync(process.env.OUT || 'surface.vitest.json', JSON.stringify(rows, null, 1)))
