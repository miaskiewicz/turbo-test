import { it } from 'vitest';
it('string', () => { throw 'plain string'; });
it('props', () => { const e = new Error('with code'); e.code = 'E_CODE'; e.extra = { n: 1 }; throw e; });
it('object', () => { throw { a: 1 }; });
it('null', () => { throw null; });
