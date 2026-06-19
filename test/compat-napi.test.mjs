// Native-addon (.node / N-API) crash-hardening tests.
//
// Regression guard for the SIGSEGV-on-native-addon bug: a spec that require()s a
// native addon which calls an N-API entrypoint turbo-test doesn't implement used to
// jump to a null function pointer (unresolved flat-namespace symbol under
// -export_dynamic) -> SIGSEGV (exit 139), zero output, killing the ENTIRE run with no
// diagnostic. The fix exports every N-API symbol real addons reference (implementing the
// safe ones, routing the genuinely-unsupported ones to a catchable JS throw) and guards
// the addon's module-init + callback FFI calls with catch_unwind. A misbehaving addon
// must now surface as a normal catchable error and the rest of the run must survive.
//
// The fixture .node is compiled here with `cc` (it's gitignored, never committed). If no
// C compiler is available the test is skipped.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');
const FIX = path.join(ROOT, 'fixtures', 'napi');
const ADDON = path.join(FIX, 'bad_addon.node');

function haveCC() {
  return spawnSync('cc', ['--version'], { encoding: 'utf8' }).status === 0;
}

function buildAddon() {
  // -undefined dynamic_lookup: leave napi_* imports to be resolved against the host
  // (turbo-test) at load time, exactly like a real prebuilt napi-rs .node.
  const r = spawnSync(
    'cc',
    ['-shared', '-undefined', 'dynamic_lookup', '-o', ADDON, path.join(FIX, 'bad_addon.c')],
    { encoding: 'utf8' }
  );
  return r.status === 0;
}

function run(args) {
  // Spawn the native binary directly (NOT through cli.js) so we observe the real exit
  // signal: a SIGSEGV surfaces as status === null + signal 'SIGSEGV', which cli.js would
  // otherwise mask as a plain exit 1.
  const res = spawnSync('node', [CLI, ...args], { cwd: ROOT, encoding: 'utf8' });
  return { code: res.status, signal: res.signal, out: res.stdout || '', err: res.stderr || '' };
}

test('a misbehaving native .node addon does not SIGSEGV the run', { skip: !haveCC() && 'no cc' }, () => {
  assert.ok(buildAddon(), 'failed to build fixture addon');

  const BAD = path.join(FIX, 'bad-addon.spec.ts');
  const SURVIVES = path.join(FIX, 'survives.spec.ts');
  const r = run(['--reporter', 'json', BAD, SURVIVES]);

  // Hard requirement: NOT a crash. Before the fix this was signal SIGSEGV / code 139.
  assert.notEqual(r.signal, 'SIGSEGV', `runner segfaulted:\n${r.err}`);
  assert.notEqual(r.code, 139, `runner exited 139 (segfault):\n${r.err}`);

  // The addon spec asserts the load throws catchably; the sibling spec must have run too.
  const line = r.out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
  assert.ok(line, `no JSON summary (run likely died):\n${r.out}\n${r.err}`);
  const j = JSON.parse(line);
  assert.equal(j.numFailedTests, 0, 'all assertions (incl. the toThrow) should pass');
  assert.ok(j.numTotalTests >= 3, 'both spec files ran (addon spec + sibling survived)');

  fs.rmSync(ADDON, { force: true });
});
