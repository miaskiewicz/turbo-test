// turbo-test#101: a throwing afterAll fails its suite and stops the afterAll hooks after it and
// the beforeAll cleanups; a throwing beforeAll drops the cleanups of the beforeAll hooks before
// it (the afterAll hooks still run); a throwing beforeAll cleanup fails the suite.
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
const log: string[] = [];
describe('afterAll throws', () => {
  beforeAll(() => () => { log.push('ba cleanup'); });
  afterAll(() => { log.push('aa1'); });
  afterAll(() => { throw new Error('aa2 boom'); });
  it('t', () => {});
});
describe('beforeAll throws', () => {
  beforeAll(() => () => { log.push('ba1 cleanup'); });
  beforeAll(() => { throw new Error('ba2 boom'); });
  afterAll(() => { log.push('aa after ba throw'); });
  it('t', () => {});
});
describe('beforeAll cleanup throws', () => {
  beforeAll(() => () => { throw new Error('ba cleanup boom'); });
  afterAll(() => { log.push('aa before cleanup'); });
  it('t', () => {});
});
it('ran what vitest runs', () => {
  expect(log).toEqual(['aa after ba throw', 'aa before cleanup']);
});
