// turbo-test#268: --hookTimeout 0 disables the timeout entirely, as vitest.
import { it, beforeEach, afterAll } from 'vitest'
const log = []
beforeEach(async () => { await new Promise(r => setTimeout(r, 10200)); log.push('be done') })
it('slow-hook-passes-with-timeout-0', () => { log.push('t runs') })
afterAll(() => console.log('ORDER hookzero ' + JSON.stringify(log)))
