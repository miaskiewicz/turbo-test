jest.setTimeout(100)
test('times-out-at-jest-setTimeout', async () => { await new Promise(r => setTimeout(r, 300)) })
