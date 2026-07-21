import { defineConfig } from 'vitest/config';
export default defineConfig({
  test: {
    environment: 'node',
    isolate: false,
    setupFiles: ['./setup.ts'],
    coverage: {
      include: ['src/**/*.ts'],
    },
  },
});
