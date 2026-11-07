import { describe, it, beforeAll, afterAll, beforeEach, afterEach } from 'vitest'
const log = []
let gen = 0, cur = 0
const mark = (tag) => {
  log.push(tag + '@' + cur)
  const g = ++gen; cur = 0
  const step = () => { if (g !== gen || cur > 400) return; cur++; queueMicrotask(step) }
  queueMicrotask(step)
}
describe('a', () => {
  beforeEach(async () => { await Promise.resolve(); mark('abe') })
  afterEach(() => new Promise((r) => { mark('aae'); r() }))
  describe('b', () => {
    beforeEach(() => mark('bbe'))
    afterEach(() => mark('bae'))
    describe('c', () => {
      beforeEach(() => { mark('cbe'); return async () => mark('cbeC') })
      it('c1', (ctx) => { ctx.onTestFailed(() => mark('otfail')); mark('c1'); throw new Error('boom') })
      it('c2', async () => { mark('c2'); await new Promise((r) => r()); mark('c2b') })
      it('c3', (ctx) => { mark('c3'); ctx.skip() })
      it.todo('c4')
      it('c5', () => { mark('c5'); return { then(r) { mark('then'); r() } } })
    })
  })
})
describe('fb', () => {
  beforeEach(() => { mark('fbe'); throw new Error('be') })
  afterEach(() => mark('fae'))
  it('f1', () => mark('f1'))
})
describe('fa', () => {
  beforeAll(() => { mark('faba'); throw new Error('ba') })
  afterAll(() => mark('faaa'))
  it('f2', () => mark('f2'))
  describe('fn', () => { it('f3', () => mark('f3')) })
})
describe('ae', () => {
  afterEach(() => { mark('aeX'); throw new Error('ae') })
  it('f4', () => mark('f4'))
})
it.each([1, 2])('each %i', (n) => mark('each' + n))
it('rep', () => { mark('rep'); console.log('ORDER edge ' + JSON.stringify(log)) })
