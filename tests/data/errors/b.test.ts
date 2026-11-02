import { it } from 'vitest';
it('multi', () => { throw 'line one\nline two'; });
it('undef', () => { throw undefined; });
it('number', () => { throw 42; });
it('empty', () => { throw ''; });
it('named', () => { throw { name: 'Custom', message: 'msg', code: 7 }; });
it('nameonly', () => { throw { name: 'OnlyName', list: [1, 'two'] }; });
class MyError extends Error { constructor(m) { super(m); this.name = 'MyError'; this.when = 3; } describe() { return 'x'; } }
it('class', () => { throw new MyError('mine'); });
it('cause props', () => { const c = new Error('inner'); c.code = 'C'; throw new Error('outer', { cause: c }); });
it('fn prop', () => { const e = new TypeError('typed'); e.fn = function helper() {}; e.nested = { deep: { deeper: { deepest: 1 } } }; throw e; });
