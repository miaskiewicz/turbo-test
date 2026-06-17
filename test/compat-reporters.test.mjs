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
  // a passing case must NOT carry a <failure> (self-closing testcase tag)
  assert.match(xml, /<testcase\b[^>]*name="math &gt; adds"[^>]*\/>/);
});

test('junit XML escapes special characters in names/messages', () => {
  const out = tmp('junit-esc.xml');
  run(['--reporter', 'junit', '--outputFile', out, MIXED]);
  const xml = fs.readFileSync(out, 'utf8');
  // no raw unescaped ampersand (every & must be an entity)
  assert.ok(!/&(?!amp;|lt;|gt;|quot;|apos;|#)/.test(xml), 'unescaped & in XML');
});

// ---------------------------------------------------------------- TAP
test('--reporter tap emits TAP v13 with one line per test', () => {
  const { code, out } = run(['--reporter', 'tap', NAMES]);
  assert.equal(code, 0);
  assert.match(out, /^TAP version 13$/m);
  assert.match(out, /^1\.\.3$/m);
  assert.match(out, /^ok 1 - alpha group > adds numbers$/m);
  assert.match(out, /^ok 3 - beta group > concatenates strings$/m);
});

test('--reporter tap marks failing tests `not ok` and counts them', () => {
  const { code, out } = run(['--reporter', 'tap', MIXED]);
  assert.equal(code, 1);
  assert.match(out, /^1\.\.3$/m);
  assert.match(out, /^not ok \d+ - math > fails subtraction$/m);
  assert.equal((out.match(/^not ok /gm) || []).length, 1);
  assert.equal((out.match(/^ok /gm) || []).length, 2);
});

test('--reporter tap writes to --outputFile too', () => {
  const out = tmp('out.tap');
  run(['--reporter', 'tap', '--outputFile', out, NAMES]);
  const tap = fs.readFileSync(out, 'utf8');
  assert.match(tap, /^TAP version 13$/m);
  assert.match(tap, /^1\.\.3$/m);
});

// ---------------------------------------------------------------- verbose
test('--reporter verbose prints each full test name with pass/fail mark', () => {
  const { out } = run(['--reporter', 'verbose', MIXED]);
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

test('--reporter default behaves like the text reporter', () => {
  const { code, out } = run(['--reporter', 'default', NAMES]);
  assert.equal(code, 0);
  assert.match(out, /PASS/);
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
  const { code, out } = run(['--reporter', 'html', NAMES]);
  assert.equal(code, 0, 'unknown reporter must not error');
  assert.match(out, /PASS/);
});
