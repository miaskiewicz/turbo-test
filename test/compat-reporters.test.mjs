// Reporter / --outputFile vitest-compatibility tests. Spawns the real launcher (cli.js),
// writes the active reporter to a temp --outputFile, and asserts on the XML/TAP/text content.
// Run via `npm test`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CLI = path.join(ROOT, 'cli.js');
const NAMES = path.join(ROOT, 'fixtures', 'compat', 'names.test.ts'); // 3 pass
const MIXED = path.join(ROOT, 'fixtures', 'compat', 'mixed.test.ts'); // 2 pass, 1 fail

function run(args, opts = {}) {
  const res = spawnSync('node', [CLI, ...args], { cwd: ROOT, encoding: 'utf8', ...opts });
  return { code: res.status, out: res.stdout || '', err: res.stderr || '' };
}

function tmp(name) {
  return path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'tt-rep-')), name);
}

// ---------------------------------------------------------------- JUnit XML
test('--reporter junit + --outputFile writes JUnit XML with per-testcase entries', () => {
  const out = tmp('junit.xml');
  const { code } = run(['--reporter', 'junit', '--outputFile', out, NAMES]);
  assert.equal(code, 0);
  const xml = fs.readFileSync(out, 'utf8');
  assert.match(xml, /<testsuites\b/);
  assert.match(xml, /<testsuite\b[^>]*tests="3"/);
  assert.match(xml, /failures="0"/);
  // each passing test should appear as its own <testcase> with the full describe>it name
  // (the `>` separator is XML-escaped to &gt; in attribute values)
  assert.match(xml, /<testcase\b[^>]*name="alpha group &gt; adds numbers"/);
  assert.match(xml, /<testcase\b[^>]*name="beta group &gt; concatenates strings"/);
});

test('--reporter junit records failures as <failure> inside the failing testcase', () => {
  const out = tmp('junit-fail.xml');
  const { code } = run(['--reporter', 'junit', '--outputFile', out, MIXED]);
  assert.equal(code, 1, 'a failing test must flip the exit code');
  const xml = fs.readFileSync(out, 'utf8');
  assert.match(xml, /<testsuite\b[^>]*tests="3"[^>]*failures="1"/);
  assert.match(xml, /<testcase\b[^>]*name="math &gt; fails subtraction"[\s\S]*?<failure\b/);
  // a passing case carries no <failure>: its closing tag follows on the next line, as in
  // vitest 4.1.12's report (turbo-test#81)
  assert.match(xml, /<testcase\b[^>]*name="math &gt; adds" time="[^"]*">\n        <\/testcase>\n/);
});

test('junit XML escapes special characters in names/messages', () => {
  const out = tmp('junit-esc.xml');
  run(['--reporter', 'junit', '--outputFile', out, MIXED]);
  const xml = fs.readFileSync(out, 'utf8');
  // no raw unescaped ampersand (every & must be an entity)
  assert.ok(!/&(?!amp;|lt;|gt;|quot;|apos;|#)/.test(xml), 'unescaped & in XML');
});

// ---------------------------------------------------------------- TAP
// vitest 4.1.12's TapReporter / TapFlatReporter (turbo-test#170): a file and its describes are
// nested subtests; tap-flat is one plan of `file > describe > test`
test('--reporter tap nests files and describes as subtests', () => {
  const { code, out } = run(['--reporter', 'tap', NAMES]);
  assert.equal(code, 0);
  assert.match(out, /^TAP version 13\n1\.\.1\nok 1 - fixtures\/compat\/names\.test\.ts # time=[\d.]+ms \{\n {4}1\.\.2\n {4}ok 1 - alpha group # time=[\d.]+ms \{\n {8}1\.\.2\n {8}ok 1 - adds numbers # time=[\d.]+ms\n/);
  assert.match(out, /^ {4}ok 2 - beta group # time=[\d.]+ms \{\n {8}1\.\.1\n {8}ok 1 - concatenates strings # time=[\d.]+ms\n {4}\}\n\}$/m);
});

test('--reporter tap marks failing tests `not ok` with vitest\'s error block', () => {
  const { code, out } = run(['--reporter', 'tap', MIXED]);
  assert.equal(code, 1);
  assert.match(out, /^not ok 1 - fixtures\/compat\/mixed\.test\.ts # time=/m);
  assert.match(out, /^ {8}not ok 2 - fails subtraction # time=[\d.]+ms\n {12}---\n {12}error:\n {16}name: "AssertionError"\n {16}message: "expected 2 to be 5 \/\/ Object\.is equality"\n {12}at: ".*mixed\.test\.ts:\d+:\d+"\n {12}actual: "2"\n {12}expected: "5"\n {12}\.\.\.$/m);
  assert.equal((out.match(/^ *not ok /gm) || []).length, 3);
});

test('--reporter tap-flat is one plan of file > describe > test', () => {
  const { code, out } = run(['--reporter', 'tap-flat', MIXED]);
  assert.equal(code, 1);
  assert.match(out, /^TAP version 13\n1\.\.3\n/);
  assert.match(out, /^not ok \d+ - fixtures\/compat\/mixed\.test\.ts > math > fails subtraction # time=/m);
  assert.equal((out.match(/^not ok /gm) || []).length, 1);
  assert.equal((out.match(/^ok /gm) || []).length, 2);
});

test('--reporter tap writes to --outputFile too', () => {
  const out = tmp('out.tap');
  run(['--reporter', 'tap', '--outputFile', out, NAMES]);
  const tap = fs.readFileSync(out, 'utf8');
  assert.match(tap, /^TAP version 13\n1\.\.1\nok 1 - fixtures\/compat\/names\.test\.ts # time=/);
});

// ---------------------------------------------------------------- verbose
const NO_COLOR = { env: { ...process.env, NO_COLOR: '1' } };
test('--reporter verbose prints each full test name with pass/fail mark', () => {
  const { out } = run(['--reporter', 'verbose', MIXED], NO_COLOR);
  assert.match(out, /alpha|math > adds/); // adds appears
  assert.match(out, /math > adds/);
  assert.match(out, /math > fails subtraction/);
  assert.match(out, /strings > concatenates/);
});

// ---------------------------------------------------------------- dot / default
test('--reporter dot prints one char per file and still exits by status', () => {
  const pass = run(['--reporter', 'dot', NAMES]);
  assert.equal(pass.code, 0);
  const fail = run(['--reporter', 'dot', MIXED]);
  assert.equal(fail.code, 1);
});

// vitest's default reporter (turbo-test#94): a passing file is its module line
test('--reporter default prints vitest\'s module line', () => {
  const { code, out } = run(['--reporter', 'default', NAMES], NO_COLOR);
  assert.equal(code, 0);
  assert.match(out, / ✓ fixtures\/compat\/names\.test\.ts \(3 tests\) \d+ms\n/);
});

// ---------------------------------------------------------------- json + outputFile
test('--reporter json honors --outputFile (writes JSON to disk)', () => {
  const out = tmp('out.json');
  run(['--reporter', 'json', '--outputFile', out, NAMES]);
  const j = JSON.parse(fs.readFileSync(out, 'utf8'));
  assert.equal(j.numPassedTests, 3);
  assert.equal(j.numFailedTests, 0);
});

// ---------------------------------------------------------------- unknown reporter
test('an unimplemented reporter value is accepted (falls back to text), never errors', () => {
  const { code, out } = run(['--reporter', 'html', NAMES], NO_COLOR);
  assert.equal(code, 0, 'unknown reporter must not error');
  assert.match(out, / ✓ fixtures\/compat\/names\.test\.ts \(3 tests\) \d+ms\n/);
});
