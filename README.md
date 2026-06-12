# turbo-test

**A blazing-fast native test runner — a drop-in replacement for [vitest](https://vitest.dev).**

Written in Rust on V8: per-file transforms via [oxc](https://oxc.rs)/esbuild, a native
[turbo-dom](https://www.npmjs.com/package/@miaskiewicz/turbo-dom) DOM, work-stealing parallelism,
and an optional isolate-reuse mode. Runs your existing `*.test.ts(x)` files — same `describe`/`it`/
`expect`/`vi`, same `@testing-library/react` + jest-dom — typically **~6× faster than vitest+jsdom**.

```bash
npm i -D @miaskiewicz/turbo-test
npx turbo-test            # discovers + runs every *.test.* / *.spec.* under cwd
npx turbo-test src/foo.test.ts --jobs 8 --reporter json
```

## Benchmarks

Two real production app suites, **same machine, same session, identical pass counts** (Apple
M-series, 10 workers). To keep it apples-to-apples we measured vitest two ways — with stock
**jsdom** (the default most projects run) and with **turbo-dom** swapped in as the environment —
so you can see the DOM's contribution separately from the runner's:

| Suite | Tests | vitest + jsdom | vitest + turbo-dom | **turbo-test** | vs jsdom | vs turbo-dom |
|---|---|---|---|---|---|---|
| **payroll-app** | 10,006 | 296s | 130s | **51s** | **5.8×** | 2.6× |
| **ui-design-components** | 6,189 | 428s | 358s | **76s** | **5.6×** | 4.7× |

All three configs pass 100% (10006/0 and 6189/0). Two takeaways:

- **The DOM matters.** Just swapping jsdom → turbo-dom under plain vitest already cuts wall time
  ~1.2–2.3× (jsdom's `environment` setup alone was **1228s cumulative** across workers on payroll
  vs turbo-dom's **96s**). If you can't switch runners yet, switching the environment is free speed.
- **The runner matters more.** turbo-test's native transform + per-package dep-bundling + V8
  worker pool collapses vitest's `setup + import` cost (hundreds of seconds cumulative) and lands
  **~6× faster than the jsdom baseline** — with zero config changes.

> Numbers are from a busy long-uptime workstation, so absolute seconds run high; the **ratios**
> are what travel. An `isolate: false` reuse mode exists for extra headroom (see below) but on this
> loaded box it landed even with fresh isolation (~80s on ui-design), so fresh is the headline.

## Isolate-reuse (extra speed, zero config)

By default turbo-test isolates every file (like vitest's `isolate: true`). If your vitest config
sets `isolate: false`, turbo-test honors it and **reuses one V8 isolate per worker** — node_modules
barrels evaluate once instead of per file. Any file that fails under reuse is automatically re-run
on a clean isolate, so correctness always matches isolated mode (still 6189/0). How much it buys
depends on how barrel-heavy your imports are and how loaded the machine is — on a quiet box it can
beat fresh isolation handily; under heavy contention it converges with it.

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
