jest.retryTimes(3)
let n = 0
test('passes-on-third-try', () => { n++; if (n < 3) throw new Error('nope ' + n) })
