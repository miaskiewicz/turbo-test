// turbo-test#101: vitest's default `sequence.hooks: 'stack'` — afterEach / afterAll run last
// registered first (a suite's own before its parent's), a function returned by beforeEach runs
// after the test's afterEach hooks and one returned by beforeAll after the suite's afterAll hooks
// (both last returned first).
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
const log: string[] = [];
const L = (s: string) => { log.push(s); console.log(s); };
describe('outer', () => {
  beforeAll(() => { log.push('outer ba1'); return () => { log.push('outer ba1 cleanup'); }; });
  beforeAll(() => { log.push('outer ba2'); return () => { log.push('outer ba2 cleanup'); }; });
  beforeEach(() => { L('outer be1'); return () => { L('outer be1 cleanup'); }; });
  beforeEach(() => { L('outer be2'); return () => { L('outer be2 cleanup'); }; });
  beforeEach(() => { L('outer be3 (no cleanup)'); return 42; });
  afterEach(() => { L('outer ae1'); });
  afterEach(() => { L('outer ae2'); });
  afterAll(() => { log.push('outer aa1'); });
  afterAll(() => { log.push('outer aa2'); });
  describe('inner', () => {
    beforeAll(() => { log.push('inner ba'); return () => { log.push('inner ba cleanup'); }; });
    beforeEach(() => { L('inner be'); return async () => { await null; L('inner be cleanup'); }; });
    afterEach(() => { L('inner ae1'); });
    afterEach(() => { L('inner ae2'); });
    afterAll(() => { log.push('inner aa1'); });
    afterAll(() => { log.push('inner aa2'); });
    it('t1', () => { L('t1'); });
  });
  it('t2', () => { L('t2'); });
});
it('ran in vitest order', () => {
  expect(log).toEqual([
    'outer ba1', 'outer ba2',
    'inner ba',
    'outer be1', 'outer be2', 'outer be3 (no cleanup)', 'inner be', 't1',
    'inner ae2', 'inner ae1', 'outer ae2', 'outer ae1',
    'inner be cleanup', 'outer be2 cleanup', 'outer be1 cleanup',
    'inner aa2', 'inner aa1', 'inner ba cleanup',
    'outer be1', 'outer be2', 'outer be3 (no cleanup)', 't2',
    'outer ae2', 'outer ae1', 'outer be2 cleanup', 'outer be1 cleanup',
    'outer aa2', 'outer aa1', 'outer ba2 cleanup', 'outer ba1 cleanup',
  ]);
});
