#!/usr/bin/env node
'use strict';
// Thin launcher for the native turbo-test binary. Resolves the prebuilt binary for this platform
// and execs it, forwarding argv verbatim and inheriting stdio. ALL launcher logic — vitest
// subcommand stripping, flag parsing, default test-file discovery, vitest config include/exclude,
// coverage-config injection, --changed git filtering, environment/isolate env vars — now lives in
// the native binary (src/launcher.rs). This file exists only to (a) locate the right platform
// binary inside the npm package and (b) hand off. The npm package is a thin distribution wrapper;
// no Node-side runtime logic remains.
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

// turbo-test's own esbuild (a dependency of this package), for projects that don't link one at
// node_modules/.bin/esbuild — e.g. pnpm, where a transitive esbuild lives only under
// node_modules/.pnpm. Without it the runner can't transform vi.mock factories (TS/JSX) and
// evaluates them raw. The native binary prefers the project's own esbuild when it has one.
function bundledEsbuild() {
  try {
    const dir = path.dirname(require.resolve('esbuild/package.json'));
    const p =
      process.platform === 'win32'
        ? path.join(
            path.dirname(require.resolve(`@esbuild/win32-${process.arch}/package.json`, { paths: [dir] })),
            'esbuild.exe',
          )
        : path.join(dir, 'bin', 'esbuild');
    return fs.existsSync(p) ? p : null;
  } catch {
    return null;
  }
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
  // Forward argv verbatim; the binary's launcher (src/launcher.rs) does the rest. cwd is inherited
  // so the binary's discovery / config walk / git --changed all run relative to where the user is.
  const env = { ...process.env };
  if (!env.TURBO_ESBUILD) {
    const esbuild = bundledEsbuild();
    if (esbuild) env.TURBO_ESBUILD = esbuild;
  }
  const res = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit', env });
  if (res.error) { console.error('turbo-test:', res.error.message); process.exit(1); }
  process.exit(res.status == null ? 1 : res.status);
}

main();
