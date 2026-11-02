import { test, expect } from 'vitest'
test('mo', () => { expect({ x: 3, y: { z: 1, w: 2 }, q: 1 }).toMatchObject({ x: expect.any(String), y: { z: 1 } }) })
test('closeTo diff', () => { expect({ a: 1.1 }).toEqual({ a: expect.closeTo(1) }) })
