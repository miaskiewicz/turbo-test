import { test, expect, vi } from 'vitest'
import * as fs from 'node:fs'
import { cases } from './cases.js'
test('matchers', () => {
  const out = []
  for (const [label, fn] of cases(expect, vi)) {
    try { fn(); out.push({ label, passed: true }) } catch (e) { out.push({ label, name: e && e.name, message: e && e.message, actual: e && e.actual, expected: e && e.expected }) }
  }
  fs.writeFileSync('out.json', JSON.stringify(out, null, 1))
})
