import { describe } from 'vitest';
import { createRequire } from 'node:module';
import { row } from './d';

const F = 'dynamic';
const req = createRequire(import.meta.url);
describe('import()', () => {
  row(F, 'mexp-fn', async () => (await import('mexp-fn')).default);
  row(F, 'mexp-fn ns', async () => await import('mexp-fn'));
  row(F, 'exp-default-flag', async () => (await import('exp-default-flag')).default);
  row(F, 'exp-default-flag ns', async () => await import('exp-default-flag'));
  row(F, 'exp-default-noflag', async () => (await import('exp-default-noflag')).default);
  row(F, 'tsc-out', async () => (await import('tsc-out')).default);
  row(F, 'babel-out', async () => (await import('babel-out')).default);
  row(F, 'exports-map', async () => (await import('exports-map')).default);
  row(F, 'esm-only ns', async () => await import('esm-only'));
  row(F, 'Comp', async () => (await import('../src/Comp')).default);
  row(F, 'barrel', async () => (await import('../src/barrel')).LessonPage);
  row(F, 'reexports', async () => (await import('../src/reexports')).default);
});
describe('createRequire', () => {
  row(F, 'require mexp-fn', () => req('mexp-fn'));
  row(F, 'require exp-default-flag', () => req('exp-default-flag'));
  row(F, 'require exports-map', () => req('exports-map'));
  row(F, 'require cjs-requires-esm', () => req('cjs-requires-esm'));
});
