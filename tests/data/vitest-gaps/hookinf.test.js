// turbo-test#273: `test.hookTimeout: Infinity` in vitest config disables the hook timeout,
// matching vitest's withTimeout rule.
import { it, beforeEach, afterAll } from 'vitest'
const log = []
beforeEach(async () => { await new Promise(r => setTimeout(r, 10200)); log.push('be done') })
it('slow-hook-passes-under-infinity-config', () => { log.push('t runs') })
afterAll(() => console.log('ORDER hookinf ' + JSON.stringify(log)))
