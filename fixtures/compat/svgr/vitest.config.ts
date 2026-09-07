import { defineConfig } from 'vitest/config';
import path from 'node:path';

// The `?react` svgr stub imports `react`; alias it to a local shim so this fixture is self-contained
// (no installed react needed) and still exercises real bare-import resolution through the stub.
export default defineConfig({
  resolve: {
    alias: {
      react: path.resolve(__dirname, './react-shim.ts'),
    },
  },
  test: { globals: true },
});
