// Reuse-mode heap (turbo-test#124, from #79): a reused isolate only grows (every file's module
// graph, DOM and timer closures stay reachable, like vitest's `isolate: false`), and V8's answer to
// a full heap is to abort the whole process — #79 lost a 529-file run, with no per-file output, at
// 2.0 GB. vitest reports a worker that runs out of memory as a failed run, not a lost one.
//   - heap-stress: twelve files each keep ~100 MB reachable under a 700 MB cap. It only passes if
//     the worker replaces its isolate when the live heap passes TURBO_HEAP_RECYCLE_MB.
//   - heap-oom: one file allocates until the heap limit. The guard terminates it, that file is
//     reported as a failure, and the files around it still run.
// Fixtures and the original test: alphajew420, miaskiewicz/turbo-test#104 / #107.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');

function run(fixture, env) {
  const cache = fs.mkdtempSync(path.join(os.tmpdir(), 'tt-heap-cache-'));
  try {
    const res = spawnSync('node', [CLI, '--no-isolate', '--jobs', '1', '--reporter', 'json'], {
      cwd: path.join(ROOT, 'fixtures', fixture),
      encoding: 'utf8',
      maxBuffer: 64 << 20,
      env: { ...process.env, TURBO_CACHE_DIR: cache, TURBO_HEAP_LOG: '1', ...env },
    });
    const out = res.stdout || '';
    const err = res.stderr || '';
    const line = out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
    // file basename -> { status, message } (json reporter: "passed" / "failed")
    const files = {};
    if (line) {
      for (const f of JSON.parse(line).testResults) {
        files[path.basename(f.name)] = { status: f.status, message: f.message || '' };
      }
    }
    return { code: res.status, signal: res.signal, files, out, err };
  } finally {
    fs.rmSync(cache, { recursive: true, force: true });
  }
}

const statuses = (r) => Object.fromEntries(Object.entries(r.files).map(([k, v]) => [k, v.status]));

test('a reused isolate that outgrows its budget is replaced, not aborted', () => {
  const r = run('heap-stress', { TURBO_MAX_HEAP_MB: '700', TURBO_HEAP_RECYCLE_MB: '250' });
  assert.equal(r.signal, null, `the process died:\n${r.err.slice(-2000)}`);
  const expected = Object.fromEntries(
    Array.from({ length: 12 }, (_, i) => [`s${String(i + 1).padStart(2, '0')}.test.ts`, 'passed']),
  );
  assert.deepEqual(statuses(r), expected, r.err.slice(-2000));
  assert.equal(r.code, 0);
  // 12 x ~100 MB with a 250 MB recycle threshold: the isolate is replaced more than once.
  const recycles = r.err.match(/^\[heap\] .* recycled=true /gm) || [];
  assert.ok(recycles.length >= 2 && recycles.length <= 11, `recycles: ${recycles.length}\n${r.err.slice(-2000)}`);
});

test('a file that runs out of memory fails alone and the run continues', () => {
  const r = run('heap-oom', { TURBO_MAX_HEAP_MB: '256' });
  assert.equal(r.signal, null, `the process died:\n${r.err.slice(-2000)}`);
  assert.deepEqual(
    statuses(r),
    { 'a-ok.test.ts': 'passed', 'b-oom.test.ts': 'failed', 'c-ok.test.ts': 'passed', 'd-ok.test.ts': 'passed' },
    r.err.slice(-2000),
  );
  assert.equal(r.code, 1, 'the failed file must fail the run');
  assert.match(
    r.files['b-oom.test.ts'].message,
    /JavaScript heap out of memory \(limit \d+ MB\): the file was aborted and its isolate discarded/,
  );
});

test('a finished file\'s spy history is released, so spies do not grow a reused isolate', { todo: 'turbo-test#124' }, () => {
  // 12 files x ~60 MB of recorded call arguments under a 400 MB cap, and recycling disabled: only
  // releasing each finished file's call history keeps the isolate under the cap. Within a file
  // every spy still sees all of its calls (the fixtures assert mock.calls.length).
  const r = run('heap-spies', { TURBO_MAX_HEAP_MB: '400', TURBO_HEAP_RECYCLE_MB: '100000' });
  assert.equal(r.signal, null, `the process died:\n${r.err.slice(-2000)}`);
  const expected = Object.fromEntries(
    Array.from({ length: 12 }, (_, i) => [`p${String(i + 1).padStart(2, '0')}.test.ts`, 'passed']),
  );
  assert.deepEqual(statuses(r), expected, r.err.slice(-2000));
  assert.equal(r.code, 0);
  // One heap line per file, none recycled or out of memory (a file that ran out of heap would be
  // re-run on a fresh isolate and pass, hiding the growth), and the live heap after each file
  // holds that file's ~70 MB of history only: unreleased, it climbs by ~70 MB per file.
  assert.doesNotMatch(r.err, /heap out of memory/);
  const used = [...r.err.matchAll(/^\[heap\] .* used=(\d+)MB .* recycled=false /gm)].map((m) => Number(m[1]));
  assert.equal(used.length, 12, r.err.slice(-2000));
  assert.ok(Math.max(...used) < 120, `live heap after each file (MB): ${used.join(', ')}`);
});
