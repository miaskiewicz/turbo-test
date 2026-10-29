import { beforeAll, describe, expect, test } from 'vitest';
describe('hooked', () => {
  beforeAll(() => { throw new Error('hook boom'); });
  test('a', () => { expect(1).toBe(1); });
});
test('after', () => {});
