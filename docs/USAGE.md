# Using turbo-test in a repo

turbo-test is a **drop-in replacement for `vitest run`** — a native Rust binary (V8 + oxc, no Node
process) that runs your existing vitest test files, reads your existing `vitest.config.*`, and
prints vitest-shaped output. You don't rewrite tests; you swap the runner.

## Install & run

```jsonc
// package.json
{
  "scripts": {
    "test": "turbo-test"          // was: "vitest run"
  },
  "devDependencies": {
    "@miaskiewicz/turbo-test": "^0.2.x"
  }
}
```

```bash
npm test                          # discovers + runs every *.test.* / *.spec.*
npx turbo-test run src/foo.test.ts   # a single file (leading `run`/`watch`/`dev` is accepted & stripped)
npx turbo-test --jobs 12 --coverage  # parallelism + native V8 coverage
```

No file args → vitest-style discovery (honors your config `test.include`/`exclude`). The npm package
ships a prebuilt binary per platform; `cli.js` is a thin shim that execs it.

## How it consumes vitest

turbo-test is **vitest-config- and vitest-API-compatible by reading the same inputs**, not by
importing vitest:

1. **Config** — walks up for the nearest `vitest.config.*` / `vite.config.*` and **string-scans**
   (no TS eval) `test.include`/`exclude` (discovery), `test.environment` (node/jsdom/happy-dom),
   and the `coverage.*` block (include/exclude/thresholds). `-c/--config <path>` forces an exact
   file. Dynamic config (functions, plugins, `defineConfig` logic, aliases) is invisible — see gaps.
2. **Globals are always on** — `describe`/`it`/`expect`/`vi` are injected; you don't import them
   (importing from `'vitest'` also works as a no-op shim). `--globals`/`--no-globals` are accepted
   no-ops.
3. **The DOM env** — `test.environment: jsdom | happy-dom` both map onto turbo-dom; `node` skips the
   DOM. A per-file `// @vitest-environment <env>` docblock overrides it.
4. **Output** — `--reporter json` emits a vitest-ish summary (`numTotalTests`, `numPassedTests`,
   per-file `testResults`); `junit`/`tap`/`verbose`/`dot`/`default` also supported.

## What's compatible (vitest surface)

Full matrix + quirks: `vitest.compat.md`. Summary:

**CLI flags** ✅ `<file globs>`, `-j/--jobs` (≈ `--maxWorkers`), `--shard i/n`, `--reporter`
(json/junit/tap/verbose/dot/default) + `--outputFile`, `-t/--testNamePattern`, `--testTimeout`,
`--retry`, `--bail`, `-c/--config`, `--root`/`--dir`, `--environment`, `--isolate`/`--no-isolate`,
`--changed [since]`, `--passWithNoTests`, `-u/--update`, the `--coverage*` family. Leading
`run`/`watch`/`dev` accepted-and-stripped. Unknown flags warned + ignored (never treated as files).

**Test / `vi` / `expect` API** ✅
- `describe`/`it`/`test` + `.skip`/`.only`/`.todo`/`.each`/`.skipIf`/`.runIf`/`.concurrent`
  (concurrent runs sequentially within a file), `it.fails`.
- hooks `beforeAll`/`afterAll`/`beforeEach`/`afterEach`; per-test `{ timeout }`/`{ retry }`.
- `expect` + a large matcher set, `expect.extend`, `.soft`, asymmetric matchers,
  `expect.assertions`/`hasAssertions` (enforced), `toMatchObject`/`toContainEqual`/`toSatisfy`/
  `toHaveBeenCalledOnce`/`toHaveBeenNthCalledWith`/…
- `expect(...).toMatchSnapshot()` → `__snapshots__/<file>.snap`, `-u` to update.
- `vi.fn`/`spyOn`/`mock`/`unmock`/`doMock`/`mocked`/`hoisted`/`waitFor`/`waitUntil`,
  full fake-timers (`useFakeTimers` + advance/run/clear + `setSystemTime`), `stubGlobal`/`stubEnv`.
- `jest.*` alias shim.

**Coverage** ✅ native V8 line/function + derived statement/branch coverage; lcov / json-summary /
text / html reporters; thresholds gate; include/exclude globs (auto-read from config).

## Known gaps (🟡 / ❌)

- `--reporter` non-default values map to text except json/junit/tap/verbose/dot; vitest's **dotted**
  `--coverage.*` flag form is not parsed (use the `--coverage-*` namespace).
- `toMatchInlineSnapshot` / `toThrowErrorMatchingInlineSnapshot`: **compare-only**, no auto-write —
  pass the expected string explicitly.
- `it.extend` fixtures: best-effort (no per-fixture teardown / `{ auto }` / `TestContext` extras).
- `bench()`: not implemented.
- Config read is a regex scan: dynamic/`defineConfig`-computed config, plugins, aliases beyond
  tsconfig `paths`, and most `test.*` non-discovery options are not read from config.

## How it works (one paragraph)

Native binary embeds V8. Each test file is transformed **TS/JSX → CJS with oxc** (app files + a
native package bundler for node_modules — esbuild only as a fallback / for coverage), loaded into a
worker isolate booted from a shared framework snapshot, run across N workers with work-stealing.
The test harness (`describe`/`it`/`expect`/timers/`vi`) is JS baked into the binary. DOM via
turbo-dom. Typically ~5–6× faster than vitest-on-jsdom.
