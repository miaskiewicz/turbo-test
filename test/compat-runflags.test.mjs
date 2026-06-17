// Execution-control flag compat tests: --testTimeout / per-test { timeout }, --retry,
// --bail, --maxWorkers/--minWorkers, --silent, --allowOnly/--no-allowOnly.
// Spawns the real launcher (cli.js) the way CI would, and asserts on exit code +
// reporter JSON / stdout. Run via `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');
const C = (...p) => path.join(ROOT, 'fixtures', 'compat', ...p);
const BAIL_FILES = ['a', 'b', 'c'].map((n) => C('bail', `${n}.test.ts`));

function run(args, opts = {}) {
  const res = spawnSync('node', [CLI, ...args], { cwd: ROOT, encoding: 'utf8', ...opts });
  return { code: res.status, out: res.stdout || '', err: res.stderr || '' };
}

function parseJson(out) {
  const line = out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
  assert.ok(line, `no JSON summary in output:\n${out}`);
  return JSON.parse(line);
}

// ---- --testTimeout / per-test { timeout } ----

test('--testTimeout enforces a global timeout on a hung test', () => {
  const { code, out } = run(['--testTimeout', '100', '--reporter', 'json', C('timeout.test.ts')]);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 1, 'fast test passes');
  assert.equal(j.numFailedTests, 1, 'hung test times out -> fails (not hangs the worker)');
  assert.notEqual(code, 0);
});

test('per-test { timeout } (and numeric arg) enforce a timeout', () => {
  const { code, out } = run(['--reporter', 'json', C('per-test-timeout.test.ts')]);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 1, 'the fast test passes');
  assert.equal(j.numFailedTests, 2, 'both hung tests with tiny per-test timeouts fail');
  assert.notEqual(code, 0);
});

// ---- --retry ----

test('--retry default lets a flaky test pass after retries', () => {
  // without retry: 1 failure; with --retry 3: the 3rd attempt passes.
  const fail = parseJson(run(['--reporter', 'json', C('retry.test.ts')]).out);
  assert.equal(fail.numFailedTests, 1, 'no retry -> fails on first attempt');

  const { code, out } = run(['--retry', '3', '--reporter', 'json', C('retry.test.ts')]);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 1, '--retry 3 -> passes on 3rd attempt');
  assert.equal(j.numFailedTests, 0);
  assert.equal(code, 0);
});

// ---- --bail ----

test('--bail stops the run after N failed tests (partial results)', () => {
  // 3 files x 2 failing tests each = 6 potential failures. --bail 1, single worker:
  // after the first file finishes the cumulative failures >= 1 so no more files are pulled.
  const { code, out } = run(['--bail', '1', '--maxWorkers', '1', '--reporter', 'json', ...BAIL_FILES]);
  const j = parseJson(out);
  assert.notEqual(code, 0, 'failures -> non-zero exit');
  assert.ok(j.numFailedTests >= 1, 'at least one failure reported');
  assert.ok(j.numFailedTests < 6, `bail should stop early, saw ${j.numFailedTests} of 6`);
});

test('without --bail the whole run executes (all 6 failures)', () => {
  const { out } = run(['--maxWorkers', '1', '--reporter', 'json', ...BAIL_FILES]);
  const j = parseJson(out);
  assert.equal(j.numFailedTests, 6, 'all failing tests run when no bail');
});

// ---- --maxWorkers / --minWorkers ----

test('--maxWorkers is accepted (alias of --jobs) and runs normally', () => {
  const { code, out } = run(['--maxWorkers', '2', '--reporter', 'json', C('names.test.ts')]);
  assert.equal(code, 0, out);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 3);
});

test('--minWorkers is accepted (no-op) and does not flip exit code', () => {
  const { code, out } = run(['--minWorkers', '1', '--reporter', 'json', C('names.test.ts')]);
  assert.equal(code, 0, out);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 3);
});

// ---- --silent ----

test('--silent suppresses test console.* output', () => {
  const noisy = run(['--reporter', 'json', C('silent', 'console.test.ts')]);
  assert.ok(/TURBO_SILENT_MARKER_LOG/.test(noisy.out), 'baseline prints console markers');

  const { code, out } = run(['--silent', '--reporter', 'json', C('silent', 'console.test.ts')]);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 1, 'test still runs/passes under --silent');
  assert.equal(code, 0);
  assert.ok(!/TURBO_SILENT_MARKER/.test(out), `console output leaked under --silent:\n${out}`);
});

// ---- --allowOnly / --no-allowOnly ----

test('.only is allowed by default (--allowOnly)', () => {
  const { code, out } = run(['--allowOnly', '--reporter', 'json', C('only', 'only.test.ts')]);
  assert.equal(code, 0, out);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 1, 'only the .only test runs');
});

test('--no-allowOnly fails the run when a stray .only exists', () => {
  const { code, err, out } = run(['--no-allowOnly', '--reporter', 'json', C('only', 'only.test.ts')]);
  assert.notEqual(code, 0, 'stray .only under --no-allowOnly must fail');
  assert.ok(/only/i.test(err + out), 'a clear message mentions .only');
});
