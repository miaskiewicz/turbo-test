import { test, beforeEach, afterEach, afterAll } from 'vitest'
const log = []
afterAll(() => console.log('ORDER hookdef ' + JSON.stringify(log)))
beforeEach(() => new Promise(() => {}))
afterEach((c) => { log.push(c.task.result.state + ' ' + c.task.result.errors[0].message) })
test('t', () => { log.push('never') })
