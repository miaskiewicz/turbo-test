import * as fs from 'node:fs'
const rows = []
const realNow = () => Date.now() > 1.6e12
const row = (name, f) => { let r; try { r = ['value', JSON.stringify(f())] } catch (e) { r = ['throws', `${e.name}: ${e.message}`] } rows.push([name, ...r]) }
const atLoad = [typeof setTimeout.clock, Date.now() === 1000]
test('first test', () => {
  row('faked at load', () => atLoad)
  row('faked in test', () => [typeof setTimeout.clock, Date.now() === 1000, jest.now() === 1000])
  row('timers run on advance', () => { let n = 0; setTimeout(() => { n++ }, 50); jest.advanceTimersByTime(50); return [n, jest.getTimerCount()] })
  row('doNotFake', () => [typeof queueMicrotask.clock, typeof setInterval.clock])
  row('timerLimit', () => { if (typeof setTimeout.clock !== 'object') return 'real'; const tick = () => setTimeout(tick, 1); tick(); jest.runAllTimers(); return 'no throw' })
  row('useRealTimers', () => { jest.useRealTimers(); return [typeof setTimeout.clock, realNow()] })
  row('useFakeTimers() takes the config', () => { jest.useFakeTimers(); return [typeof setTimeout.clock, Date.now(), typeof queueMicrotask.clock] })
  row('useFakeTimers(cfg) over the config', () => { jest.useFakeTimers({ now: 5 }); return [Date.now(), typeof queueMicrotask.clock] })
  row('advanceTimers', () => { jest.useFakeTimers({ advanceTimers: 20 }); return typeof setTimeout.clock })
})
test('second test keeps the last state', () => { row('second test', () => typeof setTimeout.clock) })
afterAll(() => fs.writeFileSync(process.env.OUT, JSON.stringify(rows, null, 1)))
