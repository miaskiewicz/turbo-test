// toEqual & co. on arrays with undefined items (turbo-test#423): each case's pass/fail per
// matcher. `globalThis.EXPECTED` (set by the including test) holds the oracle's rows.
const same = (got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`got ${JSON.stringify(got)}\nwant ${JSON.stringify(want)}`);
};
const fn = () => (typeof jest !== 'undefined' ? jest.fn() : vi.fn());
const outcome = (f) => {
  try { f(); return 'pass'; } catch (e) { return 'fail'; }
};
const CASES = [
  ['trailing undefined', ['a', 'b', undefined], ['a', 'b']],
  ['expected trailing undefined', ['a', 'b'], ['a', 'b', undefined]],
  ['undefined vs hole', [1, undefined, 3], [1, , 3]],
  ['undefined property', { a: 1, b: undefined }, { a: 1 }],
  ['nested array', { x: [1, undefined] }, { x: [1] }],
  ['undefined vs null item', [undefined], [null]],
  ['empty vs undefined', [], [undefined]],
  ['two trailing', [1], [1, undefined, undefined]],
  ['undefined then value', [1], [undefined, 1]],
  ['array own undefined property', Object.assign([1], { p: undefined }), [1]],
];
const MATCHERS = {
  toEqual: (a, b) => expect(a).toEqual(b),
  toStrictEqual: (a, b) => expect(a).toStrictEqual(b),
  objectContaining: (a, b) => expect({ v: a }).toEqual(expect.objectContaining({ v: b })),
  calledWith: (a, b) => { const f = fn(); f(a); expect(f).toHaveBeenCalledWith(b); },
  toMatchObject: (a, b) => expect({ v: a }).toMatchObject({ v: b }),
  arrayContaining: (a, b) => expect([a]).toEqual(expect.arrayContaining([b])),
};
test('undefined array items per matcher', () => {
  const rows = CASES.map(([name, a, b]) => [name, ...Object.values(MATCHERS).map((m) => outcome(() => m(a, b)))]);
  if (!globalThis.EXPECTED) console.log(JSON.stringify(rows));
  same(rows, globalThis.EXPECTED);
});
test('call arity stays strict', () => {
  const f = fn();
  f();
  f(1, undefined);
  same([
    outcome(() => expect(f).toHaveBeenNthCalledWith(1, undefined)),
    outcome(() => expect(f).toHaveBeenNthCalledWith(2, 1)),
    outcome(() => expect(f).toHaveBeenLastCalledWith(1, undefined)),
  ], ['fail', 'fail', 'pass']);
});
