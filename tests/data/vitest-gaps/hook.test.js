import { describe, test, beforeAll, beforeEach, afterEach, afterAll } from 'vitest'
const log = []
const hang = () => new Promise(() => {})
afterAll(() => console.log('ORDER hook ' + JSON.stringify(log)))
describe('be', () => {
  beforeEach(hang, 30)
  afterEach(() => { log.push('be ae') })
  test('t', () => { log.push('never be') })
  afterEach((c) => { log.push('be ' + c.task.result.errors.map((e) => e.message.split('\n')[0]).join('|')) })
})
describe('ae', () => {
  afterEach((c) => { log.push('ae signal ' + c.signal.aborted); return hang() }, 40)
  test('t', (c) => { c.onTestFinished(() => log.push('fin ' + c.task.result.state + ' ' + c.task.result.errors[0].message.split('\n')[0] + ' ' + c.signal.aborted)) })
})
describe('ba', () => {
  beforeAll(hang, 20)
  test('t', () => { log.push('never ba') })
  afterAll(() => { log.push('ba aa runs') })
})
describe('aa', () => {
  afterAll(hang, 25)
  test('t', () => { log.push('aa t') })
})
describe('cfg', () => {
  // the config's hookTimeout (60)
  beforeEach(hang)
  afterEach((c) => { log.push('cfg ' + c.task.result.errors[0].message.split('\n')[0]) })
  test('t', () => {})
})
describe('ok', () => {
  beforeEach(() => new Promise((r) => setTimeout(r, 20)), 200)
  test('t', () => { log.push('ok t') })
  beforeAll(() => new Promise((r) => setTimeout(r, 20)), 0)
})
describe('cleanup', () => {
  beforeEach(() => () => hang(), 35)
  test('t', (c) => { c.onTestFailed(() => log.push('cleanup ' + c.task.result.errors[0].message.split('\n')[0])) })
})
describe('fin', () => {
  test('t', (c) => {
    c.onTestFailed(() => log.push('fin ' + c.task.result.errors[0].message.split('\n')[0] + ' ' + c.signal.aborted))
    c.onTestFinished(hang, 45)
  })
  test('cfg', (c) => {
    c.onTestFailed(() => log.push('fin cfg ' + c.task.result.errors[0].message.split('\n')[0]))
    c.onTestFinished(hang)
  })
})
describe('ba-cleanup', () => {
  beforeAll(() => () => hang(), 15)
  test('t', () => { log.push('ba-cleanup t') })
})
