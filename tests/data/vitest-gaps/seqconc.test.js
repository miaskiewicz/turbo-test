import { describe, test, afterAll } from 'vitest'
const log = []
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
afterAll(() => console.log('ORDER seqconc ' + JSON.stringify(log)))
// sequence.concurrent: every test is concurrent; maxConcurrency 2
test('a', async () => { log.push('a0'); await sleep(80); log.push('a1') })
test('b', async () => { log.push('b0'); await sleep(20); log.push('b1') })
test('b2', async () => { log.push('b2-0'); await sleep(20); log.push('b2-1') })
test.sequential('c', async () => { log.push('c0'); await sleep(10); log.push('c1') })
describe.sequential('ds', () => {
  test('d', async () => { log.push('d0'); await sleep(30); log.push('d1') })
  test('e', async () => { log.push('e0'); await sleep(10); log.push('e1') })
})
describe('dc', () => {
  test('f', async () => { log.push('f0'); await sleep(30); log.push('f1') })
  test('g', async () => { log.push('g0'); await sleep(10); log.push('g1') })
})
