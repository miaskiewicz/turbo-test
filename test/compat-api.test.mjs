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
// Some fixtures need their OWN cwd so the launcher discovers their vitest.config.ts (resolve.alias,
// test.alias) walking up from there — run them with cwd set to the fixture directory.
function runIn(cwd, args) {
  const res = spawnSync('node', [CLI, ...args], { cwd, encoding: 'utf8' });
  return { code: res.status, out: res.stdout || '', err: res.stderr || '' };
}

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

// issue #18: a top-level window must self-reference (window.parent/top/self === window), matching
// browsers/jsdom/vitest — otherwise iframe-detection (window.parent !== window) reads "always framed".
test('top-level window self-references (window.parent/top/self/frames === window, frameElement null)', () => {
  const j = parseJson(run(['--reporter', 'json', file('window-framing.test.ts')]).out);
  assert.equal(j.numPassedTests, 4);
  assert.equal(j.numFailedTests, 0);
});

// vite/vitest resolve.alias (object form, path.resolve) + test.alias (array find/replacement) —
// turbo-test resolves these natively; the same thing vite-tsconfig-paths' non-tsconfig path map does.
test('resolve.alias + test.alias: @ / ~lib prefix aliases and @math exact alias resolve', () => {
  const dir = path.join(FIX, 'alias');
  const j = parseJson(runIn(dir, ['--reporter', 'json', 'alias.test.ts']).out);
  assert.equal(j.numPassedTests, 3);
  assert.equal(j.numFailedTests, 0);
});

// vite-plugin-svgr: `import Icon from './x.svg?react'` yields a render-safe <svg> React component
// (default + legacy ReactComponent export). turbo-test resolves the `?react` query natively.
test('vite-plugin-svgr ?react import resolves to a render-safe React component', () => {
  const dir = path.join(FIX, 'svgr');
  const j = parseJson(runIn(dir, ['--reporter', 'json', 'svgr.test.ts']).out);
  assert.equal(j.numPassedTests, 5);
  assert.equal(j.numFailedTests, 0);
});

// issue #15: Web Streams used to be a stub whose reader always resolved { done: true } — the
// globals existed but any consumer saw an empty stream. Now they actually stream.
test('Web Streams actually deliver enqueued chunks (ReadableStream/WritableStream/TransformStream/pipe/tee)', () => {
  const j = parseJson(run(['--reporter', 'json', file('streams.test.ts')]).out);
  assert.equal(j.numPassedTests, 15);
  assert.equal(j.numFailedTests, 0);
});

// issue #16: `expectTypeOf` / `assertType` must resolve from the `vitest` module (named AND
// namespace import) and be callable — a runtime no-op backing the tsc-checked type assertions.
test('expectTypeOf / assertType are exported from vitest and callable (named + namespace import)', () => {
  const j = parseJson(run(['--reporter', 'json', file('expecttypeof.test.ts')]).out);
  assert.equal(j.numPassedTests, 5);
  assert.equal(j.numFailedTests, 0);
});

test('constructable CSSStyleSheet + adoptedStyleSheets (emotion/MUI adopt pattern)', () => {
  const j = parseJson(run(['--reporter', 'json', file('constructable-stylesheet.test.ts')]).out);
  assert.equal(j.numPassedTests, 4);
  assert.equal(j.numFailedTests, 0);
});

test('circular module graph (A <-> B) loads without a TDZ "before initialization" error', () => {
  const j = parseJson(run(['--reporter', 'json', file('circular/circular.test.ts')]).out);
  assert.equal(j.numPassedTests, 4, 'all four assertions across the cycle pass');
  assert.equal(j.numFailedTests, 0);
});

test('circular decorated models (@Entity/@Field, User <-> Post) load — legacy decorator lowering', () => {
  // Regression for the Sequelize/NestJS case: without legacy-decorator lowering the native
  // transform emitted `export @Entity class …` (Unexpected token 'export') or a 2022-standard
  // class-binding TDZ ("Cannot access 'X' before initialization"). Both must load now.
  const j = parseJson(run(['--reporter', 'json', file('circular-decorators/circular-decorators.test.ts')]).out);
  assert.equal(j.numPassedTests, 2);
  assert.equal(j.numFailedTests, 0);
});
