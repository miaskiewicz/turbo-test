import { describe, it, expect } from 'vitest';
import { readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// fixtures/compat/readdir-tree: a.txt, sub/b.txt
const url = new URL('./readdir-tree', import.meta.url);
const dir = fileURLToPath(url);

describe('node:fs readdirSync', () => {
  it('resolves a relative URL against import.meta.url', () => {
    expect(dir.endsWith('/fixtures/compat/readdir-tree') || dir.endsWith('\\fixtures\\compat\\readdir-tree')).toBe(true);
  });

  it('lists names for a string path', () => {
    expect(readdirSync(dir)).toEqual(['a.txt', 'sub']);
  });

  it('accepts a URL object', () => {
    expect(readdirSync(url)).toEqual(['a.txt', 'sub']);
  });

  it('withFileTypes returns Dirents', () => {
    const [file, sub] = readdirSync(dir, { withFileTypes: true });
    expect(file.name).toBe('a.txt');
    expect(file.isFile()).toBe(true);
    expect(file.isDirectory()).toBe(false);
    expect(file.parentPath).toBe(dir);
    expect(sub.isDirectory()).toBe(true);
    expect(sub.isSymbolicLink()).toBe(false);
  });

  it('recursive lists nested entries relative to the root', () => {
    expect(readdirSync(dir, { recursive: true })).toEqual(['a.txt', 'sub', 'sub/b.txt']);
  });

  it('throws ENOENT for a missing directory', () => {
    expect(() => readdirSync(dir + '/nope')).toThrow(/ENOENT/);
  });

  it('fs.promises.readdir works', async () => {
    const fs = await import('node:fs');
    expect(await fs.promises.readdir(dir)).toEqual(['a.txt', 'sub']);
  });
});
