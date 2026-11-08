import { describe, it, beforeAll, afterAll, beforeEach, afterEach, expect } from 'vitest'
const log = []
const probe = (tag) => {
  log.push(tag)
  queueMicrotask(() => log.push(tag + ':m1'))
  Promise.resolve().then(() => Promise.resolve()).then(() => log.push(tag + ':m3'))
  setTimeout(() => log.push(tag + ':t0'), 0)
}
describe('s', () => {
  beforeAll(() => probe('ba'))
  beforeEach(() => probe('be'))
  afterEach(() => probe('ae'))
  afterAll(() => probe('aa'))
  it('a', () => probe('a'))
  it('b', async () => { probe('b'); await null; probe('b2') })
  it.fails('c', () => { probe('c'); throw new Error('x') })
})
it('report', () => { console.log('ORDER order ' + JSON.stringify(log)) })
