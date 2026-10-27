import { defineConfig } from 'vitest/config'
export default defineConfig({
  test: {
    // project-wide fake-timer settings (turbo-test#180)
    fakeTimers: { now: 1000, toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'], loopLimit: 7 },
  },
})
