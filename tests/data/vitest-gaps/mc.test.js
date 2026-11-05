import { describe, test, afterAll, expect } from 'vitest'
const log = []
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
afterAll(() => console.log('ORDER mc ' + JSON.stringify(log)))
describe.concurrent('d', () => {
test('a', async () => { log.push('a0'); await sleep(80); log.push('a1') })
test('b', async () => { log.push('b0'); await sleep(20); log.push('b1') })
test('b2', async () => { log.push('b2-0'); await sleep(20); log.push('b2-1') })
})
