// Jest-compatibility tests. turbo-test is a drop-in for vitest, but the same runtime also backs
// jest projects: the `jest` global controller, jest.config setupFiles/<rootDir>, CJS-first
// resolution, AND `import { … } from '@jest/globals'` (jest's explicit-import surface, used when
// `injectGlobals: false`). Spawns the real launcher against fixtures/jest and asserts on the JSON
// reporter summary. Mirrors compat-api.test.mjs's spawn/parse style.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);

// The emitDecoratorMetadata→tsc routing needs the project's TypeScript resolvable from `dir`.
function hasTypescript(dir) {
  try {
    require.resolve('typescript/package.json', { paths: [dir] });
    return true;
  } catch {
    return false;
  }
}
const CLI = path.join(ROOT, 'cli.js');
const FIX = path.join(ROOT, 'fixtures', 'jest');

function run(args) {
  const res = spawnSync('node', [CLI, ...args], { cwd: FIX, encoding: 'utf8' });
  return { code: res.status, out: res.stdout || '', err: res.stderr || '' };
}
function parseJson(out) {
  const line = out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
  assert.ok(line, `no JSON summary in output:\n${out}`);
  return JSON.parse(line);
}

test('jest global shim: jest.fn/mock/spyOn + jest.config setupFiles (<rootDir>) all work', () => {
  const r = parseJson(run(['--reporter', 'json', 'src/jest-shim.spec.ts']).out);
  assert.equal(r.numFailedTests, 0, 'jest global shim suite passes');
  assert.equal(r.numPassedTests, 4);
});

test("@jest/globals named imports resolve from the runtime (not the real package)", () => {
  const r = parseJson(run(['--reporter', 'json', 'src/jest-globals-import.spec.ts']).out);
  assert.equal(r.numFailedTests, 0, 'imported describe/it/expect/jest behave like the globals');
  assert.equal(r.numPassedTests, 2);
});

test('emitDecoratorMetadata field typed as an imported interface loads (tsc-path routing)', (t) => {
  // A decorated field typed as an imported INTERFACE: oxc would emit
  // Reflect.metadata("design:type", <Iface>) referencing the type-only import as a value, which the
  // ESM linker then rejects ("does not provide an export named <Iface>"). Routing emitDecoratorMetadata
  // files through the project's TypeScript (emits `Object` for non-value types) loads them.
  const FIX = path.join(ROOT, 'fixtures', 'compat', 'decorator-metadata-type');
  // This routing REQUIRES the project's typescript (oxc alone can't tell a type-only import from a
  // value). When TS isn't installed, skip rather than fail — same convention as the type-shim guard.
  if (!hasTypescript(FIX)) {
    t.skip('typescript not installed — run `npm install` to enable the emitDecoratorMetadata→tsc routing');
    return;
  }
  const res = spawnSync('node', [CLI, '--reporter', 'json', 'decorator-metadata-type.spec.ts'], { cwd: FIX, encoding: 'utf8' });
  const line = (res.stdout || '').split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
  assert.ok(line, `no JSON summary:\n${res.stdout}\n${res.stderr}`);
  const j = JSON.parse(line);
  assert.equal(j.numFailedTests, 0);
  assert.ok(j.numPassedTests >= 1);
});
