# vitest.compat.md — vitest CLI/command compatibility audit & tracker

Living tracker of how closely **turbo-test** matches the `vitest` command-line surface and the
test/`vi` API surface. Goal: be a true drop-in for `vitest run` in CI.

- Status legend: ✅ supported · 🟡 partial / quirks · ❌ missing · 🔜 in progress · ⏸ won't-do (out of scope)
- Audit date: 2026-06-17 (against turbo-test v0.2.15). Update the matrix + the **Changelog of this file**
  section below whenever a gap is closed.

Sources of truth in this repo:
- CLI arg parser: `src/bin/turbo_test.rs` (`main`, the `while let Some(a) = args.next()` match).
- Launcher / config reading / file discovery: `cli.js`.
- Test + `vi`/`expect` API + runner loop: `src/runtime.js`.
- CLI-level compat tests: `test/*.mjs` (run via `npm test` → `node --test`).

---

## 1. Subcommands

vitest dispatches on a leading subcommand (`vitest run`, `vitest watch`, …). turbo-test has **no
subcommand layer** — `argv[0]` that isn't a flag is treated as a test-file path.

| vitest subcommand | turbo-test | Notes |
|---|---|---|
| `vitest` (watch in TTY, run in CI) | 🟡 | Always single-run. No watch. CI-equivalent only. |
| `vitest run` | 🟡→🔜 | The canonical CI invocation. `run` was being parsed as a phantom test-file path (warned & skipped). Being made an accepted no-op token. |
| `vitest watch` / `vitest dev` | ❌ | No file watcher. Out of scope for a CI runner; revisit later. |
| `vitest related <files>` | 🟡 | A separate `m5-affected` binary exists (`src/bin/m5_affected.rs`, `--changed`) but is **not wired into the main CLI**. |
| `vitest bench` | ❌ | No benchmark API (`bench()`), no reporter. |
| `vitest list` | ❌ | No "collect-only, print test names" mode. |
| `vitest init` | ❌ | No scaffolding. ⏸ low value. |
| `vitest typecheck` | ❌ | No `tsc`/`vue-tsc` typecheck pass. ⏸ out of scope. |

---

## 2. CLI options / flags

### Supported today
| flag | turbo-test | Notes |
|---|---|---|
| `<file glob…>` positional | ✅ | Falls back to vitest-style discovery + config `include`/`exclude` when none given (`cli.js`). |
| `-j, --jobs N` | ✅ | turbo-test's worker count. ≈ vitest `--maxWorkers` (no separate min). Also `TURBO_JOBS` env. |
| `--shard i/n` | ✅ | Deterministic index partition. Matches vitest's `--shard`. |
| `--reporter <r>` | 🟡 | **Only `json` is honored** (terse vitest-ish JSON). Any other value → default text reporter. See §3. |
| `--coverage` | ✅ | Native V8 coverage. |
| `--coverage-dir DIR` | ✅ | lcov output dir (implies `--coverage`). |
| `--coverage-thresholds k=v,…` / `--coverage-threshold` | ✅ | `lines,functions,branches,statements`. Auto-read from config by `cli.js`. |
| `--coverage-per-file` | ✅ | Per-file threshold gate. |
| `--coverage-reporter[s] a,b` | ✅ | `lcov,json-summary,text,html`. |
| `--coverage-include GLOB` / `--coverage-exclude GLOB` | ✅ | Brace-alternation; auto-read from config. |

> Note: turbo-test's coverage flags use a `--coverage-*` namespace, **not** vitest's dotted
> `--coverage.*` form (`--coverage.reporter`, `--coverage.thresholds.lines`). A vitest invocation
> using dotted coverage flags would not be understood. See §3 dotted-flag gap.

### Missing / partial — prioritized
| flag | status | Priority | Notes |
|---|---|---|---|
| `-t, --testNamePattern <re>` | ✅ | **P0 — DONE** | Filter tests by name (regex, unanchored, case-sensitive). `turbo_test.rs` → `TURBO_TEST_NAME_PATTERN` env → `__TT_NAME_PATTERN` global → `runtime.js` `runSuite` filter. `cli.js` forwards the value. |
| `--run` | ✅ | **P0 — DONE** | `run`/`watch`/`dev` leading subcommand token stripped in `cli.js`; `--run` (and other unknown flags) accepted-and-ignored. |
| `--passWithNoTests` | ✅ | **P0 — DONE** | `cli.js`: exit 0 instead of 1 when discovery finds no files. |
| `--bail <n>` | ❌ | P1 | Stop the run after N failures. Needs cross-worker abort. |
| `--reporter junit` / `--outputFile` | ❌ | P1 | JUnit XML is the standard CI artifact. No file output of any reporter today. |
| `--reporter` (verbose/dot/tap/tap-flat/html/default) | ❌ | P2 | Only `json` recognized. |
| `-c, --config <path>` | ✅ | **P1 — DONE** | Forces that exact config file for include/exclude/coverage/`environment` reading instead of the walk-up search. `cli.js` `findConfig`/`vitestPatterns`/`vitestCoverage` take a `forced` arg; the file's dir becomes the discovery root for relative globs. Still a string-scan (no TS eval — same limits as auto-discovery). |
| `--root <path>` / `--dir <path>` | ✅ | **P2 — DONE** | Override the directory `discover()` walks (default `cwd`). turbo-test scans the project root for files, so `--root` and `--dir` are equivalent (last wins); vitest's finer root-vs-dir distinction is not modeled. |
| `--environment <node\|jsdom\|happy-dom>` | 🟡 | **P1 — mostly DONE** | `cli.js` maps the flag (and config `test.environment`) → `TURBO_ENV`, read in `runner.rs` `needs_dom`/`forced_env`. `node` SKIPS the turbo-dom DOM-global install; `jsdom`/`happy-dom` both FORCE it on (turbo-dom is the single DOM impl — jsdom vs happy-dom are NOT distinguished). Per-file `// @vitest-environment <env>` pragma is honored and OVERRIDES the run-level env. **Gap:** `node` only *skips* DOM globals; it does not strip any DOM API that may already be present from a reused isolate under `--no-isolate` (in practice DOM install is per-worker-once and gated by the same `needs_dom`, so a node file in a node-typed run never sees one). |
| `--globals` / `--no-globals` | 🟡 | **P2 — accepted, gap documented** | Both spellings accepted (`cli.js`). Globals (`describe`/`it`/`expect`) are **always on** and injected unconditionally; `--no-globals` is a **no-op** — it CANNOT be honored because there is no `vitest` module-export shim to `import { describe } from 'vitest'` from. Honoring it would require shipping such a shim. |
| `--isolate` / `--no-isolate` | ✅ | **P1 — DONE** | CLI flag added: `cli.js` sets `--no-isolate` → `TURBO_REUSE_ISOLATE=1` (reuse one isolate per worker), `--isolate` → `TURBO_NO_REUSE=1` (force fresh) in the env before spawning the binary (which inherits it). Config `isolate:false` autodetect still applies when neither flag is passed. |
| `--pool <threads\|forks\|vmThreads>` | ⏸ | — | turbo-test has its own native worker model; flag is meaningless but should be accepted-and-ignored. |
| `--maxWorkers` / `--minWorkers` | 🟡 | P2 | Map `--maxWorkers` → `--jobs`. No min. |
| `--maxConcurrency <n>` | ❌ | P3 | Within-file concurrency; turbo-test runs a file's tests sequentially anyway. |
| `-u, --update` | ❌ | P1 | Snapshot update. Blocked on snapshot support (see §4). |
| `--retry <n>` | ❌ | P2 | Global retry. Per-test `{ retry }` option **is** honored; no CLI/global form. |
| `--silent` | ❌ | P2 | Suppress test `console.*` output. |
| `--changed [since]` | 🟡 | **P2 — DONE (direct filter)** | `cli.js` `gitChanged()`: `git diff --name-only [since]` + `--cached` + untracked (`ls-files --others`), intersected with discovered test files (absolute paths). `since` arg is optional (working-tree vs HEAD/index by default). When nothing changed → exit 0 (running nothing is not a failure). When git is unavailable / not a repo → runs all. **Gap:** this is a direct changed-*file* filter, NOT an affected-graph — a test that imports a changed *source* file but is itself unchanged is NOT re-run (no import graph built; the `m5-affected` graph idea is still unwired). |
| `--allowOnly` / `--no-allowOnly` | 🟡 | P3 | `.only` always allowed; vitest CI default forbids it. No flag to error on stray `.only`. |
| `--watch` / `-w` | ❌ | ⏸ | No watcher. |
| `--ui` | ❌ | ⏸ | No browser UI. |
| `--browser` | ❌ | ⏸ | No browser-mode. |
| `--inspect` / `--inspect-brk` | ❌ | P3 | No debugger bridge. |
| `--mode <mode>` | ❌ | P3 | Vite mode / `.env` selection. |
| `--sequence.shuffle` / `--sequence.seed` | ❌ | P3 | Order is duration-aware (slowest-first), not shuffleable. |
| `--logHeapUsage` | ❌ | P3 | |
| `--no-color` / `FORCE_COLOR` | 🟡 | P3 | Output is plain text already; no explicit color control. |
| `--project <name>` | ❌ | P3 | No workspace/projects support. |
| `--hideSkippedTests` / `--printConsoleTrace` / `--clearScreen` | ❌ | P3 | Minor output knobs. |
| `--version` / `-v`, `--help` / `-h` | ❌ | P2 | No version/help output; unknown flags currently mis-handled (see below). |

### Robustness gap — unknown flags become phantom files — ✅ FIXED
Previously `src/bin/turbo_test.rs`'s `_ =>` arm did `files.push(PathBuf::from(a))` for **any**
unrecognized token — including `--something` — so an unmodeled vitest flag (e.g. `--silent`,
`--pool=forks`) reached the runner as a hard **load-error** and flipped the exit code. Now a new
`other if other.starts_with('-')` arm warns + ignores unknown flags. **Known limitation:** an
unknown flag with a *space-separated* value (`--pool forks`) still leaves `forks` as a positional —
`cli.js`'s missing-file filter drops it (no exit-code flip), but prefer the `=` form (`--pool=forks`)
for unmodeled flags. Covered by `test/cli-compat.test.mjs`.

---

## 3. Reporters & output

| capability | status | Notes |
|---|---|---|
| default text (`PASS/FAIL file (n passed, n failed)` + summary line) | ✅ | turbo-test's own format, not byte-identical to vitest's. |
| `--reporter json` | 🟡 | Emits `{numTotalTests,numPassedTests,…,testResults[]}` to **stdout** (vitest's JSON is richer + per-assertion). No `--outputFile`. |
| `--reporter junit` | ❌ | P1 — standard CI artifact. |
| verbose / dot / tap / tap-flat / html / hanging-process | ❌ | P2/P3. |
| `--outputFile[.<reporter>] <path>` | ❌ | P1 — needed to write any reporter to disk. |
| dotted flags (`--coverage.reporter`, `--reporter.0`) | ❌ | turbo-test uses flat `--coverage-*`; vitest's dotted form unrecognized. |

---

## 4. Test / `vi` / `expect` API surface

Mostly strong. Notable gaps:

| API | status | Notes |
|---|---|---|
| `describe` / `.skip` / `.only` / `.each` | ✅ | `.each` template-name function form supported. |
| `describe.todo` / `.skipIf` / `.runIf` / `.concurrent` | 🟡 | `it.*` has these; `describe.*` is missing `todo/skipIf/runIf/concurrent`. |
| `it`/`test`, `.skip`/`.only`/`.todo`/`.each`/`.skipIf`/`.runIf` | ✅ | |
| `it.concurrent` | 🟡 | Accepted as alias; tests still run **sequentially** within a file. |
| `it.fails` | ❌ | "expected to fail" inversion not implemented. |
| `it.extend` (fixtures) | ❌ | Test-context fixtures unsupported. |
| `{ timeout }` per-test / `--testTimeout` | ❌ | **Parsed but NOT enforced** (`runtime.js:1441` "no per-file timeout gate yet"). A hung test hangs the worker. P1. |
| `{ retry }` per-test | ✅ | Honored in `runSuite`. |
| hooks `beforeAll/afterAll/beforeEach/afterEach` | ✅ | Throwing hooks recorded as failures, run settles. |
| `expect` + core matchers | ✅ | Large matcher set, `expect.extend`, `.soft`, asymmetric matchers, `expect.not`. |
| `expect.assertions(n)` / `expect.hasAssertions()` | 🟡 | **No-ops** (`runtime.js:1128-1129`) — never enforce the assertion count. P2. |
| `expect(...).toMatchSnapshot()` / `toMatchInlineSnapshot()` | ❌ | **No snapshot support** at all → blocks `-u/--update`. P1. |
| `expect(...).toThrowErrorMatchingSnapshot()` | ❌ | Same. |
| `vi.fn/spyOn/mock/unmock/doMock/mocked` | ✅ | |
| `vi.useFakeTimers` + advance/run/clear family, `setSystemTime` | ✅ | Full fake-timer set incl. async variants. |
| `vi.stubGlobal/stubEnv` + `unstubAllGlobals/unstubAllEnvs` | ✅ | |
| `vi.hoisted` | ✅ | Shared between mock-prepass & module. |
| `vi.waitFor/waitUntil` | ✅ | |
| `jest.*` alias | ✅ | Compatibility shim object. |
| `bench()` / `expect().toMatchObject` etc. | — | bench ❌; verify individual matchers ad-hoc. |

---

## 5. Config-file reading (`cli.js`)

| capability | status | Notes |
|---|---|---|
| auto-discover nearest `vitest.config.*` / `vite.config.*` | ✅ | walks up from cwd. `-c/--config <path>` forces an exact file (skips the walk-up; its dir becomes the discovery root). |
| `test.include` / `test.exclude` globs | ✅ | string-scan (no TS eval); drives discovery. |
| `coverage.include/exclude/thresholds` | ✅ | string-scan; flags win over config. |
| `test.environment` | ✅ | string-scan; sets the run default env when `--environment` is absent → `TURBO_ENV`. Per-file pragma still wins. |
| anything requiring evaluating the config (functions, `defineConfig` logic, env interpolation, plugins, `setupFiles` array beyond first, aliases) | 🟡/❌ | Pure regex scan — dynamic config is invisible. |
| `test.globals`, `test.testTimeout`, `test.retry`, `test.bail`, `test.pool`, `test.setupFiles`, `test.reporters` | ❌ | Not read from config. (`test.isolate:false` IS autodetected separately in `runner.rs`.) |

---

## 6. Prioritized backlog (fix order)

**P0 (drop-in `vitest run` for CI) — ✅ SHIPPED 2026-06-17:**
1. ✅ `-t, --testNamePattern <re>` — name filter.
2. ✅ `run`/`watch`/`dev` subcommand accepted as no-op + unknown-`--flag` ignore (no phantom-file load errors).
3. ✅ `--run`, `--pool`, `--silent`, etc. accepted-and-ignored (no-op pass-through via the unknown-flag arm).
4. ✅ `--passWithNoTests` — exit 0 on empty match.

All four are locked by `test/cli-compat.test.mjs` (`npm test`).

**P1 (next):**
- `--bail <n>`; test `{ timeout }` enforcement + `--testTimeout`; `--reporter junit` + `--outputFile`;
  snapshots (`toMatchSnapshot` + `-u`); `--maxWorkers` alias.
- ✅ DONE this round: `-c/--config`, `--root`/`--dir`, `--environment` selection + `// @vitest-environment`
  pragma, `--isolate`/`--no-isolate`, `--changed [since]` (direct file filter), `--globals`/`--no-globals`
  (accepted; `--no-globals` no-op — documented).

**P2/P3:** see per-row priorities in §2/§4.

---

## Changelog of this file
- 2026-06-17 — Initial audit against v0.2.15. Matrix + P0 backlog established.
- 2026-06-17 — **P0 batch shipped**: `-t/--testNamePattern`, `run`/`watch`/`dev` subcommand strip,
  unknown-flag ignore, `--passWithNoTests`. Added `test/cli-compat.test.mjs` (the previously-missing
  `test/` dir that `npm test` expects). Next up: P1 (`--bail`, test `{ timeout }` enforcement,
  `--reporter junit` + `--outputFile`, `-c/--config`, snapshots).
- 2026-06-17 — **config / discovery / environment batch shipped**: `-c/--config` (force exact
  config), `--root`/`--dir` (discovery root override), `--environment <node|jsdom|happy-dom>` +
  per-file `// @vitest-environment` pragma (→ `TURBO_ENV`, gates turbo-dom DOM install in
  `runner.rs`), `--isolate`/`--no-isolate` (→ `TURBO_NO_REUSE`/`TURBO_REUSE_ISOLATE` env),
  `--changed [since]` (direct git changed-file filter, no import graph), `--globals`/`--no-globals`
  (accepted; `--no-globals` is a documented no-op). `test.environment` now read from config as the
  env default. `cli.js` value-flag handling extended (incl. optional-arg `--changed` and `--k=v`
  inline form). Tests: `test/compat-config-env.test.mjs` (13 cases) + 4 Rust unit tests for the
  pragma parser (`runner::env_pragma_tests`). Gaps: jsdom/happy-dom not distinguished (both →
  turbo-dom); `--no-globals` can't be honored (no `vitest` export shim); `--changed` is a file
  filter, not an affected-graph.
</content>
</invoke>
