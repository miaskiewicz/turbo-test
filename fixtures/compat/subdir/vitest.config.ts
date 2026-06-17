import { defineConfig } from 'vitest/config';

// Includes ONLY *.feature.spec.ts (not the default *.test.* / *.spec.*) so a test that this
// config's `include` is honored can distinguish it from default discovery. The sibling
// `ignored.test.ts` must NOT be picked up when this config drives discovery.
export default defineConfig({
  test: {
    include: ['**/*.feature.spec.ts'],
    exclude: ['**/node_modules/**'],
    environment: 'node',
  },
});
