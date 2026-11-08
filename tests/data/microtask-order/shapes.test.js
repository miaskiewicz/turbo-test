import { describe, it, afterAll, beforeAll } from 'vitest'
const log = []
let gen = 0, cur = 0
const mark = (tag) => {
  log.push(tag + '@' + cur)
  const g = ++gen; cur = 0
  const step = () => { if (g !== gen || cur > 400) return; cur++; queueMicrotask(step) }
  queueMicrotask(step)
}
it('p0', () => mark('p0'))
it('p1', () => mark('p1'))
describe('x', () => { it('x1', () => mark('x1')) })
it('p2', () => mark('p2'))
describe('y', () => { it('y1', () => mark('y1')) })
describe('z', () => { it('z1', () => mark('z1')) })
it.skip('s', () => {})
it('p3', () => mark('p3'))
describe.skip('h', () => { it('s2', () => {}) })
it('p4', () => mark('p4'))
describe('w', () => { it('w1', () => mark('w1')); afterAll(() => mark('waa')) })
describe('v', () => { beforeAll(() => mark('vba')); it('v1', () => mark('v1')) })
describe('u', () => { describe('u2', () => { it('u1', () => mark('u1')) }) })
it('rep', () => { mark('rep'); console.log('ORDER shapes ' + JSON.stringify(log)) })
