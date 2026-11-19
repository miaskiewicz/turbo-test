const rec = (...args) => { throw new Error('ARGS ' + args.length + ':' + JSON.stringify(args)); };
it.failing.each([[1], [2]])('failing %s', (n) => { throw new Error('expected ' + n); });
it.failing.each`
  v
  ${'t'}
`('failing tagged $v', () => {});
it.each([])('empty table %s', rec);
it.each`no data`('tagged no data', rec);
it.each`
  a | b
  ${1}
`('tagged missing $a', rec);
it.each('nope')('not an array %s', rec);
describe.skip.each([[1]])('skipped suite %s', () => { it('inner', () => {}); });
it.each([[10n], [{ n: 10n }]])('bigint j %j p %p', rec);
