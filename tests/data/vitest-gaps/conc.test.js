import { describe, test, it, beforeEach, afterEach, afterAll } from 'vitest'
const log = []
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
afterAll(() => console.log('ORDER conc ' + JSON.stringify(log)))
describe.concurrent('dc', () => {
  beforeEach((c) => { log.push('be ' + c.task.name) })
  afterEach((c) => { log.push('ae ' + c.task.name) })
  test('a', async () => { log.push('a0'); await sleep(120); log.push('a1') })
  test('b', async () => { log.push('b0'); await sleep(40); log.push('b1') })
  test('c', () => { log.push('c') })
})
describe('mixed', () => {
  test('s1', async () => { log.push('s1-0'); await sleep(10); log.push('s1-1') })
  it.concurrent('p1', async () => { log.push('p1-0'); await sleep(80); log.push('p1-1') })
  it.concurrent('p2', async () => { log.push('p2-0'); await sleep(20); log.push('p2-1') })
  test('s2', () => { log.push('s2') })
  test.concurrent('p3', async () => { log.push('p3') })
})
describe('limit', () => {
  // seven concurrent tests, five at a time (maxConcurrency)
  for (let i = 0; i < 7; i++) test.concurrent('l' + i, async () => { log.push('l' + i + '-0'); await sleep(i === 0 ? 50 : 200 - i * 20); log.push('l' + i + '-1') })
})
describe.concurrent('nested', () => {
  describe('n1', () => {
    test('x', async () => { log.push('x0'); await sleep(60); log.push('x1') })
    test('y', async () => { log.push('y0'); await sleep(20); log.push('y1') })
  })
  test.sequential('seq', async () => { log.push('seq0'); await sleep(10); log.push('seq1') })
  test('z', async () => { log.push('z0'); await sleep(10); log.push('z1') })
})
describe('fail', () => {
  test.concurrent.fails('f', async () => { log.push('f0'); await sleep(20); throw new Error('f') })
  test.concurrent('g', async () => { log.push('g0'); await sleep(60); log.push('g1') })
  test.concurrent.skip('h', () => { log.push('never') })
  it.concurrent.each([1, 2])('e%i', async (n) => { log.push('e' + n + '-0'); await sleep(50 - n * 20); log.push('e' + n + '-1') })
})
