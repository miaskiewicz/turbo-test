import { test, vi, afterAll } from 'vitest'
import * as fs from 'node:fs'
const rows = []
const realSetImmediate = globalThis.setImmediate
const realPerformance = globalThis.performance
const row = (name, f) => {
  let r
  try { r = ['value', JSON.stringify(f())] } catch (e) { r = ['throws', `${e.name}: ${e.message}`] } finally { vi.useRealTimers() }
  rows.push([name, ...r])
}
test('test.fakeTimers is the default of vi.useFakeTimers()', () => {
  row('now', () => { vi.useFakeTimers(); return Date.now() })
  row('toFake', () => { vi.useFakeTimers(); return [setImmediate === realSetImmediate, performance === realPerformance] })
  row('loopLimit', () => { vi.useFakeTimers(); setInterval(() => {}, 1); vi.runAllTimers() })
  row('argument merged over it', () => { vi.useFakeTimers({ now: 5 }); setInterval(() => {}, 1); try { vi.runAllTimers() } catch (e) { return [Date.now(), e.message] } })
  row('argument toFake replaces it', () => { vi.useFakeTimers({ toFake: ['setImmediate'] }); return [Date.now() > 1.6e12, setImmediate === realSetImmediate] })
  row('setConfig over it', () => { vi.setConfig({ fakeTimers: { now: 77 } }); vi.useFakeTimers(); setInterval(() => {}, 1); try { vi.runAllTimers() } catch (e) { return [Date.now(), e.message] } })
  row('resetConfig back to it', () => { vi.resetConfig(); vi.useFakeTimers(); return Date.now() })
})
afterAll(() => fs.writeFileSync(process.env.OUT || 'config.vitest.json', JSON.stringify(rows, null, 1)))
