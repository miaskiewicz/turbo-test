// Reuse-mode retries (turbo-test#126, from #79): under isolate reuse (vitest `isolate: false`, or
// `--no-isolate`) every file that failed was run a second time on a fresh isolate, doubling the
// cost of each failing file. vitest runs every file once, on the shared worker, and reports that
// run. The fixture's b-fail.test.ts fails on any isolate in any order; each file logs every
// evaluation to the fixture's runs.log while it exists. The oracle is vitest 4.1.12 on the same
// fixture (tests/data/reuse-fail-once-vitest-4.1.12.json).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');
const ORACLE = JSON.parse(fs.readFileSync(path.join(ROOT, 'tests', 'data', 'reuse-fail-once-vitest-4.1.12.json'), 'utf8'));

function run(args) {
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'tt-reuse-retry-'));
  const cwd = path.join(ROOT, 'fixtures', 'reuse-fail-once'), log = path.join(cwd, 'runs.log');
  fs.writeFileSync(log, '');
  try {
    const res = spawnSync('node', [CLI, '--jobs', '1', '--reporter', 'json', ...args], {
      cwd,
      encoding: 'utf8',
      maxBuffer: 64 << 20,
      env: { ...process.env, TURBO_CACHE_DIR: path.join(scratch, 'cache') },
    });
    const line = (res.stdout || '').split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
    assert.ok(line, `no json report:\n${res.stdout}\n${res.stderr}`);
    const report = JSON.parse(line);
    const files = {};
    for (const f of report.testResults) {
      files[path.basename(f.name)] = { status: f.status, tests: f.assertionResults.map((a) => [a.title, a.status]) };
    }
    const runs = {};
    for (const name of fs.readFileSync(log, 'utf8').split('\n').filter(Boolean)) runs[name] = (runs[name] || 0) + 1;
    return { code: res.status, report, files, runs, err: res.stderr || '' };
  } finally {
    fs.rmSync(log, { force: true });
    fs.rmSync(scratch, { recursive: true, force: true });
  }
}

const sorted = (o) => Object.fromEntries(Object.keys(o).sort().map((k) => [k, o[k]]));

for (const [label, args] of [['isolate: false in the config', []], ['--no-isolate', ['--no-isolate']]]) {
  test(`a failing file runs once under isolate reuse and reports what vitest does (${label})`, { todo: 'turbo-test#126: fails until the fix' }, () => {
    const r = run(args);
    assert.deepEqual(r.runs, ORACLE.runs, `each file must be evaluated exactly once\n${r.err.slice(-2000)}`);
    assert.deepEqual(sorted(r.files), ORACLE.files);
    assert.equal(r.report.numTotalTests, ORACLE.numTotalTests);
    assert.equal(r.report.numPassedTests, ORACLE.numPassedTests);
    assert.equal(r.report.numFailedTests, ORACLE.numFailedTests);
    assert.equal(r.code, ORACLE.exitCode);
  });
}
