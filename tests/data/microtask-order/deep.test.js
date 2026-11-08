import { it, beforeEach, afterEach } from 'vitest'
const log = []
const chain = (tag, n) => { let p = Promise.resolve(); for (let i = 0; i < n; i++) p = p.then(() => {}); p.then(() => log.push(tag + ':m' + n)) }
const probe = (tag) => { log.push(tag); for (const n of [1, 5, 20, 100, 1000]) chain(tag, n); setTimeout(() => log.push(tag + ':t0'), 0) }
beforeEach(() => probe('be'))
afterEach(() => probe('ae'))
it('a', () => probe('a'))
it('b', () => probe('b'))
it('report', () => console.log('ORDER deep ' + JSON.stringify(log)))
