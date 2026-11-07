import { appendFileSync, readFileSync } from 'node:fs'
jest.retryTimes(3)
test('passes-on-third-try', () => {
  appendFileSync(process.env.OUT, 'a')
  const n = readFileSync(process.env.OUT, 'utf8').length
  if (n < 3) throw new Error('nope attempt ' + n)
})
