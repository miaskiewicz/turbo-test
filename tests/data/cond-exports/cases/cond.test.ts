import { createRequire } from 'node:module';
import { which as reqFirst } from 'req-first';
import { which as reqImport } from 'req-import';
import { which as defaultFirst } from 'default-first';
import { which as nested } from 'nested';
import { which as feature } from 'subpath/feature';
import { which as ua } from 'subpath/u/a';

const require = createRequire(import.meta.url);
const tail = (u: string) => u.slice(u.indexOf('/node_modules/') + '/node_modules/'.length);

test('import picks the first matching key in exports order', () => {
  expect([reqFirst, reqImport, defaultFirst, nested, feature, ua]).toEqual([
    'req-first:js', 'req-import:mjs', 'default-first:default', 'nested:mjs',
    'subpath/feature:js', 'subpath/u/a:js',
  ]);
});
test('dynamic import agrees', async () => {
  expect((await import('req-first')).which).toBe('req-first:js');
  expect((await import('subpath/u/a')).which).toBe('subpath/u/a:js');
});
test('import.meta.resolve uses the import conditions', () => {
  expect(['req-first', 'req-import', 'default-first', 'nested', 'subpath/feature', 'subpath/u/a']
    .map((s) => tail(import.meta.resolve(s)))).toEqual([
    'req-first/dist/i.js', 'req-import/e.mjs', 'default-first/d.js', 'nested/m/i.mjs',
    'subpath/f.js', 'subpath/u/a.js',
  ]);
});
test('require() picks the require condition', () => {
  expect(['req-first', 'req-import', 'default-first', 'nested', 'subpath/feature', 'subpath/u/a', 'req-only']
    .map((s) => require(s).which)).toEqual([
    'req-first:cjs', 'req-import:cjs', 'default-first:default', 'nested:cjs',
    'subpath/feature:cjs', 'subpath/u/a:cjs', 'req-only:cjs',
  ]);
});
