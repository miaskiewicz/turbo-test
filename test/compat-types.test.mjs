// Type-level regression guard for the shipped `.d.ts` shim. The runner strips types, so the shim's
// correctness can't be asserted at run time — instead we compile types/typetests/*.test-d.ts (which
// exercises every widened API: it.each tuple/as-const/union rows, vi.mocked/hoisted/fn<T>, mock
// importOriginal<T>, expect.fail) with `tsc --noEmit --strict`. Any regression in
// types/turbo-test-api.d.ts becomes a tsc error and fails this test. Skips if typescript is absent.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);

function tscBin() {
  try {
    const pkg = require.resolve('typescript/package.json', { paths: [ROOT] });
    const bin = path.join(path.dirname(pkg), 'bin', 'tsc');
    return fs.existsSync(bin) ? bin : null;
  } catch {
    return null;
  }
}

test('shipped .d.ts shim type-checks the full vitest/jest API surface (tsc --strict)', (t) => {
  const tsc = tscBin();
  if (!tsc) {
    t.skip('typescript not installed — run `npm install` to enable the type-shim guard');
    return;
  }
  const files = fs
    .readdirSync(path.join(ROOT, 'types', 'typetests'))
    .filter((f) => f.endsWith('.test-d.ts'))
    .map((f) => path.join('types', 'typetests', f));
  assert.ok(files.length > 0, 'at least one typetest file exists');

  const res = spawnSync(
    process.execPath,
    [
      tsc,
      '--noEmit', '--strict', '--skipLibCheck',
      '--moduleResolution', 'node16', '--module', 'node16',
      '--target', 'es2022', '--lib', 'es2022,dom',
      ...files,
    ],
    { cwd: ROOT, encoding: 'utf8' },
  );
  assert.equal(res.status, 0, `tsc reported type errors in the shim guard:\n${res.stdout}\n${res.stderr}`);
});
