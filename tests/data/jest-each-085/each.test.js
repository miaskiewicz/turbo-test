// Every test throws `ARGS <signature of its arguments>` so the report carries what it received.
const sig = (args) => args.length + ':' + args.map((a) => {
  if (a === undefined) return 'u';
  if (typeof a === 'bigint') return a + 'n';
  if (typeof a === 'function') return 'fn:' + a.name;
  if (a instanceof Date) return 'date:' + a.toISOString();
  try { return JSON.stringify(a); } catch { return 'cyclic'; }
}).join('|');
const rec = (...args) => { throw new Error('ARGS ' + sig(args)); };
const cyc = { name: 'c' }; cyc.self = cyc;
class Thing {}
function namedFn() {}
const long = 'L'.repeat(60);
const date = new Date('2024-01-02T03:04:05.000Z');

it.each([['a', 1], ['b', 2.5]])('s %s d %d', rec);
it.each([['7.9x', '3.7', '1.5y']])('i %i f %f d %d', rec);
it.each([['str'], [42], [null], [undefined], [true], [{ a: 1, b: { c: [1, 2] } }], [[1, [2, [3]]]], [date], [namedFn], [Thing], [long], [-0], [NaN]])('p %p', rec);
it.each([[{ a: 1 }], [[1, 'x']], ['s'], [cyc]])('j %j', rec);
it.each([[{ a: 1, b: { c: { d: 1 } } }], ['str'], [[1, 2]]])('s %s o %o O %O', rec);
it.each([['x', 'y']])('idx %# num %$ pct %% s %s', rec);
it.each([['100%s'], ['50%p']])('pct value %s then %s', rec);
it.each([1, 2, 3])('flat %i', rec);
it.each([null, [1], [1, 2]])('mixed %j', rec);
it.each([[1, 2, 3]])('more values %s', rec);
it.each([[1]])('more placeholders %s %s %p', rec);
it.each([[long]])('long %s', rec);
it.each([[{ a: 1 }, 'b']])('object s %s and %s', rec);
it.each([{ a: 1, b: { c: 'deep' } }, { a: 'two', b: { c: [1] } }])('object $a $b.c $b $# $missing', rec);
it.each([{ a: 1 }, { a: 2 }])('object row p %p', rec);
it.each([{ a: 1 }, 'plain'])('object mixed $a %s', rec);
it.each`
  a       | b             | obj
  ${1}    | ${'x'}        | ${{ k: { z: 1 } }}
  ${null} | ${[1, 2]}     | ${undefined}
`('tagged $a $b $obj $obj.k $obj.k.z $# %s', rec);
describe.each([['d1', 1], ['d2', 2]])('suite %s %d', (name, n) => {
  it('inner', () => rec(name, n));
});
describe.each`
  x
  ${'tx'}
`('tsuite $x', ({ x }) => {
  it('inner', () => rec(x));
});
it.skip.each([[1]])('skipped %s', rec);
it.concurrent.each([['c']])('concurrent %s', rec);
