#!/usr/bin/env node
// Check turbo-test's CI detection against std-env's `isCI` (turbo-test#74).
//
//   node scripts/check-ci-detection.mjs <project dir with std-env installed> [turbo-test binary]
//
// The project dir is one where vitest is installed (std-env resolves from it, e.g. vitest
// 4.1.12's std-env 4.3.0); the binary defaults to target/release/turbo-test. For every row of
// src/ci_providers.json (its variable set, then set but empty), every `CI` value, the rows
// that do not count as CI next to their neighbours, and a few variables std-env ignores, it
// asks std-env for `isCI` in a fresh node process and turbo-test for its decision, observed
// the way vitest's shows: a missing snapshot fails and is not written on CI, and is written
// elsewhere. Prints each mismatch and exits 1 if there is any.
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const [project, binArg] = process.argv.slice(2);
if (!project) {
  console.error('usage: node scripts/check-ci-detection.mjs <project dir with std-env> [turbo-test binary]');
  process.exit(2);
}
const entry = createRequire(path.join(path.resolve(project), 'package.json')).resolve('std-env');
let pkgDir = path.dirname(entry);
while (!fs.existsSync(path.join(pkgDir, 'package.json'))) pkgDir = path.dirname(pkgDir);
const stdEnvVersion = JSON.parse(fs.readFileSync(path.join(pkgDir, 'package.json'), 'utf8')).version;
const bin = path.resolve(binArg || path.join(ROOT, 'target', 'release', 'turbo-test'));
const rows = JSON.parse(fs.readFileSync(path.join(ROOT, 'src', 'ci_providers.json'), 'utf8')).providers;

const cases = [];
for (const r of rows) {
  cases.push({ [r.env]: '1' }, { [r.env]: '' });
}
for (const v of ['true', '1', 'false', '0', 'yes', '']) cases.push({ CI: v });
cases.push({ CI: '', GITHUB_ACTIONS: 'true' });
rows.forEach((r, i) => {
  if (r.ci) return;
  if (i > 0) cases.push({ [rows[i - 1].env]: '1', [r.env]: '1' });
  if (i + 1 < rows.length) cases.push({ [r.env]: '1', [rows[i + 1].env]: '1' });
  cases.push({ [r.env]: '1', CI: 'true' });
});
for (const k of ['TF_BUILD', 'BITBUCKET_BUILD_NUMBER', 'BUILD_ID', 'TEST']) cases.push({ [k]: '1' });
cases.push({ NODE_ENV: 'test' });

const base = { PATH: process.env.PATH, HOME: process.env.HOME || os.homedir() };
function stdEnvIsCI(env) {
  const code = `const m = await import(${JSON.stringify(pathToFileURL(entry).href)}); process.stdout.write(String(m.isCI));`;
  const r = spawnSync(process.execPath, ['--input-type=module', '-e', code], { env: { ...base, ...env }, encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`std-env probe failed: ${r.stderr}`);
  return r.stdout === 'true';
}

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tt-ci-check-'));
fs.writeFileSync(path.join(dir, 'package.json'), '{ "type": "module" }\n');
fs.writeFileSync(
  path.join(dir, 's.test.js'),
  "import { test, expect } from 'vitest'\ntest('snap', () => { expect({ a: 1 }).toMatchSnapshot() })\n",
);
const snapDir = path.join(dir, '__snapshots__');
function turboTestIsCI(env) {
  fs.rmSync(snapDir, { recursive: true, force: true });
  const r = spawnSync(bin, ['s.test.js'], { cwd: dir, env: { ...base, TMPDIR: dir, ...env }, encoding: 'utf8' });
  const written = fs.existsSync(path.join(snapDir, 's.test.js.snap'));
  if (r.status === 1 && !written) return true;
  if (r.status === 0 && written) return false;
  throw new Error(`turbo-test run ${JSON.stringify(env)}: exit ${r.status}, snapshot written ${written}\n${r.error || ''}${r.stdout || ''}${r.stderr || ''}`);
}

let bad = 0;
for (const env of cases) {
  const want = stdEnvIsCI(env);
  const got = turboTestIsCI(env);
  if (want !== got) {
    bad++;
    console.log(`MISMATCH ${JSON.stringify(env)}: std-env isCI=${want}, turbo-test=${got}`);
  }
}
fs.rmSync(dir, { recursive: true, force: true });
console.log(`std-env ${stdEnvVersion} (${entry}) vs ${bin}: ${rows.length} rows, ${cases.length} cases, ${bad} mismatches`);
process.exit(bad ? 1 : 0);
