import { defineConfig } from 'vitest/config';
import path from 'node:path';

// Exercises resolve.alias (object form, path.resolve values) AND test.alias (array form,
// find/replacement). turbo-test resolves these natively — no vite-tsconfig-paths / plugin needed.
export default defineConfig({
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      '~lib': path.resolve(__dirname, './lib'),
    },
  },
  test: {
    globals: true,
    alias: [
      { find: '@math', replacement: path.resolve(__dirname, './lib/math.ts') },
    ],
  },
});
