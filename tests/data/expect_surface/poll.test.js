import { test, expect, vi } from 'vitest'
import * as fs from 'node:fs'
const rows = []
const show = (v) => { try { return v === undefined ? 'undefined' : typeof v === 'function' ? 'fn' : typeof v === 'bigint' ? v + 'n' : JSON.stringify(v) } catch { return String(v) } }
const isAssertion = (v) => v != null && typeof v === 'object' && typeof v.toBe === 'function' && typeof v.toEqual === 'function'
const t = async (name, f) => { let r; try { const v = await f(); r = v === undefined ? 'passed' : isAssertion(v) ? 'assertion' : ['value', show(v)] } catch (e) { r = [e && e.name, e && e.message, e && e.cause && e.cause.message] } rows.push(JSON.stringify([name, r])) }
let outside
try { expect.poll(() => 1) } catch (e) { outside = [e.name, e.message] }
const counter = (pass) => { let n = 0; const fn = () => { n++; return pass(n) }; return [fn, () => n] }
const real = Date.now
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
test('poll', async () => {
  await t('outside a test', () => outside)
  await t('typeof', () => typeof expect.poll)
  await t('pass first', () => expect.poll(() => 1).toBe(1))
  await t('result', async () => { const p = expect.poll(() => 1).toBe(1); const r = [Object.prototype.toString.call(p), p instanceof Promise, typeof p.then, typeof p.catch, typeof p.finally]; await p; return r })
  await t('pass after 3', async () => { const [fn, n] = counter((i) => i); await expect.poll(fn, { interval: 10 }).toBe(3); return n() })
  await t('async fn', async () => { const [fn, n] = counter(async (i) => i >= 2 ? 'ok' : 'no'); await expect.poll(fn, { interval: 10 }).toBe('ok'); return n() })
  await t('not', async () => { const [fn, n] = counter((i) => i); await expect.poll(fn, { interval: 10 }).not.toBeLessThan(4); return n() })
  await t('toEqual', () => expect.poll(() => ({ a: [1] })).toEqual({ a: [1] }))
  await t('fail', () => expect.poll(() => 1, { timeout: 100, interval: 20 }).toBe(2))
  await t('fail not', () => expect.poll(() => 1, { timeout: 100, interval: 20 }).not.toBe(1))
  await t('fail message', () => expect.poll(() => 1, { timeout: 100, message: 'the msg' }).toBe(2))
  await t('fn throws', () => expect.poll(() => { throw new Error('boom') }, { timeout: 100 }).toBe(1))
  await t('fn throws then passes', async () => { const [fn, n] = counter((i) => { if (i < 3) throw new Error('not yet'); return i }); await expect.poll(fn, { interval: 10 }).toBe(3); return n() })
  await t('fn rejects', () => expect.poll(async () => { throw new TypeError('rej') }, { timeout: 60 }).toBe(1))
  await t('toThrow', () => expect.poll(() => 1).toThrow)
  await t('toMatchSnapshot', () => expect.poll(() => 1).toMatchSnapshot)
  await t('toMatchInlineSnapshot', () => expect.poll(() => 1).toMatchInlineSnapshot)
  await t('toThrowError', () => expect.poll(() => 1).not.toThrowError)
  await t('not property', async () => { const p = expect.poll(() => 1).not.toBe(2); const r = [typeof expect.poll(() => 1).not, typeof expect.poll(() => 1).not.toBe, Object.prototype.toString.call(p)]; await p; return r })
  await t('resolves', () => expect.poll(() => 1).resolves)
  await t('rejects', () => expect.poll(() => 1).rejects)
  await t('assertion count', async () => { const a = expect.getState().assertionCalls; const p = expect.poll(() => 1); const b = expect.getState().assertionCalls; await p.toBe(1); return [b - a, expect.getState().assertionCalls - a] })
  await t('real timer', async () => { let done = false; setTimeout(() => { done = true }, 125); const [fn, n] = counter(() => done); await expect.poll(fn, { interval: 100 }).toBe(true); return n() })
  await t('timeout elapsed', async () => { const t0 = Date.now(); try { await expect.poll(() => 1, { timeout: 300, interval: 100 }).toBe(2) } catch {} const d = Date.now() - t0; return [d >= 300, d < 600] })
  await t('default timeout', async () => { const t0 = Date.now(); try { await expect.poll(() => 1).toBe(2) } catch {} const d = Date.now() - t0; return [d >= 1000, d < 1400] })
  await t('default interval', async () => { const t0 = Date.now(); const [fn, n] = counter((i) => i); await expect.poll(fn).toBe(3); const d = Date.now() - t0; return [n(), d >= 100, d < 300] })
  await t('slow fn', async () => { const [fn, n] = counter(async (i) => { await sleep(200); return i }); try { await expect.poll(fn, { timeout: 300, interval: 10 }).toBe(99) } catch (e) { return [n(), e.message] } })
  vi.useFakeTimers()
  await t('fake timers', async () => { let done = false; setTimeout(() => { done = true }, 120); const t0 = Date.now(); const [fn, n] = counter(() => done); await expect.poll(fn, { interval: 50 }).toBe(true); return [n(), Date.now() - t0] })
  await t('fake timers fail', async () => { const t0 = Date.now(); const [fn, n] = counter(() => 1); try { await expect.poll(fn, { interval: 50, timeout: 300 }).toBe(2) } catch (e) { return [(Date.now() - t0) === (n() - 1) * 50, n() >= 4, n() <= 9, e.message] } })
  vi.useRealTimers()
  await t('extend', async () => { expect.extend({ toBeK5Poll: (r, n) => ({ pass: r === n, message: () => `k5 ${r} ${n}` }) }); const [fn, n] = counter((i) => i); await expect.poll(fn, { interval: 10 }).toBeK5Poll(2); return n() })
  await t('extend fail', () => expect.poll(() => 1, { timeout: 50 }).toBeK5Poll(2))
  await t('this', async () => { let self; await expect.poll(function () { self = this; return 1 }).toBe(1); return [isAssertion(self), typeof self] })
  fs.writeFileSync('out.txt', rows.join('\n') + '\n')
})
