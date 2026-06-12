#!/usr/bin/env node
'use strict';
// Launcher for the native turbo-test binary. Resolves the prebuilt binary for this platform,
// expands default test-file patterns when none are given, and execs it inheriting stdio.
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

// On Linux, glibc vs musl (Alpine) need different binaries but report the same
// process.platform/arch — detect musl so we pick the right one.
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
  // Prefer the musl build on musl systems, fall back to the default name.
  const names = isMusl() ? [`${base}-musl${ext}`, `${base}${ext}`] : [`${base}${ext}`];
  for (const name of names) {
    const p = path.join(__dirname, 'bin', name);
    if (fs.existsSync(p)) return p;
  }
  // dev fallback: a cargo build in this repo
  const dev = path.join(__dirname, 'target', 'release', `turbo-test${ext}`);
  if (fs.existsSync(dev)) return dev;
  return null;
}

// Default test-file discovery (vitest-style) when the user passes no file args.
const TEST_RE = /\.(test|spec)\.(ts|tsx|js|jsx|mts|cts)$/;
const SKIP_DIR = new Set(['node_modules', '.git', 'dist', 'build', 'coverage', '.next', '.turbo', 'target']);
function walk(dir, out) {
  let entries;
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const e of entries) {
    if (e.name.startsWith('.') && e.name !== '.') continue;
    const full = path.join(dir, e.name);
    if (e.isDirectory()) { if (!SKIP_DIR.has(e.name)) walk(full, out); }
    else if (TEST_RE.test(e.name)) out.push(full);
  }
  return out;
}

function main() {
  const bin = binaryPath();
  if (!bin) {
    console.error(
      `turbo-test: no prebuilt binary for ${process.platform}-${process.arch}.\n` +
      `Build from source (requires Rust): cargo build --release  (in the turbo-test repo).`
    );
    process.exit(1);
  }
  const argv = process.argv.slice(2);
  // split flags (start with -) from file/glob args
  const flags = [];
  const files = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('-')) {
      flags.push(a);
      // flags that take a value: --jobs N, --shard i/n, --reporter X
      if (/^(-j|--jobs|--shard|--reporter)$/.test(a) && i + 1 < argv.length && !argv[i + 1].startsWith('-')) {
        flags.push(argv[++i]);
      }
    } else {
      files.push(a);
    }
  }
  let testFiles = files;
  if (testFiles.length === 0) {
    testFiles = walk(process.cwd(), []).sort();
    if (testFiles.length === 0) {
      console.error('turbo-test: no test files found (looked for *.test.* / *.spec.*).');
      process.exit(1);
    }
  }
  const res = spawnSync(bin, [...flags, ...testFiles], { stdio: 'inherit' });
  if (res.error) { console.error('turbo-test:', res.error.message); process.exit(1); }
  process.exit(res.status == null ? 1 : res.status);
}

main();
