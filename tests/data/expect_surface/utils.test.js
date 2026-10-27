import { test, expect } from 'vitest'
import * as fs from 'node:fs'
const rows = []
const show = (v) => { try { return v === undefined ? 'undefined' : typeof v === 'function' ? 'fn' : typeof v === 'bigint' ? v + 'n' : JSON.stringify(v) } catch { return String(v) } }
const t = (name, f) => { let r; try { const v = f(); r = v === undefined ? 'passed' : ['value', show(v)] } catch (e) { r = [e && e.name, e && e.message] } rows.push(JSON.stringify([name, r])) }
let u
expect.extend({
  grab() { u = this.utils; return { pass: true, message: () => '' } },
  toBeK5(received, expected) {
    const pass = received === expected
    return { pass, message: () => `${this.utils.matcherHint('toBeK5', undefined, undefined, { isNot: this.isNot, promise: this.promise })}\n\nExpected: ${this.utils.printExpected(expected)}\nReceived: ${this.utils.printReceived(received)}` }
  },
  toEqualK5(received, expected) {
    return { pass: false, message: () => `${this.utils.matcherHint('.toEqualK5')}\n\n${this.utils.diff(expected, received)}` }
  },
})
test('utils', async () => {
  expect(1).grab()
  const circ = { a: 1 }; circ.self = circ
  const deep = { a: { b: { c: { d: { e: { f: { g: { h: { i: { j: { k: { l: 1 } } } } } } } } } } } };
  class K { constructor() { this.x = 1 } };
  const values = [{ a: [1] }, 'str  ', 'a\nb  \nc', 'q"uote\\', undefined, null, -0, 0, 1.5, NaN, 1n, Symbol('s'), function named() {}, () => {}, new Map([[1, { a: 2 }]]), new Set([1, 'a']), new Date(0), new Error('boom'), new TypeError('te'), /a.b/g, deep, circ, new K(), [], {}, [undefined, null], expect.any(Number), { m: expect.objectContaining({ a: 1 }) }, new Uint8Array([1, 2]), Object.create(null), { [Symbol('k')]: 1, b: 2, a: 1 }, true, Array.from({ length: 3 }, (_, i) => ({ i }))]
  t('keys', () => Object.keys(u).sort().map((k) => k + ':' + typeof u[k]))
  values.forEach((v, i) => t(`printReceived ${i}`, () => u.printReceived(v)))
  values.forEach((v, i) => t(`stringify ${i}`, () => u.stringify(v)))
  t('printExpected', () => u.printExpected({ a: [1], s: 'x  ' }))
  t('stringify depth 1', () => u.stringify({ a: { b: [1] }, c: 1 }, 1))
  t('stringify depth 0', () => u.stringify({ a: { b: [1] } }, 0))
  t('stringify maxLength', () => u.stringify({ a: { b: { c: { d: 'x'.repeat(30) } } } }, 10, { maxLength: 40 }))
  t('stringify min', () => u.stringify({ a: [1, { b: 2 }] }, 10, { min: true }))
  t('stringify escape', () => u.stringify('a"b', 10, { escapeString: true }))
  t('matcherHint .toK', () => u.matcherHint('.toK'))
  t('matcherHint toK', () => u.matcherHint('toK'))
  t('matcherHint args', () => u.matcherHint('toK', 'r', 'e'))
  t('matcherHint no expected', () => u.matcherHint('toK', undefined, ''))
  t('matcherHint no received', () => u.matcherHint('toK', '', 'e'))
  t('matcherHint isNot', () => u.matcherHint('toK', undefined, undefined, { isNot: true }))
  t('matcherHint promise', () => u.matcherHint('toK', undefined, undefined, { promise: 'rejects', isNot: true }))
  t('matcherHint .old isNot', () => u.matcherHint('.not.toK', undefined, undefined, { isNot: true }))
  t('matcherHint comment', () => u.matcherHint('toK', 'r', 'e', { comment: 'deep equality', secondArgument: 'n' }))
  t('matcherHint direct', () => u.matcherHint('toK', 'r', 'e', { isDirectExpectCall: true }))
  t('matcherHint colors', () => u.matcherHint('toK', 'r', 'e', { expectedColor: (s) => `<${s}>`, receivedColor: (s) => `[${s}]`, secondArgument: 'z', secondArgumentColor: (s) => `{${s}}` }))
  t('diff objects', () => u.diff({ a: 1, b: [1, 2] }, { a: 2, b: [1, 2] }))
  t('diff omit', () => u.diff({ a: 1 }, { a: 2 }, { omitAnnotationLines: true }))
  t('diff strings', () => u.diff('abc\ndef', 'abc\ndeg'))
  t('diff string line', () => u.diff('abc', 'abd'))
  t('diff numbers', () => u.diff(1, 2))
  t('diff same', () => u.diff({ a: 1 }, { a: 1 }))
  t('diff identical', () => u.diff(1, 1))
  t('diff types', () => u.diff(1, 'a'))
  t('diff null', () => u.diff(null, undefined))
  t('diff anno', () => u.diff({ a: 1 }, { a: 2 }, { aAnnotation: 'Want', bAnnotation: 'Got' }))
  t('printDiffOrStringify', () => u.printDiffOrStringify('abc', 'abd'))
  t('printDiffOrStringify obj', () => u.printDiffOrStringify({ a: 1 }, { a: 2 }))
  t('printDiffOrStringify lines', () => u.printDiffOrStringify('a\nb', 'a\nc'))
  t('printDiffOrStringify num', () => u.printDiffOrStringify(1, 2))
  t('printWithType', () => u.printWithType('Received', 1, u.printReceived))
  t('printWithType null', () => u.printWithType('Received', null, u.printReceived))
  t('printWithType undefined', () => u.printWithType('Expected', undefined, u.printExpected))
  t('printWithType obj', () => u.printWithType('Received', { a: 1 }, u.stringify))
  t('printWithType array', () => u.printWithType('Received', [1], u.stringify))
  t('colors', () => [u.EXPECTED_COLOR('e'), u.RECEIVED_COLOR('r'), u.INVERTED_COLOR('i'), u.BOLD_WEIGHT('b'), u.DIM_COLOR('d')])
  t('iterableEquality', () => [u.iterableEquality([1], [1]), u.iterableEquality([1], [2]), u.iterableEquality(1, 2), u.subsetEquality({ a: 1, b: 2 }, { a: 1 }), u.subsetEquality({ a: 1 }, { a: 2 })])
  t('k5 fail', () => expect(1).toBeK5(2))
  t('k5 not fail', () => expect(2).not.toBeK5(2))
  t('k5 resolves', async () => { try { await expect(Promise.resolve(1)).resolves.toBeK5(2) } catch (e) { rows.push(JSON.stringify(['k5 resolves msg', e.message])) } })
  t('k5 diff', () => expect({ a: 1 }).toEqualK5({ a: 2 }))
  await new Promise((r) => setTimeout(r, 1))
  fs.writeFileSync('out.txt', rows.join('\n') + '\n')
})
