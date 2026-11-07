// turbo-test#273: --testTimeout 0 disables the test timeout entirely.
import { it, afterAll } from 'vitest'
const log = []
it('slow-test-passes-with-timeout-0', async () => { await new Promise(r => setTimeout(r, 6000)); log.push('t done') })
afterAll(() => console.log('ORDER testzero ' + JSON.stringify(log)))
