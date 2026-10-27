import { describe, vi } from 'vitest';
import { row } from './d';
import MexpFn from 'mexp-fn';
import DefFlag, { named as defFlagNamed } from 'exp-default-flag';
import MexpObj, { Comp as MexpObjComp } from 'mexp-obj';
import DefNoFlag from 'exp-default-noflag';
import TscComp from 'tsc-out';

vi.mock('mexp-fn', () => ({ default: function Mocked() {} }));
vi.mock('exp-default-flag', async (importOriginal) => ({ ...(await importOriginal<object>()), named: 'mocked' }));
vi.mock('mexp-obj');
vi.mock('exp-default-noflag', () => ({ named: 'only' }));
vi.mock('tsc-out', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('tsc-out');
  return { ...actual, named: 'mocked' };
});

const F = 'mock';
describe('vi.mock', () => {
  row(F, 'factory default', () => MexpFn);
  row(F, 'importOriginal spread default', () => DefFlag);
  row(F, 'importOriginal spread named', () => defFlagNamed);
  row(F, 'automock default', () => MexpObj);
  row(F, 'automock named', () => MexpObjComp);
  row(F, 'factory without default', () => DefNoFlag);
  row(F, 'importActual spread default', () => TscComp);
});
describe('vi.importActual', () => {
  row(F, 'mexp-fn', () => vi.importActual('mexp-fn'));
  row(F, 'exp-default-flag', () => vi.importActual('exp-default-flag'));
  row(F, 'exp-default-noflag', () => vi.importActual('exp-default-noflag'));
  row(F, 'tsc-out', () => vi.importActual('tsc-out'));
  row(F, 'Comp', () => vi.importActual('../src/Comp'));
});
