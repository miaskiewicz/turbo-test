# Layer B — Lazy Module Stubs (`TURBO_LAZY_STUBS`)

## Problem

turbo-test boots a **fresh V8 isolate per test file** (`run_test_file`, `runner.rs`). Transform
output is disk-cached (99% hit), but **JS evaluation is not** — every file that imports a heavy
barrel re-evaluates it from scratch in its cold isolate.

`@mui/icons-material` is the worst case: its entry re-exports ~2,000 single-icon modules. In
`ui-design-components`, 110 of 386 test files import it. With `--packages=external` (every bare
import in a dep bundle becomes a `require()`), each of those files pays the full barrel eval.

Measured: removing that per-file eval dropped the suite from **~98s → ~35s** (2.8×), same
6186/2 pass count.

## Idea

A "barrel" like `@mui/icons-material` is **registry-shaped**: hundreds of exports, each a
uniform display component (a `forwardRef` SVG icon), and any given test touches a handful. So
don't resolve / transform / evaluate the real package at all — return a **lazy Proxy namespace**
whose every property mints a stub icon component on first access and caches it.

This is the turbo-dom thesis (don't build what you don't touch) applied to the **module layer**
instead of the DOM.

Config is an env var of bare specifiers, so the runner has **zero hardcoded library knowledge**:

```bash
TURBO_LAZY_STUBS='@mui/icons-material,lucide-react' turbo-test <files...>
```

Safe **only** for display-component barrels (every export is a component). NOT `@mui/material`
(exports `styled` / `useTheme` / `alpha` / `Box` — stubbing those breaks hundreds of tests).

## Implementation (all in `src/runner.rs`)

### 1. Registry field

`Registry` gains a per-isolate cache of built stub namespaces, keyed by the matched specifier:

```rust
lazy_stub_ns: HashMap<String, v8::Global<v8::Value>>,
```

Cleared in both `clear_registry()` and (via `mem::take`) `forget_registry()`.

### 2. Config + matcher

```rust
fn lazy_stub_specs() -> &'static Vec<String>   // OnceLock, parsed from TURBO_LAZY_STUBS (comma-sep)

fn lazy_stub_match(spec: &str) -> Option<(String, Option<String>)>
//  spec == "@mui/icons-material"        -> (spec, None)            // named-import barrel
//  spec == "@mui/icons-material/Add"    -> (spec, Some("Add"))     // subpath: default = Add icon
```

### 3. Stub factory (JS source built in Rust)

`lazy_stub_factory_src(default_js)` returns a self-invoking function that:

- requires the **test's** React via `globalThis.__nativeRequire(globalThis.__ttDir, 'react', false)`
  — React 19's element symbols can't be hand-rolled, so real `React.forwardRef`/`createElement`
  are mandatory;
- builds a `Proxy({}, …)` whose `get(t, k)`:
  - `k === '__esModule'` → `false` (lets esbuild's `__toESM` add `default`),
  - `k === 'default'` → cached stub named `default_js` (subpath basename) or `'default'`,
  - any other string key → cached `make(k)` — a `forwardRef` rendering
    `<svg class="MuiSvgIcon-root" data-testid="${k}Icon" data-icon="${k}" …>` (mirrors the proven
    `setup-optimized.ts` icon mock shape);
- `has` → `true` for everything; `ownKeys` → `[]`; `getOwnPropertyDescriptor` → `undefined`.

```rust
fn ensure_lazy_stub<'s>(scope, cache_key, default_name) -> Option<v8::Local<'s, v8::Value>>
//  returns the cached Global if present; else compiles+runs the factory, stores the Global.
//  Returns None on failure -> caller falls through to a real load (graceful degradation).
```

### 4. Interception points

Two hooks, both **before** the real resolve/transform:

- **`native_require`** — after the `vitest`/node-builtin short-circuits, before `resolve_spec_as`:
  ```rust
  if let Some((key, def)) = lazy_stub_match(&spec) {
      if let Some(ns) = ensure_lazy_stub(scope, &key, def.as_deref()) { rv.set(ns); return; }
  }
  ```
- **`dynamic_import_callback`** — same check, resolves the promise with the stub namespace.

In module-runner mode (default) test files + dep bundles are CJS, so app/dep imports of the
barrel become `require()` → `native_require` covers them. `dynamic_import_callback` covers
`await import('@mui/icons-material')`.

### Why named imports stay lazy

turbo-test overrides esbuild's `__toESM` to return the **same** object (identity, no prop copy)
— see `postprocess_mr_cjs` in `runner.rs`. So `import { Add } from '@mui/icons-material'` compiles
to `import_x.Add`, which reads the Proxy `get` trap directly. No `ownKeys` enumeration needed →
laziness preserved, no icon list to maintain.

## Correctness boundary

- Only barrels where **every export is a display component**. Adding `@mui/material` would break
  `styled`/`useTheme`/`alpha`/`Box`/etc.
- `recharts` / `react-simple-maps` are *technically* component barrels but tests assert specific
  structure (a `responsive-container` div, a `geography` path) — the generic SVG-icon stub would
  break them. Leave them real (only ~5 files) or give them a tailored strategy.

## Results (`ui-design-components`, 386 files, 8 workers)

| Config | Barrels | Wall | Pass |
|---|---|---|---|
| turbo-test baseline | real (per-file eval) | ~98s | 6186/2 |
| **turbo-test + `TURBO_LAZY_STUBS='@mui/icons-material'`** | stubbed | **~35s** | **6186/2** |
| vitest optimized (`setup-optimized.ts`) | mocked | ~130s | 6188/0 |

Same 2 failures (`Card.test.tsx`, a `@vitest-environment jsdom` CSS-cascade gap — unrelated),
zero new failures, deterministic across runs.

## Relationship to the other layers

- **Layer A** (`ui-design-components/src/test/lazyExports.ts`): the userland equivalent for the
  **vitest** path — a generic `lazyExports(build)` Proxy used in `setup-optimized.ts`. Cleanup
  win (no maintained icon list), speed-neutral for vitest (its `vi.mock` already skips the barrel).
- **Isolate-reuse** (`TURBO_REUSE_ISOLATE`, WIP): the zero-config alternative — keep the real
  barrels but evaluate them once per worker instead of per file. Supersedes Layer B if it reaches
  parity, because it needs no config and speeds up **all** heavy deps (incl. `@mui/material`),
  not just icons.
