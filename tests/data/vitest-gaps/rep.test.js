import { describe, test, it, beforeEach, afterEach, afterAll, beforeAll } from 'vitest'
const log = []
afterAll(() => console.log('ORDER rep ' + JSON.stringify(log)))
const r = (c) => [c.task.result.repeatCount, c.task.result.retryCount]
describe('s', () => {
  beforeAll(() => log.push('ba'))
  beforeEach((c) => { log.push(['be', c.task.name, ...r(c)]) })
  afterEach((c) => { log.push(['ae', c.task.name, ...r(c)]) })
  test('two', { repeats: 2 }, (c) => {
    log.push(['t', c.task.name, ...r(c), c.task.repeats])
    c.onTestFinished(() => log.push(['fin', ...r(c)]))
  })
  let n = 0
  test('retry', { repeats: 2, retry: 2 }, (c) => {
    n++
    log.push(['t', c.task.name, n, ...r(c)])
    if (n === 1 || n === 4) throw new Error('flaky ' + n)
  })
  test('zero', { repeats: 0 }, (c) => { log.push(['t', c.task.name, ...r(c)]) })
  describe.skip('skipped', () => { test('x', { repeats: 3 }, () => log.push('never')) })
})
describe('inherit', { repeats: 1 }, () => {
  test('in', (c) => { log.push(['t', c.task.name, ...r(c), c.task.repeats]) })
  test('own', { repeats: 2 }, (c) => { log.push(['t', c.task.name, ...r(c)]) })
})
