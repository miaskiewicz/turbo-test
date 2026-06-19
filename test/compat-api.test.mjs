// Test/expect API-compatibility tests. Spawns the real launcher (cli.js) the way a user / CI
// would, against fixtures under fixtures/compat/, and asserts on the JSON reporter summary.
// Covers: snapshots (+ -u update), expect.assertions/hasAssertions enforcement, it.fails,
// describe.skipIf/runIf/concurrent/todo, it.extend fixtures, and a few common matchers.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');
const FIX = path.join(ROOT, 'fixtures', 'compat');

function run(args) {
  const res = spawnSync('node', [CLI, ...args], { cwd: ROOT, encoding: 'utf8' });
  return { code: res.status, out: res.stdout || '', err: res.stderr || '' };
}
function parseJson(out) {
  const line = out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
  assert.ok(line, `no JSON summary in output:\n${out}`);
  return JSON.parse(line);
}
const file = (name) => path.join(FIX, name);

test('toMatchSnapshot: first run writes the snap and passes; second run compares and passes', () => {
  const snapDir = path.join(FIX, '__snapshots__');
  const snap = path.join(snapDir, 'snapshot.test.ts.snap');
  fs.rmSync(snap, { force: true });

  const first = parseJson(run(['--reporter', 'json', file('snapshot.test.ts')]).out);
  assert.equal(first.numPassedTests, 5, 'all snapshot assertions pass on first (write) run');
  assert.equal(first.numFailedTests, 0);
  assert.ok(fs.existsSync(snap), 'a .snap file was written');
  assert.match(fs.readFileSync(snap, 'utf8'), /matches an object snapshot 1/, 'keyed by full test name + counter');

  const second = parseJson(run(['--reporter', 'json', file('snapshot.test.ts')]).out);
  assert.equal(second.numPassedTests, 5, 'second run compares against the stored snapshot and passes');
  assert.equal(second.numFailedTests, 0);
});

test('toMatchSnapshot: a changed value fails; -u/--update rewrites and re-passes', () => {
  const snap = path.join(FIX, '__snapshots__', 'snapshot.test.ts.snap');
  // ensure a baseline exists
  run(['--reporter', 'json', file('snapshot.test.ts')]);
  // corrupt the primitive snapshot so the next compare mismatches
  fs.writeFileSync(snap, fs.readFileSync(snap, 'utf8').replace(/\n42\n/, '\n999\n'));

  const bad = parseJson(run(['--reporter', 'json', file('snapshot.test.ts')]).out);
  assert.equal(bad.numFailedTests, 1, 'mismatched snapshot fails');

  const upd = parseJson(run(['-u', '--reporter', 'json', file('snapshot.test.ts')]).out);
  assert.equal(upd.numFailedTests, 0, '-u rewrites the snapshot');
  assert.match(fs.readFileSync(snap, 'utf8'), /matches a primitive snapshot 1`\] = `\n42\n/, 'snapshot rewritten to 42');

  const after = parseJson(run(['--reporter', 'json', file('snapshot.test.ts')]).out);
  assert.equal(after.numFailedTests, 0, 're-compares cleanly after update');
});

test('expect.assertions(n) / hasAssertions() are enforced', () => {
  const j = parseJson(run(['--reporter', 'json', file('assertions.test.ts')]).out);
  // 2 pass (count matches / hasAssertions satisfied), 2 fail (count short / no assertion ran)
  assert.equal(j.numPassedTests, 2);
  assert.equal(j.numFailedTests, 2);
});

test('it.fails passes only when the test body throws', () => {
  const j = parseJson(run(['--reporter', 'json', file('itfails.test.ts')]).out);
  assert.equal(j.numPassedTests, 2);
  assert.equal(j.numFailedTests, 0);
});

test('describe.skipIf / runIf / concurrent / todo', () => {
  const j = parseJson(run(['--reporter', 'json', file('describe-variants.test.ts')]).out);
  // only the 3 included blocks run (2 skipped blocks contribute nothing); todo registers nothing
  assert.equal(j.numTotalTests, 3, 'skipped describe blocks register no tests');
  assert.equal(j.numPassedTests, 3);
  assert.equal(j.numFailedTests, 0);
});

test('it.extend provides test-context fixtures', () => {
  const j = parseJson(run(['--reporter', 'json', file('extend.test.ts')]).out);
  assert.equal(j.numPassedTests, 2);
  assert.equal(j.numFailedTests, 0);
});

test('common matchers: toMatchObject / toContainEqual / toSatisfy / toHaveBeenCalledOnce / toHaveBeenNthCalledWith', () => {
  const j = parseJson(run(['--reporter', 'json', file('matchers.test.ts')]).out);
  assert.equal(j.numPassedTests, 5);
  assert.equal(j.numFailedTests, 0);
});

test('extra HTML*Element constructor globals + tag-keyed instanceof + constructor.name', () => {
  const j = parseJson(run(['--reporter', 'json', file('html-element-ctors.test.ts')]).out);
  assert.equal(j.numPassedTests, 3);
  assert.equal(j.numFailedTests, 0);
});

test('constructable CSSStyleSheet + adoptedStyleSheets (emotion/MUI adopt pattern)', () => {
  const j = parseJson(run(['--reporter', 'json', file('constructable-stylesheet.test.ts')]).out);
  assert.equal(j.numPassedTests, 4);
  assert.equal(j.numFailedTests, 0);
});
