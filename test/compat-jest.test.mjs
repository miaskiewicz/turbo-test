// Jest-compatibility tests. turbo-test is a drop-in for vitest, but the same runtime also backs
// jest projects: the `jest` global controller, jest.config setupFiles/<rootDir>, CJS-first
// resolution, AND `import { … } from '@jest/globals'` (jest's explicit-import surface, used when
// `injectGlobals: false`). Spawns the real launcher against fixtures/jest and asserts on the JSON
// reporter summary. Mirrors compat-api.test.mjs's spawn/parse style.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
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
