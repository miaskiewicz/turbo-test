import { describe, expect, it } from 'vitest';
import { blockFields, plain, reexported } from '@fx/pkg/sub';

// A workspace package (symlinked, real path outside node_modules) whose `exports` subpath points
// at CommonJS emitted by tsc: named ESM imports must see exports.x = ... and
// Object.defineProperty(exports, "x", { get }) like Node's cjs-module-lexer does.
describe('named imports from a CJS exports subpath', () => {
  it('sees exports.x assignments', () => {
    expect(blockFields).toEqual(['title', 'body']);
    expect(plain).toBe(42);
  });
  it('sees defineProperty getters', () => {
    expect(reexported).toBe('from-helper');
  });
});

describe('setup file named import', () => {
  it('ran', () => {
    expect((globalThis as any).__setupBlockFields).toEqual(['title', 'body']);
  });
});
