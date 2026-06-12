# turbo-test

**A blazing-fast native test runner — a drop-in replacement for [vitest](https://vitest.dev).**

Written in Rust on V8: per-file transforms via [oxc](https://oxc.rs)/esbuild, a native
[turbo-dom](https://www.npmjs.com/package/@miaskiewicz/turbo-dom) DOM, work-stealing parallelism,
and an optional isolate-reuse mode. Runs your existing `*.test.ts(x)` files — same `describe`/`it`/
`expect`/`vi`, same `@testing-library/react` + jest-dom — typically **2.5–5× faster**.

```bash
npm i -D @miaskiewicz/turbo-test
npx turbo-test            # discovers + runs every *.test.* / *.spec.* under cwd
npx turbo-test src/foo.test.ts --jobs 8 --reporter json
```

## Benchmarks

Real app suites, same machine (Apple M-series, 8 workers), identical pass counts:

| Suite | Files / tests | vitest | turbo-test | Speedup |
|---|---|---|---|---|
| **payroll-app** | 1001 files / 10,006 tests | 99.7s | **39.5s** (fresh) | **2.5×** |
| **ui-design-components** | 386 files / 6,189 tests | ~130s | **90s** (fresh) | ~1.4× |
| **ui-design-components** | 386 files / 6,189 tests | ~130s | **33s** (isolate-reuse) | **~4×** |

Both suites pass **100%** under turbo-test (10006/0 and 6189/0). vitest's own breakdown on
payroll shows where the time goes — `setup 206s + import 433s` cumulative across workers — which
turbo-test's native transform + dep-bundling collapses.

## Isolate-reuse (extra speed, zero config)

By default turbo-test isolates every file (like vitest's `isolate: true`). If your vitest config
sets `isolate: false`, turbo-test honors it and **reuses one V8 isolate per worker** — node_modules
barrels evaluate once instead of per file (~4× on `ui-design-components`). Any file that fails under
reuse is automatically re-run on a clean isolate, so correctness always matches isolated mode.

Force it on/off regardless of config:

```bash
TURBO_REUSE_ISOLATE=1 npx turbo-test   # force reuse on
TURBO_NO_REUSE=1       npx turbo-test   # force fresh isolation
```

## How it works

- **Transform** — per-module oxc/esbuild → CJS; node_modules bundled once per package (Vite-style),
  cached on disk (99% hit on warm runs).
- **DOM** — [turbo-dom](https://www.npmjs.com/package/@miaskiewicz/turbo-dom) via a Node-API host
  (native html5ever parser + lazy copy-on-write DOM). Per-file env setup ~1–2 ms.
- **Mocks** — `vi.mock` (sync + async factories), hoisting, `vi.spyOn`, `vi.fn`, `vi.hoisted`,
  `importActual`, jest-dom matchers — all supported.
- **Parallelism** — work-stealing across `--jobs` workers; duration-aware slowest-first ordering.

## CLI

```
turbo-test [files...] [--jobs N] [--shard i/n] [--reporter json]
```

No file args → discovers `**/*.{test,spec}.{ts,tsx,js,jsx,mts,cts}` (skipping `node_modules`,
`dist`, `build`, etc.). Flags pass through to the native binary.

## Programmatic

```js
const { run } = require('@miaskiewicz/turbo-test');
const { status } = run(['src/a.test.ts'], { jobs: 8, env: { TURBO_REUSE_ISOLATE: '1' } });
process.exit(status);
```

## Config

turbo-test reads your project's `vitest.config.ts` for `setupFiles`, `environment`, and `isolate`.
No separate config needed for most suites.

## Compatibility notes

- A handful of vitest features that depend on a full Node runtime are stubbed/approximated; e2e
  helper files that import heavy Node-only packages (e.g. `@playwright/test`) get those deps
  stubbed so the file still runs.
- `Date.now()` reflects a real base + the virtual event-loop clock, so `setTimeout`-driven elapsed
  assertions pass deterministically.

## Requirements

`node >= 18`. Prebuilt binaries ship for macOS (arm64/x64), Linux (x64/arm64-gnu), and Windows x64.
Other platforms: build from source with a Rust toolchain (`npm run build`).

## License

MIT
