import * as fs from 'node:fs'
const rows = []
const show = (v) => {
  if (v === jest) return 'jest'
  if (v === undefined) return 'undefined'
  if (v && typeof v === 'object' && typeof v.message === 'string') return `${v.name}: ${v.message}`
  return JSON.stringify(v)
}
const row = async (name, f) => {
  const warns = []
  const orig = console.warn
  console.warn = (...a) => warns.push(a.map(String).join(' ').split('\nStack Trace:')[0])
  let r
  try { r = ['value', show(await f())] } catch (e) { r = ['throws', show(e)] } finally { console.warn = orig; jest.useRealTimers() }
  rows.push([name, ...r, warns])
}
const realSetTimeout = setTimeout
test('jest timers', async () => {
  await row('advance without fake', () => jest.advanceTimersByTime(10))
  await row('runAllTimers without fake', () => jest.runAllTimers())
  await row('runOnlyPendingTimers without fake', () => jest.runOnlyPendingTimers())
  await row('advanceTimersToNextTimer without fake', () => jest.advanceTimersToNextTimer())
  await row('advanceTimersByTimeAsync without fake', () => jest.advanceTimersByTimeAsync(10))
  await row('runAllTimersAsync without fake', () => jest.runAllTimersAsync())
  await row('runOnlyPendingTimersAsync without fake', () => jest.runOnlyPendingTimersAsync())
  await row('advanceTimersToNextTimerAsync without fake', () => jest.advanceTimersToNextTimerAsync())
  await row('advanceTimersToNextFrame without fake', () => jest.advanceTimersToNextFrame())
  await row('runAllTicks without fake', () => jest.runAllTicks())
  await row('getTimerCount without fake', () => jest.getTimerCount())
  await row('clearAllTimers without fake', () => jest.clearAllTimers())
  await row('setSystemTime without fake', () => { jest.setSystemTime(5); return Date.now() > 1.6e12 })
  await row('now without fake', () => jest.now() > 1.6e12)
  await row('getRealSystemTime', () => { jest.useFakeTimers({ now: 0 }); return jest.getRealSystemTime() > 1.6e12 })
  await row('setTimerTickMode without fake', () => jest.setTimerTickMode({ mode: 'manual' }))
  await row('useFakeTimers returns', () => jest.useFakeTimers())
  await row('useRealTimers returns', () => jest.useRealTimers())
  await row('now option', () => { jest.useFakeTimers({ now: 1000 }); return [Date.now(), jest.now()] })
  await row('now Date option', () => { jest.useFakeTimers({ now: new Date(2000) }); return Date.now() })
  await row('doNotFake Date', () => { jest.useFakeTimers({ doNotFake: ['Date'], now: 1000 }); let f = false; setTimeout(() => { f = true }, 10); jest.advanceTimersByTime(10); return [Date.now() > 1.6e12, f] })
  await row('doNotFake setTimeout', () => { jest.useFakeTimers({ doNotFake: ['setTimeout'], now: 1000 }); return [Date.now(), jest.getTimerCount(), setTimeout === realSetTimeout] })
  await row('doNotFake performance', () => { jest.useFakeTimers({ doNotFake: ['performance'] }); const a = performance.now(); jest.advanceTimersByTime(1000); return performance.now() - a < 500 })
  await row('timerLimit', () => { jest.useFakeTimers({ timerLimit: 5 }); setInterval(() => {}, 1); jest.runAllTimers() })
  await row('default limit', () => { jest.useFakeTimers(); setInterval(() => {}, 1); jest.runAllTimers() })
  await row('advanceTimers true', async () => { jest.useFakeTimers({ now: 0, advanceTimers: true }); await new Promise((r) => realSetTimeout(r, 200)); const t = Date.now(); return t >= 100 })
  await row('advanceTimers number', async () => { jest.useFakeTimers({ now: 0, advanceTimers: 10 }); await new Promise((r) => realSetTimeout(r, 200)); const t = Date.now(); return [t >= 100, t % 10] })
  await row('advanceTimers false', async () => { jest.useFakeTimers({ now: 0, advanceTimers: false }); await new Promise((r) => realSetTimeout(r, 50)); return Date.now() })
  await row('toFake ignored', () => { jest.useFakeTimers({ toFake: ['setTimeout'], now: 1000 }); return Date.now() })
  await row('loopLimit ignored', () => { jest.useFakeTimers({ loopLimit: 3 }); setInterval(() => {}, 1); jest.runAllTimers() })
  await row('legacy', () => { jest.useFakeTimers({ legacyFakeTimers: true }); let f = 0; setTimeout(() => { f++ }, 10); jest.runAllTimers(); return [f, Date.now() > 1.6e12, typeof jest.runAllImmediates] })
  await row('runAllImmediates modern', () => { jest.useFakeTimers(); jest.runAllImmediates() })
  await row('getTimerCount', () => { jest.useFakeTimers(); setTimeout(() => {}, 1); setInterval(() => {}, 1); return jest.getTimerCount() })
  await row('advanceTimersToNextFrame', () => { jest.useFakeTimers({ now: 0 }); jest.advanceTimersToNextFrame(); return Date.now() })
  await row('setTimerTickMode', async () => { jest.useFakeTimers({ now: 0 }); const r = jest.setTimerTickMode({ mode: 'nextAsync' }); await new Promise((r) => setTimeout(r, 5000)); return [r === jest, Date.now()] })
  await row('setTimerTickMode interval', async () => { jest.useFakeTimers({ now: 0 }); jest.setTimerTickMode({ mode: 'interval', delta: 10 }); await new Promise((r) => realSetTimeout(r, 200)); return Date.now() >= 100 })
  await row('chain returns', () => { jest.useFakeTimers(); return [jest.advanceTimersByTime(1), jest.runAllTimers(), jest.runOnlyPendingTimers(), jest.advanceTimersToNextTimer(), jest.clearAllTimers(), jest.runAllTicks(), jest.advanceTimersToNextFrame(), jest.setSystemTime(1)].map(show) })
  await row('chain returns async', async () => { jest.useFakeTimers(); return [await jest.advanceTimersByTimeAsync(1), await jest.runAllTimersAsync(), await jest.runOnlyPendingTimersAsync(), await jest.advanceTimersToNextTimerAsync()].map(show) })
})
afterAll(() => fs.writeFileSync(process.env.OUT || 'timers.jest.json', JSON.stringify(rows, null, 1)))
