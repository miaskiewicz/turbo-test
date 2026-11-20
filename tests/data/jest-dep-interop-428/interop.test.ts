import { callGet } from 'pkg';

test('a jest-transformed ESM package reads __esModule CJS .default as ts-jest does', () => {
  expect(callGet({ a: 1 }, 'a')).toBe(1);
});
