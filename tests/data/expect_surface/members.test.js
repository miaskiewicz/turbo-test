import { test, expect } from 'vitest'
import * as fs from 'node:fs'
const names = (o) => { const s = new Set(); let p = o; while (p && p !== Object.prototype && p !== Function.prototype) { for (const k of Object.getOwnPropertyNames(p)) s.add(k); p = Object.getPrototypeOf(p) } return [...s].sort() }
const own = (o) => Object.getOwnPropertyNames(o).sort()
test('surface', () => {
  const asym = {
    any: expect.any(Number), anything: expect.anything(), objectContaining: expect.objectContaining({}), arrayContaining: expect.arrayContaining([]),
    stringContaining: expect.stringContaining('a'), stringMatching: expect.stringMatching('a'), closeTo: expect.closeTo?.(1), toBeOneOf: expect.toBeOneOf([1]),
    notObjectContaining: expect.not.objectContaining({}), notStringMatching: expect.not.stringMatching('a'),
  }
  const out = {
    expect: own(expect).map((k) => k + ':' + typeof expect[k]),
    expectNot: own(expect.not).map((k) => k + ':' + typeof expect.not[k]),
    asym: Object.fromEntries(Object.entries(asym).map(([k, v]) => [k, v ? names(v).map((n) => { let t; try { t = typeof v[n] } catch { t = 'throws' } return n + ':' + t }) : null])),
    asymStr: Object.fromEntries(Object.entries(asym).map(([k, v]) => [k, v ? [String(v), v.toString?.(), v.getExpectedType?.(), v.toAsymmetricMatcher?.(), String(v.$$typeof), v.inverse, JSON.stringify(v.sample)] : null])),
    chain: {},
  }
  const desc = (o, k) => { let p = o; while (p) { const d = Object.getOwnPropertyDescriptor(p, k); if (d) return d; p = Object.getPrototypeOf(p) } }
  const fns = (o) => { const s = new Set(); for (const k of names(o)) s.add(k); return [...s].map((k) => { const d = desc(o, k); return k + ':' + (d.get ? 'get' : typeof d.value) }).sort() }
  out.chain.expect = fns(expect(1))
  out.chain.not = fns(expect(1).not)
  out.chain.resolves = fns(expect(Promise.resolve(1)).resolves)
  out.chain.rejects = fns(expect(Promise.resolve(1)).rejects)
  out.chain.soft = fns(expect.soft(1))
  fs.writeFileSync(process.env.OUT || 'surf.json', JSON.stringify(out, null, 1))
})
