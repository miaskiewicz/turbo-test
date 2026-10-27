// A reused isolate only grows (every file's module graph, DOM and timer closures stay reachable,
// like vitest's `isolate: false`), and V8's answer to a full heap is to abort the whole process —
// miaskiewicz/turbo-test#79 lost a 529-file run, with no per-file output, at 2.0 GB.
//   - heap-stress: twelve files each keep ~100 MB reachable under a 700 MB cap. It only passes if
//     the worker replaces its isolate when the live heap passes TURBO_HEAP_RECYCLE_MB.
//   - heap-oom: one file allocates until the heap limit. The guard terminates it, that file is
//     reported as an error, and the files around it still run.
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
    const res = spawnSync('node', [CLI, '--no-isolate', '--jobs', '1'], {
      cwd: path.join(ROOT, 'fixtures', fixture),
      encoding: 'utf8',
      env: { ...process.env, TURBO_CACHE_DIR: cache, TURBO_HEAP_LOG: '1', ...env },
    });
    const out = res.stdout || '';
    // "PASS  /path/file.test.ts  (1 passed, 0 failed)" / "ERROR /path/file.test.ts  (reason)"
    const files = {};
    for (const m of out.matchAll(/^(PASS|FAIL|ERROR)\s+(\S+)/gm)) files[path.basename(m[2])] = m[1];
    return { code: res.status, signal: res.signal, files, out, err: res.stderr || '' };
  } finally {
    fs.rmSync(cache, { recursive: true, force: true });
  }
}

test('a reused isolate that outgrows its budget is replaced, not aborted', () => {
  const r = run('heap-stress', { TURBO_MAX_HEAP_MB: '700', TURBO_HEAP_RECYCLE_MB: '250' });
  assert.equal(r.signal, null, `the process died:\n${r.err.slice(-2000)}`);
  assert.equal(Object.values(r.files).filter((v) => v === 'PASS').length, 12, `${r.out}\n${r.err.slice(-2000)}`);
  assert.equal(r.code, 0);
  assert.match(r.err, /recycled=true/, 'the isolate was never recycled');
});

test('a file that runs out of memory fails alone and the run continues', () => {
  const r = run('heap-oom', { TURBO_MAX_HEAP_MB: '256' });
  assert.equal(r.signal, null, `the process died:\n${r.err.slice(-2000)}`);
  assert.equal(r.files['b-oom.test.ts'], 'ERROR', `${r.out}\n${r.err.slice(-2000)}`);
  for (const ok of ['a-ok.test.ts', 'c-ok.test.ts', 'd-ok.test.ts']) {
    assert.equal(r.files[ok], 'PASS', `${ok}: ${r.out}`);
  }
  assert.notEqual(r.code, 0, 'the failed file must fail the run');
  assert.match(r.out, /JavaScript heap out of memory/);
});

test('the call history of a finished file is released, so spies do not grow a reused isolate', () => {
  // 12 files x ~60 MB of recorded call arguments under a 400 MB cap, and recycling disabled: only
  // dropping each file's history keeps the isolate under the cap.
  const r = run('heap-spies', { TURBO_MAX_HEAP_MB: '400', TURBO_HEAP_RECYCLE_MB: '100000' });
  assert.equal(r.signal, null, `the process died:\n${r.err.slice(-2000)}`);
  assert.equal(Object.values(r.files).filter((v) => v === 'PASS').length, 12, `${r.out}\n${r.err.slice(-2000)}`);
  assert.equal(r.code, 0);
});
