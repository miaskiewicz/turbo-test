import { test, expect, vi } from 'vitest';
vi.mock('./auto.js');
vi.mock('./factory.js', async () => {
  const stubbed = await vi.importMock('./value.js');
  const actual = await vi.importActual('./value.js');
  return { ...stubbed, real: actual.run };
});
test('unregistered importMock deeply mocks functions and classes, retaining values', async () => {
  const m = await vi.importMock('./value.js');
  expect(vi.isMockFunction(m.run)).toBe(true);
  expect(m.run()).toBeUndefined();
  expect(m.count).toBe(3);
  expect(m.list).toEqual([]);
  expect(vi.isMockFunction(m.nested.run)).toBe(true);
  expect(vi.isMockFunction(new m.Thing().method)).toBe(true);
  const actual = await vi.importActual('./value.js');
  expect(actual.run()).toBe('real');
  expect(actual.list).toEqual([1]);
});
test('importMock loads an existing colocated manual mock', async () => {
  expect((await vi.importMock('./manual.js')).run()).toBe('manual');
});
test('importMock respects a registered async factory without recursively resolving itself', async () => {
  const m = await vi.importMock('./factory.js');
  expect(vi.isMockFunction(m.run)).toBe(true);
  expect(m.run()).toBeUndefined();
  expect(m.real()).toBe('real');
});
test('missing modules reject instead of returning an empty fake module', async () => {
  await expect(vi.importMock('./missing.js')).rejects.toThrow();
});
test('module evaluation errors reject rather than passing with empty exports', async () => {
  await expect(vi.importMock('./throws.js')).rejects.toThrow('import-mock evaluation failure');
});

test('registered automocks create independent functions and call histories', async () => {
  const first = await vi.importMock('./auto.js');
  const second = await vi.importMock('./auto.js');
  expect(first.run).not.toBe(second.run);
  first.run('one call');
  expect(first.run).toHaveBeenCalledTimes(1);
  expect(second.run).not.toHaveBeenCalled();
});
test('registered factory functions retain their mock identity', async () => {
  const first = await vi.importMock('./factory.js');
  const second = await vi.importMock('./factory.js');
  expect(first.run).toBe(second.run);
});
