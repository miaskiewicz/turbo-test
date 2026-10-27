// turbo-test#101: a throwing afterEach (or beforeEach cleanup) fails the test, stops the hooks
// after it and keeps the test body's own error; a throwing beforeEach skips the body and the
// cleanups of the beforeEach hooks before it, while the afterEach hooks still run.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
describe('passing body', () => {
  afterEach(() => { throw new Error('ae boom'); });
  it('fails with the afterEach error', () => {});
});
describe('failing body', () => {
  beforeEach(() => () => { console.log('cleanup runs?'); });
  afterEach(() => { throw new Error('ae2 boom'); });
  afterEach(() => { console.log('ae1 runs first'); });
  it('keeps both errors', () => { expect(1).toBe(2); });
});
describe('it.fails', () => {
  afterEach(() => { throw new Error('ae3 boom'); });
  it.fails('passing body', () => {});
  it.fails('throwing body', () => { throw new Error('body'); });
});
describe('cleanup throws', () => {
  beforeEach(() => () => { console.log('outer cleanup runs?'); });
  beforeEach(() => () => { throw new Error('cleanup boom'); });
  it('fails with the cleanup error', () => {});
});
describe('beforeEach throws', () => {
  beforeEach(() => () => { console.log('cleanup after beforeEach throw?'); });
  beforeEach(() => { throw new Error('be boom'); });
  afterEach(() => { console.log('afterEach after beforeEach throw'); });
  it('skips the body', () => { console.log('body runs?'); });
});
describe('only the throwing test', () => {
  let n = 0;
  afterEach(() => { n++; if (n === 1) throw new Error('first only'); });
  it('one', () => {});
  it('two', () => {});
});
describe('retry', () => {
  let n = 0;
  afterEach(() => { n++; if (n === 1) throw new Error('retry ae'); });
  it('passes on the retry', { retry: 1 }, () => {});
});
