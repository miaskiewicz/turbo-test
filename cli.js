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

// ---- vitest include/exclude honoring -------------------------------------
// Compile a glob (supporting **, *, ?, {a,b}) to an anchored RegExp matched
// against a project-root-relative POSIX path.
function globToRe(glob) {
  let re = '^';
  for (let i = 0; i < glob.length;) {
    const c = glob[i];
    if (c === '*') {
      if (glob[i + 1] === '*') {
        if (glob[i + 2] === '/') { re += '(?:.*/)?'; i += 3; } // **/  → zero+ dirs
        else { re += '.*'; i += 2; }
      } else { re += '[^/]*'; i++; }
    } else if (c === '?') { re += '[^/]'; i++; }
    else if (c === '{') { re += '(?:'; i++; }
    else if (c === '}') { re += ')'; i++; }
    else if (c === ',') { re += '|'; i++; }
    else if ('.+^$()|[]\\'.includes(c)) { re += '\\' + c; i++; }
    else { re += c; i++; }
  }
  return new RegExp(re + '$');
}

// Find the nearest vitest/vite config and pull its test-level include/exclude
// globs (the FIRST arrays in the file — test.* precedes any coverage.* block).
// Returns { root, include:[RegExp], exclude:[RegExp] } or null if none/unparseable.
function vitestPatterns(startDir) {
  const names = ['vitest.config.ts', 'vitest.config.mts', 'vitest.config.js', 'vitest.config.mjs',
                 'vite.config.ts', 'vite.config.mts', 'vite.config.js', 'vite.config.mjs'];
  let dir = startDir;
  for (;;) {
    for (const n of names) {
      const p = path.join(dir, n);
      if (!fs.existsSync(p)) continue;
      let text;
      try { text = fs.readFileSync(p, 'utf8'); } catch { return null; }
      const arr = (key) => {
        const m = text.match(new RegExp(key + '\\s*:\\s*\\[([^\\]]*)\\]'));
        if (!m) return null;
        const items = m[1].match(/['"`]([^'"`]+)['"`]/g);
        return items ? items.map((s) => s.slice(1, -1)) : [];
      };
      const inc = arr('include');
      const exc = arr('exclude');
      if (!inc) return null; // no test.include → fall back to default discovery
      return {
        root: dir,
        include: inc.map(globToRe),
        exclude: (exc || []).map(globToRe),
      };
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

// Locate the nearest vitest/vite config and return { dir, text } or null.
function findConfig(startDir) {
  const names = ['vitest.config.ts', 'vitest.config.mts', 'vitest.config.js', 'vitest.config.mjs',
                 'vite.config.ts', 'vite.config.mts', 'vite.config.js', 'vite.config.mjs'];
  let dir = startDir;
  for (;;) {
    for (const n of names) {
      const p = path.join(dir, n);
      if (!fs.existsSync(p)) continue;
      try { return { dir, text: fs.readFileSync(p, 'utf8') }; } catch { return null; }
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

// Pull the vitest `coverage` block's include/exclude globs + thresholds so the gate, lcov report
// set, and JSON summary can be driven from config (no flags needed). Config-reading parity with
// how we already read test.include/exclude — string-scan, no TS evaluation.
function vitestCoverage(startDir) {
  const cfg = findConfig(startDir);
  if (!cfg) return null;
  const text = cfg.text;
  // slice from the `coverage:` key so include/exclude/thresholds resolve to the coverage block,
  // not the test-level ones (test.* precedes coverage.* in the config object).
  const ci = text.search(/coverage\s*:\s*\{/);
  if (ci < 0) return { include: [], exclude: [], thresholds: null };
  const slice = text.slice(ci);
  const arr = (key) => {
    const m = slice.match(new RegExp(key + '\\s*:\\s*\\[([^\\]]*)\\]'));
    if (!m) return [];
    const items = m[1].match(/['"`]([^'"`]+)['"`]/g);
    return items ? items.map((s) => s.slice(1, -1)) : [];
  };
  // thresholds can be `coverage.thresholds: { lines: 90, ... }` or the flat legacy form.
  const thrText = (() => {
    const m = slice.match(/thresholds\s*:\s*\{([^}]*)\}/);
    return m ? m[1] : slice;
  })();
  const num = (key) => {
    const m = thrText.match(new RegExp('(?:^|[^.\\w])' + key + '\\s*:\\s*(\\d+(?:\\.\\d+)?)'));
    return m ? m[1] : null;
  };
  const parts = [];
  for (const k of ['lines', 'functions', 'branches', 'statements']) {
    const v = num(k);
    if (v != null) parts.push(`${k}=${v}`);
  }
  return {
    include: arr('include'),
    exclude: arr('exclude'),
    thresholds: parts.length ? parts.join(',') : null,
  };
}

function discover(cwd) {
  const all = walk(cwd, []);
  const pats = vitestPatterns(cwd);
  if (!pats) return all.sort();
  // vitest matches globs against the project-root-relative POSIX path.
  const rel = (f) => path.relative(pats.root, f).split(path.sep).join('/');
  const kept = all.filter((f) => {
    const r = rel(f);
    return pats.include.some((re) => re.test(r)) && !pats.exclude.some((re) => re.test(r));
  });
  return kept.sort();
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
  let argv = process.argv.slice(2);
  // vitest dispatches on a leading subcommand (`vitest run …`, `vitest watch …`). turbo-test is
  // always a single run, so accept-and-strip a leading `run`/`watch`/`dev` token rather than
  // letting it reach the runner as a phantom test-file path.
  if (argv.length && /^(run|watch|dev)$/.test(argv[0])) argv = argv.slice(1);
  // `--passWithNoTests`: vitest exits 0 (not 1) when no test files match. Handled here in the
  // launcher (the discover-empty branch below); not forwarded to the native binary.
  const passWithNoTests = argv.includes('--passWithNoTests');
  argv = argv.filter((a) => a !== '--passWithNoTests');
  // split flags (start with -) from file/glob args
  const flags = [];
  const files = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('-')) {
      flags.push(a);
      // flags that take a value: --jobs N, --shard i/n, --reporter X, -t <pattern>
      if (/^(-j|--jobs|--shard|--reporter|--reporters|--outputFile|--output-file|-t|--testNamePattern|--coverage-dir|--coverage-thresholds|--coverage-threshold|--coverage-reporter|--coverage-reporters|--coverage-include|--coverage-exclude)$/.test(a) && i + 1 < argv.length && !argv[i + 1].startsWith('-')) {
        flags.push(argv[++i]);
      }
    } else {
      files.push(a);
    }
  }
  // When coverage is on, fill thresholds/include/exclude from vitest config unless the user
  // passed them explicitly — flags win over config (P1/P2).
  if (flags.some((f) => f.startsWith('--coverage'))) {
    const cov = vitestCoverage(process.cwd());
    if (cov) {
      if (cov.thresholds && !flags.includes('--coverage-thresholds') && !flags.includes('--coverage-threshold')) {
        flags.push('--coverage-thresholds', cov.thresholds);
      }
      if (cov.include.length && !flags.includes('--coverage-include')) {
        flags.push('--coverage-include', cov.include.join(','));
      }
      if (cov.exclude.length && !flags.includes('--coverage-exclude')) {
        flags.push('--coverage-exclude', cov.exclude.join(','));
      }
    }
  }

  let testFiles = files;
  if (testFiles.length === 0) {
    testFiles = discover(process.cwd());
    if (testFiles.length === 0) {
      if (passWithNoTests) {
        console.error('turbo-test: no test files found — exiting 0 (--passWithNoTests).');
        process.exit(0);
      }
      console.error('turbo-test: no test files found (looked for *.test.* / *.spec.*).');
      process.exit(1);
    }
  }
  // Drop file args that no longer exist (deleted/renamed since a caller built its list — e.g.
  // a `git diff`/staged-files wrapper). A stale path would otherwise reach the runner as a
  // hard load-error and flip the exit code, breaking `set -e` wrappers. Warn, don't fail.
  {
    const missing = testFiles.filter((f) => !fs.existsSync(f));
    if (missing.length) {
      console.error(`turbo-test: skipping ${missing.length} missing file(s): ${missing.join(', ')}`);
      testFiles = testFiles.filter((f) => fs.existsSync(f));
    }
    if (testFiles.length === 0) {
      console.error('turbo-test: no existing test files to run.');
      process.exit(0); // nothing to run is not a failure
    }
  }
  const res = spawnSync(bin, [...flags, ...testFiles], { stdio: 'inherit' });
  if (res.error) { console.error('turbo-test:', res.error.message); process.exit(1); }
  process.exit(res.status == null ? 1 : res.status);
}

main();
