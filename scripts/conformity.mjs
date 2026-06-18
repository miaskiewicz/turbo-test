#!/usr/bin/env node
'use strict';
// Conformity harness — guarantees the native oxc ESM→CJS emitter (TURBO_NATIVE_CJS) produces
// behaviorally IDENTICAL test results to the esbuild path it replaces (P2). Runs a target
// project's suite both ways with the dev binary and diffs per-FILE pass/fail.
//
// Modes:
//   parity   (default) baseline = esbuild, candidate = native (with esbuild fallback). Diffs the
//            two result sets. Any file whose pass/fail count or status differs is a CONFORMITY
//            FAILURE — the native emitter changed behavior and must be fixed before cutover.
//   coverage candidate = native-STRICT (no esbuild fallback). Reports how many files the native
//            emitter handles end-to-end (the rest load-error). Measures readiness to delete esbuild.
//
// Usage:
//   node scripts/conformity.mjs <project-dir> [--mode parity|coverage] [--jobs N] [-- <files...>]
//   node scripts/conformity.mjs <project-dir> --json out.json
//
// The binary self-discovers test files (P1), so with no file args it runs the project's whole
// suite. Pass explicit files after `--` to scope a run while iterating on the emitter.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BIN = path.join(ROOT, 'target', 'release', 'turbo-test');

function parseArgs(argv) {
  const out = { project: null, mode: 'parity', jobs: null, files: [], json: null };
  let i = 0;
  for (; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--mode') out.mode = argv[++i];
    else if (a === '--jobs') out.jobs = argv[++i];
    else if (a === '--json') out.json = argv[++i];
    else if (a === '--') { out.files = argv.slice(i + 1); break; }
    else if (!out.project) out.project = a;
    else out.files.push(a);
  }
  return out;
}

// Run the dev binary in `dir` with the given env overrides; parse the JSON reporter summary into a
// file→{status,pass,fail} map plus totals. Returns null if the binary produced no JSON.
function runMode(dir, env, files, jobs) {
  if (!fs.existsSync(BIN)) {
    console.error(`conformity: binary not found at ${BIN} — run \`cargo build --release\` first.`);
    process.exit(2);
  }
  const args = ['--reporter', 'json'];
  if (jobs) args.push('--jobs', String(jobs));
  args.push(...files);
  const res = spawnSync(BIN, args, {
    cwd: dir,
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
    env: { ...process.env, ...env },
  });
  const out = res.stdout || '';
  const line = out.split('\n').find((l) => l.trim().startsWith('{') && l.includes('numTotalTests'));
  if (!line) {
    console.error(`conformity: no JSON summary from a run in ${dir}.\nstderr tail:\n${(res.stderr || '').split('\n').slice(-15).join('\n')}`);
    return null;
  }
  const j = JSON.parse(line);
  const byFile = new Map();
  for (const tr of j.testResults || []) {
    byFile.set(path.resolve(dir, tr.name), {
      status: tr.status,
      pass: tr.numPassingTests | 0,
      fail: tr.numFailingTests | 0,
    });
  }
  return {
    total: j.numTotalTests | 0,
    pass: j.numPassedTests | 0,
    fail: j.numFailedTests | 0,
    suites: j.numTotalTestSuites | 0,
    byFile,
  };
}

function main() {
  const opt = parseArgs(process.argv.slice(2));
  if (!opt.project) {
    console.error('usage: node scripts/conformity.mjs <project-dir> [--mode parity|coverage] [--jobs N] [-- <files...>]');
    process.exit(2);
  }
  const dir = path.resolve(opt.project);
  const t0 = Date.now();

  if (opt.mode === 'coverage') {
    // native-strict: unhandled files load-error. Compare to esbuild baseline to attribute failures.
    console.error(`conformity[coverage]: ${dir}`);
    const base = runMode(dir, { TURBO_NATIVE_CJS: '0', TURBO_NATIVE_CJS_STRICT: '' }, opt.files, opt.jobs);
    const ntv = runMode(dir, { TURBO_NATIVE_CJS: '1', TURBO_NATIVE_CJS_STRICT: '1' }, opt.files, opt.jobs);
    if (!base || !ntv) process.exit(2);
    let handled = 0, unhandled = 0;
    const broke = [];
    for (const [file, b] of base.byFile) {
      const n = ntv.byFile.get(file);
      if (!n) { unhandled++; continue; }
      if (n.status === b.status && n.pass === b.pass && n.fail === b.fail) handled++;
      else { unhandled++; broke.push({ file, b, n }); }
    }
    const pct = base.byFile.size ? ((handled / base.byFile.size) * 100).toFixed(1) : '0.0';
    console.log(`\nNative coverage: ${handled}/${base.byFile.size} files handled identically (${pct}%), ${unhandled} not.`);
    for (const x of broke.slice(0, 40)) {
      console.log(`  DIVERGE ${path.relative(dir, x.file)}  esbuild=${x.b.pass}/${x.b.fail} native=${x.n.pass}/${x.n.fail} [${x.n.status}]`);
    }
    if (broke.length > 40) console.log(`  … +${broke.length - 40} more`);
    console.error(`(${((Date.now() - t0) / 1000).toFixed(1)}s)`);
    process.exit(unhandled === 0 ? 0 : 1);
  }

  // parity mode — candidate enables native app transform AND native node_modules bundling.
  console.error(`conformity[parity]: ${dir}  (esbuild baseline vs native app+deps)`);
  const base = runMode(dir, { TURBO_NATIVE_CJS: '0', TURBO_NATIVE_DEPS: '0', TURBO_NATIVE_CJS_STRICT: '' }, opt.files, opt.jobs);
  const ntv = runMode(dir, { TURBO_NATIVE_CJS: '1', TURBO_NATIVE_DEPS: '1', TURBO_NATIVE_CJS_STRICT: '' }, opt.files, opt.jobs);
  if (!base || !ntv) process.exit(2);

  const diverged = [];
  const allFiles = new Set([...base.byFile.keys(), ...ntv.byFile.keys()]);
  for (const file of allFiles) {
    const b = base.byFile.get(file);
    const n = ntv.byFile.get(file);
    if (!b || !n) { diverged.push({ file, b, n, reason: 'missing in one mode' }); continue; }
    if (b.status !== n.status || b.pass !== n.pass || b.fail !== n.fail) {
      diverged.push({ file, b, n, reason: 'count/status differ' });
    }
  }

  console.log('');
  console.log(`baseline (esbuild): ${base.pass} pass / ${base.fail} fail / ${base.total} tests / ${base.suites} files`);
  console.log(`native   (oxc)    : ${ntv.pass} pass / ${ntv.fail} fail / ${ntv.total} tests / ${ntv.suites} files`);
  if (diverged.length === 0) {
    console.log(`\n✅ PARITY: native results identical to esbuild across all ${allFiles.size} files.`);
  } else {
    console.log(`\n❌ CONFORMITY FAILURE: ${diverged.length} file(s) diverge:`);
    for (const d of diverged.slice(0, 50)) {
      const bs = d.b ? `${d.b.pass}/${d.b.fail}[${d.b.status}]` : 'absent';
      const ns = d.n ? `${d.n.pass}/${d.n.fail}[${d.n.status}]` : 'absent';
      console.log(`  ${path.relative(dir, d.file)}  esbuild=${bs} native=${ns}  (${d.reason})`);
    }
    if (diverged.length > 50) console.log(`  … +${diverged.length - 50} more`);
  }
  if (opt.json) {
    fs.writeFileSync(opt.json, JSON.stringify({ base: { ...base, byFile: undefined }, native: { ...ntv, byFile: undefined }, diverged: diverged.map((d) => ({ ...d, file: path.relative(dir, d.file) })) }, null, 2));
    console.error(`wrote ${opt.json}`);
  }
  console.error(`(${((Date.now() - t0) / 1000).toFixed(1)}s)`);
  process.exit(diverged.length === 0 ? 0 : 1);
}

main();
