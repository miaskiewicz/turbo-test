import { test, expect, vi } from 'vitest'
import * as fs from 'node:fs'
// turbo-test#103: toEqual / toStrictEqual and the matchers built on @vitest/expect's `equals`
// (Dates, Maps, Sets, RegExps, boxed primitives, symbol keys, Errors, typed arrays, circular
// structures), each with its matcher's testers.
test('equality', () => {
  const out = []
  class Foo { constructor() { this.a = 1 } get g() { return 1 } }
  class E extends Error {}
  const nonEnum = Object.defineProperty({}, 'h', { value: 1, enumerable: false })
  const iter = (n) => ({ *[Symbol.iterator]() { yield n } })
  const circ = (x) => { const o = { x }; o.self = o; return o }
  const spy = vi.fn(); spy(new Date(1), new Map([[1, 2]]))
  const ret = vi.fn(() => new Error('a')); ret()
  for (const [label, fn] of [
    ['dates', () => expect(new Date(1)).toEqual(new Date(2))],
    ['maps', () => expect(new Map([[1, 2]])).toEqual(new Map([[1, 3]]))],
    ['sets', () => expect(new Set([1])).toEqual(new Set([2]))],
    ['regex', () => expect(/a/).toEqual(/b/)],
    ['regex flags', () => expect(/a/g).toEqual(/a/i)],
    ['boxed', () => expect(new Number(1)).toEqual(new Number(2))],
    ['boxed string', () => expect(new String('a')).toEqual(new String('b'))],
    ['boxed vs primitive', () => expect(new Number(1)).toEqual(1)],
    ['array extra props', () => expect(Object.assign([1], { x: 1 })).toEqual([1])],
    ['symbol keys', () => expect({ [Symbol.for('s')]: 1 }).toEqual({ [Symbol.for('s')]: 2 })],
    ['symbol key missing', () => expect({ [Symbol.for('s')]: 1 }).toEqual({})],
    ['symbol key undefined', () => expect({ [Symbol.for('s')]: undefined }).toEqual({})],
    ['errors', () => expect(new Error('a')).toEqual(new Error('b'))],
    ['errors name', () => expect(new TypeError('a')).toEqual(new Error('a'))],
    ['errors cause', () => expect(new Error('a', { cause: 1 })).toEqual(new Error('a', { cause: 2 }))],
    ['errors props', () => expect(Object.assign(new Error('a'), { code: 1 })).toEqual(new Error('a'))],
    ['error vs object', () => expect(new Error('a')).toEqual({ message: 'a' })],
    ['date vs object', () => expect(new Date(1)).toEqual({})],
    ['map size', () => expect(new Map([[1, 2]])).toEqual(new Map())],
    ['set vs array', () => expect(new Set([1])).toEqual([1])],
    ['map vs object', () => expect(new Map()).toEqual({})],
    ['typed arrays', () => expect(new Uint8Array([1])).toEqual(new Uint8Array([2]))],
    ['typed array kinds', () => expect(new Uint8Array([1])).toEqual(new Int8Array([1]))],
    ['zero', () => expect(0).toEqual(-0)],
    ['object zero', () => expect({ a: 0 }).toEqual({ a: -0 })],
    ['functions', () => expect(() => 1).toEqual(() => 1)],
    // URLs by href (the message is left out: it prints the URL)
    ['urls', () => expect(() => expect(new URL('http://a/')).toEqual(new URL('http://b/'))).toThrow()],
    ['circular differ', () => expect(circ(1)).toEqual(circ(2))],
    ['custom iterables differ', () => expect(iter(1)).toEqual(iter(2))],
    ['array vs array-like', () => expect([1]).toEqual({ 0: 1 })],
    ['sparse strict', () => expect([, 1]).toStrictEqual([undefined, 1])],
    ['null prototype strict', () => expect(Object.assign(Object.create(null), { a: 1 })).toStrictEqual({ a: 1 })],
    ['error subclass strict', () => expect(new E('a')).toStrictEqual(new Error('a'))],
    ['typed array kinds strict', () => expect(new Uint8Array([1])).toStrictEqual(new Int8Array([1]))],
    ['zero strict', () => expect(0).toStrictEqual(-0)],
    ['not.toEqual dates', () => expect(new Date(1)).not.toEqual(new Date(1))],
    ['not.toStrictEqual maps', () => expect(new Map([[1, 2]])).not.toStrictEqual(new Map([[1, 2]]))],
    ['calledWith date', () => expect(spy).toHaveBeenCalledWith(new Date(2), new Map([[1, 2]]))],
    ['calledWith map', () => expect(spy).toHaveBeenCalledWith(new Date(1), new Map([[1, 3]]))],
    ['returnedWith error', () => expect(ret).toHaveReturnedWith(new Error('b'))],
    ['objectContaining date', () => expect({ d: new Date(1) }).toEqual(expect.objectContaining({ d: new Date(2) }))],
    ['objectContaining symbol', () => expect({ [Symbol.for('s')]: 1 }).toEqual(expect.objectContaining({ [Symbol.for('s')]: 2 }))],
    ['arrayContaining regex', () => expect([/a/]).toEqual(expect.arrayContaining([/b/]))],
    ['toMatchObject date', () => expect({ d: new Date(1) }).toMatchObject({ d: new Date(2) })],
    ['toMatchObject set', () => expect({ s: new Set([1]) }).toMatchObject({ s: new Set([2]) })],
    ['toContainEqual date', () => expect([new Date(1)]).toContainEqual(new Date(2))],
    ['toHaveProperty regex', () => expect({ r: /a/ }).toHaveProperty('r', /b/)],
    // equal (vitest passes)
    ['eq invalid dates', () => expect(new Date(NaN)).toEqual(new Date(NaN))],
    ['eq dates', () => expect(new Date(1)).toEqual(new Date(1))],
    ['eq maps order', () => expect(new Map([[1, 'a'], [2, 'b']])).toEqual(new Map([[2, 'b'], [1, 'a']]))],
    ['eq maps object keys', () => expect(new Map([[{ a: 1 }, 'x']])).toEqual(new Map([[{ a: 1 }, 'x']]))],
    ['eq sets objects', () => expect(new Set([{ a: 1 }, { b: 2 }])).toEqual(new Set([{ b: 2 }, { a: 1 }]))],
    ['eq nested', () => expect({ m: new Map([[1, new Set([1])]]) }).toEqual({ m: new Map([[1, new Set([1])]]) })],
    ['eq regex', () => expect(/a/gi).toEqual(/a/ig)],
    ['eq boxed', () => expect(new String('a')).toEqual(new String('a'))],
    ['eq errors', () => expect(new Error('a')).toEqual(new Error('a'))],
    ['eq error subclass', () => expect(new E('a')).toEqual(new Error('a'))],
    ['eq symbol keys', () => expect({ [Symbol.for('s')]: 1 }).toEqual({ [Symbol.for('s')]: 1 })],
    ['eq sparse', () => expect([, 1]).toEqual([undefined, 1])],
    ['eq undefined key', () => expect({ a: undefined, b: 1 }).toEqual({ b: 1 })],
    ['eq asymmetric in map', () => expect(new Map([['k', { a: 1, t: 5 }]])).toEqual(new Map([['k', { a: 1, t: expect.any(Number) }]]))],
    ['eq asymmetric in set', () => expect(new Set([1, 'x'])).toEqual(new Set([1, expect.any(String)]))],
    ['eq circular', () => expect(circ(1)).toEqual(circ(1))],
    ['eq non-enumerable', () => expect(nonEnum).toEqual({})],
    ['eq class vs object', () => expect(new Foo()).toEqual({ a: 1 })],
    ['eq typed arrays', () => expect(new Uint8Array([1, 2])).toEqual(new Uint8Array([1, 2]))],
    ['eq array buffers', () => expect(new Uint8Array([1]).buffer).toEqual(new Uint8Array([2]).buffer)],
    ['eq null prototype', () => expect(Object.assign(Object.create(null), { a: 1 })).toEqual({ a: 1 })],
    ['eq custom iterables', () => expect(iter(1)).toEqual(iter(1))],
    ['eq urls', () => expect(new URL('http://a/')).toEqual(new URL('http://a/'))],
    ['eq NaN', () => expect(NaN).toEqual(NaN)],
    ['eq maps strict', () => expect(new Map([[1, { a: 1 }]])).toStrictEqual(new Map([[1, { a: 1 }]]))],
    ['eq toContainEqual map', () => expect([new Map([[1, 2]])]).toContainEqual(new Map([[1, 3]]))],
    ['eq toHaveProperty set', () => expect({ s: new Set([1]) }).toHaveProperty('s', new Set([2]))],
    ['eq toMatchObject date', () => expect({ d: new Date(1), x: 1 }).toMatchObject({ d: new Date(1) })],
    ['eq calledWith', () => expect(spy).toHaveBeenCalledWith(new Date(1), new Map([[1, 2]]))],
  ]) {
    try { fn(); out.push({ label, passed: true }) } catch (e) { out.push({ label, name: e && e.name, message: e && e.message }) }
  }
  fs.writeFileSync('out4.json', JSON.stringify(out, null, 1))
})
