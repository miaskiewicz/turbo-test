import { test, afterAll } from 'vitest'
const log = []
afterAll(() => console.log('ORDER repfail ' + JSON.stringify(log)))
let n = 0
test('fails-in-repeat-1', { repeats: 2 }, (c) => {
  n++
  log.push([n, c.task.result.repeatCount, c.task.result.retryCount, c.task.result.state])
  if (n === 2) throw new Error('repeat ' + n)
})
test.fails('fails', { repeats: 1 }, () => { log.push('f'); throw new Error('x') })
