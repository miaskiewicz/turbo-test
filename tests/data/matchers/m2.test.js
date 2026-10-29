import { test, expect, vi } from 'vitest'
import * as fs from 'node:fs'
import { cases2 } from './cases2.js'
test('matchers2', async () => {
  const out = []
  for (const [label, fn] of cases2(expect, vi)) {
    try { fn(); out.push({ label, passed: true }) } catch (e) { out.push({ label, name: e && e.name, message: e && e.message }) }
  }
  for (const [label, fn] of [
    ['resolves rejected', () => expect(Promise.reject(new Error('r'))).resolves.toBe(1)],
    ['rejects resolved', () => expect(Promise.resolve({ a: 1 })).rejects.toBe(1)],
    ['resolves fail', () => expect(Promise.resolve(2)).resolves.toBe(1)],
    ['rejects toThrow', () => expect(Promise.reject(new Error('r'))).rejects.toThrow('x')],
  ]) {
    try { await fn(); out.push({ label, passed: true }) } catch (e) { out.push({ label, name: e && e.name, message: e && e.message }) }
  }
  fs.writeFileSync('out2.json', JSON.stringify(out, null, 1))
})
