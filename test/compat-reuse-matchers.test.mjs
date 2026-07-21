// Regression: setupFiles' custom matchers (jest-dom's `expect.extend`) must be available to
// EVERY test file under the isolate-reuse path — never lost for a whole worker's chunk.
//
// The `jestdom-reuse` fixture mirrors the payroll-app shape that surfaced the bug: a vendored
// package (node_modules/jest-dom-ish) whose top-level side-effect is `expect.extend({...})`, a
// setupFile that imports it, `isolate: false` (reuse), and many test files that each
// `import { expect } from 'vitest'` and call the custom matcher.
//
// Root cause (fixed): `esbuild_bundle_full` wrote the setup bundle to the shared content-addressed
// cache via `--outfile` NON-atomically. When many workers raced to create the same bundle at
// startup, a worker could read a half-written/empty file — an empty setup bundle imports nothing,
// so `expect.extend` never ran and that whole worker lost the matchers for every file it owned
// (`expect(...).toBeCustom is not a function`, jest-dom's `toBeInTheDocument` in the real app).
// The bundle write now goes through a temp file + atomic rename, like every other esbuild cache
// path. Running the fixture under reuse a few times (clean cache each time to force the concurrent
// first-build) must be all-green with every test present.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');
const FIX = path.join(ROOT, 'fixtures', 'jestdom-reuse');
const EXPECTED_TESTS = 60; // 30 files x 2 matcher assertions

function runReuse() {
  // Isolated, EMPTY cache dir per run so every run re-creates the setup bundle concurrently across
  // workers — the exact window the non-atomic write raced in. Isolated (not the shared temp cache)
  // so this test can start clean without disturbing other concurrent turbo-test processes.
  const cache = fs.mkdtempSync(path.join(os.tmpdir(), 'tt-reuse-cache-'));
  try {
    const res = spawnSync('node', [CLI, '--reporter', 'json', '--no-isolate'], {
      cwd: FIX,
      encoding: 'utf8',
      env: { ...process.env, TURBO_CACHE_DIR: cache },
    });
    const out = res.stdout || '';
    const line = out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
    assert.ok(line, `no JSON summary:\n${out}\n${res.stderr || ''}`);
    return { code: res.status, json: JSON.parse(line), out, err: res.stderr || '' };
  } finally {
    fs.rmSync(cache, { recursive: true, force: true });
  }
}

test('setupFiles expect.extend matchers reach every file under reuse (no whole-worker matcher loss)', () => {
  for (let i = 0; i < 4; i++) {
    const { code, json, out } = runReuse();
    assert.equal(json.numFailedTests, 0, `run ${i}: no failures expected\n${out}`);
    assert.equal(
      json.numPassedTests,
      EXPECTED_TESTS,
      `run ${i}: all ${EXPECTED_TESTS} matcher assertions must run + pass (a broken worker drops a whole chunk)\n${out}`,
    );
    assert.ok(!/is not a function/.test(out), `run ${i}: no missing-matcher error\n${out}`);
    assert.equal(code, 0, `run ${i}: green exit`);
  }
});
