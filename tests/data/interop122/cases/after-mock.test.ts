// Runs after mock.test.ts in one worker under isolate: false: the modules mocked there must
// be the real ones here.
import { describe } from 'vitest';
import { row } from './d';
import MexpFn from 'mexp-fn';
import DefFlag, { named as defFlagNamed } from 'exp-default-flag';
import TscComp from 'tsc-out';

const F = 'after-mock';
describe('unmocked', () => {
  row(F, 'mexp-fn default', () => MexpFn);
  row(F, 'exp-default-flag default', () => DefFlag);
  row(F, 'exp-default-flag named', () => defFlagNamed);
  row(F, 'tsc default', () => TscComp);
});
