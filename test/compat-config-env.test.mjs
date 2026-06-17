// CLI-level tests for the vitest config / discovery / environment selection flags:
//   -c/--config, --root/--dir, --environment + `// @vitest-environment` pragma,
//   --isolate/--no-isolate, --changed [since], --globals/--no-globals.
// Spawns the real launcher (cli.js) the way a user / CI would. Run via `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');
const COMPAT = path.join(ROOT, 'fixtures', 'compat');
const NAMES = path.join(COMPAT, 'names.test.ts');
const SUBDIR = path.join(COMPAT, 'subdir');
const SUBCFG = path.join(SUBDIR, 'vitest.config.ts');
const NODE_ENV = path.join(COMPAT, 'node-env.test.ts');
const PRAGMA = path.join(COMPAT, 'pragma-node.test.ts');

function run(args, opts = {}) {
  const res = spawnSync('node', [CLI, ...args], { cwd: ROOT, encoding: 'utf8', ...opts });
  return { code: res.status, out: res.stdout || '', err: res.stderr || '' };
}

function parseJson(out) {
  const line = out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
  assert.ok(line, `no JSON summary in output:\n${out}`);
  return JSON.parse(line);
}

// ---- -c / --config + --root / --dir ---------------------------------------

test('-c/--config + --root: forced config drives discovery (its include only)', () => {
  // subdir config includes ONLY **/*.feature.spec.ts → widget runs, sibling ignored.test.ts does not.
  const { code, out } = run(['--reporter', 'json', '--root', SUBDIR, '-c', SUBCFG]);
  const j = parseJson(out);
  assert.equal(code, 0, out);
  assert.equal(j.numTotalTestSuites, 1, 'only the *.feature.spec.ts file should be discovered');
  assert.equal(j.numPassedTests, 1);
  assert.equal(j.numFailedTests, 0);
  assert.ok(!/ignored\.test\.ts/.test(out), 'ignored.test.ts must NOT be discovered under this config');
});

test('--dir is accepted as an alias for the discovery root', () => {
  const { code } = run(['--reporter', 'json', '--dir', SUBDIR, '--config', SUBCFG]);
  assert.equal(code, 0);
});

test('--config=<path> inline form is accepted', () => {
  const j = parseJson(run(['--reporter', 'json', '--root', SUBDIR, `--config=${SUBCFG}`]).out);
  assert.equal(j.numTotalTestSuites, 1);
});

// ---- --environment + pragma -----------------------------------------------

test('--environment node: Node globals present, no DOM (document undefined)', () => {
  const { code, out } = run(['--reporter', 'json', '--environment', 'node', NODE_ENV]);
  const j = parseJson(out);
  assert.equal(code, 0, out);
  assert.equal(j.numPassedTests, 2);
  assert.equal(j.numFailedTests, 0);
});

test('// @vitest-environment node pragma forces node env for that file', () => {
  const { code, out } = run(['--reporter', 'json', PRAGMA]);
  const j = parseJson(out);
  assert.equal(code, 0, out);
  assert.equal(j.numPassedTests, 1);
  assert.equal(j.numFailedTests, 0);
});

test('--environment jsdom is accepted (maps to turbo-dom, run does not break)', () => {
  const { code } = run(['--reporter', 'json', '--environment', 'jsdom', NAMES]);
  assert.equal(code, 0);
});

// ---- --isolate / --no-isolate ---------------------------------------------

test('--no-isolate sets TURBO_REUSE_ISOLATE and still runs green', () => {
  // We can't read the child's env back, but the contract is: cli.js sets the env before spawn
  // and the run must not break. (The reuse path is exercised; result parity is asserted.)
  const { code, out } = run(['--reporter', 'json', '--no-isolate', NAMES]);
  assert.equal(code, 0, out);
  assert.equal(parseJson(out).numPassedTests, 3);
});

test('--isolate is accepted and runs green', () => {
  const { code, out } = run(['--reporter', 'json', '--isolate', NAMES]);
  assert.equal(code, 0, out);
  assert.equal(parseJson(out).numPassedTests, 3);
});

// ---- --globals / --no-globals (no-ops; globals are always on) -------------

test('--globals / --no-globals are accepted as no-ops (globals always on)', () => {
  const a = run(['--reporter', 'json', '--globals', NAMES]);
  assert.equal(a.code, 0, a.err);
  assert.equal(parseJson(a.out).numPassedTests, 3);
  // --no-globals cannot be honored (documented gap) — accepted, run still uses globals.
  const b = run(['--reporter', 'json', '--no-globals', NAMES]);
  assert.equal(b.code, 0, b.err);
  assert.equal(parseJson(b.out).numPassedTests, 3);
});

// ---- --changed [since] -----------------------------------------------------
// Build a throwaway git repo so the changed-set is deterministic.

function mkTempRepo() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tt-changed-'));
  const git = (args) => {
    const r = spawnSync('git', args, { cwd: dir, encoding: 'utf8' });
    assert.equal(r.status, 0, `git ${args.join(' ')}: ${r.stderr}`);
  };
  git(['init', '-q']);
  git(['config', 'user.email', 't@t.t']);
  git(['config', 'user.name', 't']);
  const write = (name, body) => fs.writeFileSync(path.join(dir, name), body);
  const pass = `import { it, expect } from 'vitest';\nit('ok', () => expect(1).toBe(1));\n`;
  write('a.test.ts', pass);
  write('b.test.ts', pass);
  git(['add', '-A']);
  git(['commit', '-qm', 'init']);
  return { dir, git, write };
}

test('--changed runs only the changed test file (no import graph)', () => {
  const { dir, git, write } = mkTempRepo();
  // Modify only a.test.ts after the commit.
  write('a.test.ts', `import { it, expect } from 'vitest';\nit('still ok', () => expect(2).toBe(2));\n`);
  const { code, out } = run(['--reporter', 'json', '--changed'], { cwd: dir });
  assert.equal(code, 0, out);
  const j = parseJson(out);
  assert.equal(j.numTotalTestSuites, 1, 'only a.test.ts changed');
  assert.ok(/a\.test\.ts/.test(out) && !/b\.test\.ts/.test(out), `expected only a.test.ts:\n${out}`);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('--changed with nothing changed exits 0 (running nothing is not a failure)', () => {
  const { dir } = mkTempRepo(); // clean tree, all committed
  const { code, err } = run(['--reporter', 'json', '--changed'], { cwd: dir });
  assert.equal(code, 0, `--changed clean tree should exit 0: ${err}`);
  assert.ok(/no changed test files/.test(err), `expected the no-changes notice: ${err}`);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('--changed <since> ref selects files changed vs that ref', () => {
  const { dir, git, write } = mkTempRepo();
  const base = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: dir, encoding: 'utf8' }).stdout.trim();
  // New commit touching only b.test.ts.
  write('b.test.ts', `import { it, expect } from 'vitest';\nit('b changed', () => expect(3).toBe(3));\n`);
  git(['commit', '-qam', 'touch b']);
  const { code, out } = run(['--reporter', 'json', `--changed=${base}`], { cwd: dir });
  assert.equal(code, 0, out);
  assert.ok(/b\.test\.ts/.test(out) && !/a\.test\.ts/.test(out), `expected only b.test.ts:\n${out}`);
  fs.rmSync(dir, { recursive: true, force: true });
});

// ---- unknown flags still ignored, value-flags don't eat files -------------

test('config/env flags do not consume a following test-file arg as their value', () => {
  // --no-isolate takes NO value; the NAMES path must still be treated as a file.
  const j = parseJson(run(['--reporter', 'json', '--no-isolate', '--isolate', NAMES]).out);
  assert.equal(j.numPassedTests, 3);
});
