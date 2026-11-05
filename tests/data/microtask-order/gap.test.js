import { describe, it, beforeAll, afterAll, beforeEach, afterEach } from 'vitest'
const log = []
let gen = 0, cur = 0
const mark = (tag) => {
  log.push(tag + '@' + cur)
  const g = ++gen; cur = 0
  const step = () => { if (g !== gen || cur > 400) return; cur++; queueMicrotask(step) }
  queueMicrotask(step)
}
const am = (tag) => async () => { mark(tag) }
const aw = (tag) => async () => { await null; mark(tag) }
beforeAll(() => mark('fba'))
beforeAll(am('fba2'))
beforeEach(() => mark('fbe'))
beforeEach(am('fbe2'))
afterEach(() => mark('fae'))
afterEach(aw('fae2'))
afterAll(() => mark('faa'))
it('t1', () => mark('t1'))
it('t2', am('t2'))
it('t3', aw('t3'))
describe('d', () => {
  beforeAll(() => mark('dba'))
  beforeEach(() => { mark('dbe'); return () => mark('dbeC') })
  afterEach(() => mark('dae'))
  afterAll(() => mark('daa'))
  it('t4', (ctx) => { ctx.onTestFinished(() => mark('otf')); ctx.onTestFinished(async () => mark('otf2')); mark('t4') })
  let n = 0
  it('t5', { retry: 1 }, () => { mark('t5'); if (!n++) throw new Error('r') })
  it.fails('t6', () => { mark('t6'); throw new Error('x') })
  describe('e', () => { it('t7', () => mark('t7')) })
  describe('f', () => { beforeAll(aw('fba-in')); it('t8', aw('t8')) })
})
describe('g', () => { it('t9', () => mark('t9')) })
it.skip('s1', () => mark('s1'))
describe.skip('h', () => { it('s2', () => mark('s2')) })
it('t10', () => mark('t10'))
beforeAll(() => () => mark('fbaC'))
it('report', () => { mark('rep'); console.log('ORDER gap ' + JSON.stringify(log)) })
