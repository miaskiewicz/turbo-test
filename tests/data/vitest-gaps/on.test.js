import { describe, test, expect, beforeEach, afterEach, afterAll, onTestFinished, onTestFailed } from 'vitest'
const log = []
const msg = (f) => { try { f(); return 'ok' } catch (e) { return e.constructor.name + ': ' + e.message } }
log.push('top ' + msg(() => onTestFinished(() => {})))
log.push('top ' + msg(() => onTestFailed(() => {})))
afterAll(() => console.log('ORDER on ' + JSON.stringify(log)))
describe('s', () => {
  beforeEach((ctx) => { onTestFinished((c) => log.push('be-fin ' + c.task.name + ' ' + (c === ctx))) })
  afterEach((ctx) => { onTestFinished(() => log.push('ae-fin ' + ctx.task.name)) })
  test('a', () => {
    onTestFinished(() => log.push('fin1 a'))
    onTestFinished(() => log.push('fin2 a'))
    onTestFailed(() => log.push('failed a'))
    log.push('type ' + msg(() => onTestFinished(1)))
    log.push('type ' + msg(() => onTestFailed('x')))
  })
  test.fails('b', () => {
    onTestFailed((c) => {
      log.push('failed b ' + c.task.result.state + ' ' + c.task.result.errors[0].message)
      log.push('inner ' + msg(() => c.onTestFinished(() => {})))
    })
    onTestFinished(() => log.push('fin b'))
    expect(1).toBe(2)
  })
  let n = 0
  test('retry', { retry: 1 }, () => {
    n++
    onTestFinished(() => log.push('fin retry ' + n))
    onTestFailed(() => log.push('failed retry ' + n))
    if (n === 1) throw new Error('first')
  })
  test('async', async () => {
    onTestFinished(async () => { await null; log.push('fin async') })
    await null
  })
})
