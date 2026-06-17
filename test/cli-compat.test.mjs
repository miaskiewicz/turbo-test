// CLI-level vitest-compatibility tests. Spawns the real launcher (cli.js) the way a user /
// CI would, and asserts on exit code + reporter output. Run via `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');
const NAMES = path.join(ROOT, 'fixtures', 'compat', 'names.test.ts');
const EMPTY = path.join(ROOT, 'fixtures', 'compat', 'empty');

function run(args, opts = {}) {
  const res = spawnSync('node', [CLI, ...args], { cwd: ROOT, encoding: 'utf8', ...opts });
  return { code: res.status, out: res.stdout || '', err: res.stderr || '' };
}

// Pull the JSON summary line out of mixed stdout (the text PASS line is also printed).
function parseJson(out) {
  const line = out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
  assert.ok(line, `no JSON summary in output:\n${out}`);
  return JSON.parse(line);
}

test('baseline: runs all tests in the fixture', () => {
  const { code, out } = run(['--reporter', 'json', NAMES]);
  const j = parseJson(out);
  assert.equal(code, 0);
  assert.equal(j.numPassedTests, 3);
  assert.equal(j.numFailedTests, 0);
});

test('-t / --testNamePattern filters by test name (regex)', () => {
  const j = parseJson(run(['--reporter', 'json', '-t', 'adds', NAMES]).out);
  assert.equal(j.numPassedTests, 1, 'only "adds numbers" should run');
  assert.equal(j.numFailedTests, 0);

  const j2 = parseJson(run(['--reporter', 'json', '--testNamePattern', 'group', NAMES]).out);
  assert.equal(j2.numPassedTests, 3, 'pattern matches the describe prefix on all 3');
});

test('-t pattern matches against the full describe>it name', () => {
  const j = parseJson(run(['--reporter', 'json', '-t', 'beta group', NAMES]).out);
  assert.equal(j.numPassedTests, 1, 'only the beta-group test');
});

test('`run` subcommand is accepted as a no-op (canonical `vitest run`)', () => {
  const { code, out, err } = run(['run', '--reporter', 'json', NAMES]);
  assert.equal(code, 0, `stderr:\n${err}`);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 3);
  assert.ok(!/skipping .*missing/.test(err), `\`run\` leaked as a phantom file: ${err}`);
});

test('unknown --flags are ignored, not treated as test files', () => {
  const { code, out, err } = run(['--silent', '--pool=forks', '--reporter', 'json', NAMES]);
  assert.equal(code, 0, `unknown flags flipped exit code. stderr:\n${err}`);
  const j = parseJson(out);
  assert.equal(j.numPassedTests, 3);
});

test('--passWithNoTests exits 0 when no test files are found', () => {
  const fail = run([], { cwd: EMPTY });
  assert.notEqual(fail.code, 0, 'no-tests should fail without the flag');

  const pass = run(['--passWithNoTests'], { cwd: EMPTY });
  assert.equal(pass.code, 0, `--passWithNoTests should exit 0. stderr:\n${pass.err}`);
});
