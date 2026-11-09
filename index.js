'use strict';
// Programmatic entry: spawn the native turbo-test binary. The CLI (cli.js) is the primary
// interface; this exists so `require('@miaskiewicz/turbo-test')` works in scripts.
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

function isMusl() {
  if (process.platform !== 'linux') return false;
  try {
    return !process.report.getReport().header.glibcVersionRuntime;
  } catch {
    return false;
  }
}

function binaryPath() {
  const ext = process.platform === 'win32' ? '.exe' : '';
  const base = `turbo-test-${process.platform}-${process.arch}`;
  const names = isMusl() ? [`${base}-musl${ext}`, `${base}${ext}`] : [`${base}${ext}`];
  for (const name of names) {
    const local = path.join(__dirname, 'bin', name);
    if (fs.existsSync(local)) return local;
  }
  const dev = path.join(__dirname, 'target', 'release', `turbo-test${ext}`);
  if (fs.existsSync(dev)) return dev;
  return null;
}

function pushFlag(args, flag, value) {
  if (value) args.push(flag, String(value));
}

function pushCoverage(args, opts) {
  if (opts.coverageDir) args.push('--coverage-dir', String(opts.coverageDir));
  else if (opts.coverage) args.push('--coverage');
}

function buildRunArgs(opts, files) {
  const args = [];
  pushFlag(args, '--jobs', opts.jobs);
  pushFlag(args, '--reporter', opts.reporter);
  pushFlag(args, '--shard', opts.shard);
  pushCoverage(args, opts);
  args.push(...files);
  return args;
}

/**
 * Run turbo-test over the given files.
 * @param {string[]} files  test file paths
 * @param {{ jobs?: number, reporter?: string, shard?: string, coverage?: boolean, coverageDir?: string, env?: object }} [opts]
 * @returns {{ status: number }}
 */
function run(files, opts = {}) {
  const bin = binaryPath();
  if (!bin) throw new Error(`turbo-test: no binary for ${process.platform}-${process.arch}`);
  const args = buildRunArgs(opts, files);
  const res = spawnSync(bin, args, { stdio: 'inherit', env: { ...process.env, ...(opts.env || {}) } });
  return { status: res.status == null ? 1 : res.status };
}

module.exports = { run, binaryPath };
