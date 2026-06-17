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
| `--bail <n>` | 🟡 | **P1 — DONE** | Stop after N **failed tests** total. `turbo_test.rs`: shared `Arc<AtomicUsize>` failure counter incremented after each file; workers stop pulling new files once it reaches N. **File-granular**: a worker already mid-file finishes that file, so the final failed count can exceed N (esp. with multiple workers); partial results are still reported. `cli.js` forwards the value. |
| `--reporter junit` / `--outputFile` | ✅ | **P1 — DONE** | JUnit XML (per-testcase) + `--outputFile` for json/junit/tap. See §3. |
| `--reporter` (verbose/dot/tap/default) | ✅ | **P1/P2 — DONE** | `tap`/`verbose`/`dot`/`default` implemented; unknown values (`html`, `tap-flat`, …) accepted-and-ignored → text fallback. See §3. |
| `-c, --config <path>` | ❌ | P1 | Config path is auto-discovered (nearest `vitest/vite.config.*`); cannot override. |
| `--root <path>` / `--dir <path>` | ❌ | P2 | No root override; discovery is `cwd`-rooted. |
| `--environment <node\|jsdom\|happy-dom>` | 🟡 | P1 | Env is effectively fixed (turbo-dom DOM globals always installed). Not selectable per-run; `// @vitest-environment` pragma not honored. |
| `--globals` / `--no-globals` | 🟡 | P2 | Globals are **always on**; cannot disable. `import { describe } from 'vitest'` interop relies on this. |
| `--isolate` / `--no-isolate` | 🟡 | P1 | Controlled by `TURBO_REUSE_ISOLATE`/`TURBO_NO_REUSE` env + config `isolate:false` autodetect — **no CLI flag**. |
| `--pool <threads\|forks\|vmThreads>` | ⏸ | — | turbo-test has its own native worker model; flag is meaningless but should be accepted-and-ignored. |
| `--maxWorkers` / `--minWorkers` | ✅ | **P2 — DONE** | `--maxWorkers N` aliases `--jobs` (`turbo_test.rs`). `--minWorkers` is accepted-and-ignored — turbo-test has no minimum-worker concept (work-stealing scales down naturally). `cli.js` forwards both values. |
| `--maxConcurrency <n>` | ❌ | P3 | Within-file concurrency; turbo-test runs a file's tests sequentially anyway. |
| `-u, --update` | ✅ | **P1 — DONE** | Snapshot update. `turbo_test.rs` → `TURBO_UPDATE_SNAPSHOTS` env → `globalThis.__TT_UPDATE_SNAPSHOTS`; `toMatchSnapshot` writes missing/changed keys instead of failing. Forwarded as a boolean flag by `cli.js`. Inline-snapshot auto-write is **not** supported (see §4). |
| `--retry <n>` | ✅ | **P2 — DONE** | Global default retry. `turbo_test.rs` → `TURBO_TEST_RETRY` env → `__TT_DEFAULT_RETRY` global → `runtime.js` `runSuite` (per-test `{ retry }` still wins). `cli.js` forwards the value. |
| `--silent` | ✅ | **P2 — DONE** | `turbo_test.rs` → `TURBO_TEST_SILENT` → `__TT_SILENT` global; `runtime.js` console.log/info/warn/error become no-ops (checked at call time, since the global is injected after the runtime module is snapshot-evaluated). |
| `--changed [since]` | ❌ | P2 | Run only tests affected by git changes. Logic exists in `m5-affected`, unwired. |
| `--allowOnly` / `--no-allowOnly` | 🟡 | **P3 — DONE** | Both flags accepted (`turbo_test.rs`). Default allows `.only`. `--no-allowOnly` → `TURBO_TEST_FORBID_ONLY` → `__TT_FORBID_ONLY`; `runtime.js` `__tt.run()` records a failure (flipping the exit code) for any **file** that collected a `.only`. **Partial**: per-file granularity — the failure is attributed to the file, and the file's `.only` tests still execute (vitest collect-time errors before running); the run exits non-zero with a clear message, which is the CI-relevant behavior. |
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
| default text (`PASS/FAIL file (n passed, n failed)` + summary line) | ✅ | turbo-test's own format, not byte-identical to vitest's. `--reporter default` selects it. |
| `--reporter json` | 🟡 | Emits `{numTotalTests,numPassedTests,…,testResults[]}` (file-level, not per-assertion). To **stdout** (summary line moved to stderr to keep stdout clean) or to `--outputFile`. `testResults[]` is per-FILE counts, not per-test `assertionResults[]` like vitest. **Where:** `src/bin/turbo_test.rs` `Reporter::Json`. |
| `--reporter junit` + `--outputFile` | ✅ | JUnit XML: `<testsuites tests= failures= errors=>` → one `<testsuite>` per file → one `<testcase name="describe > it" classname=file time=>` per test, with `<failure message=…/>` on failures. Names/messages XML-escaped. A file that fails to LOAD emits one synthetic `<testcase>` with `<error>`. Skipped tests omitted from `<testcase>` list. **Where:** `src/bin/turbo_test.rs` `Reporter::Junit`; per-test list from `TestReport.tests` (`runner.rs`) ← `summary.tests[]` (`runtime.js runSuite`). **Gap:** no `<system-out>`/`<properties>`; `time` is wall-clock `Date.now()` (ms-granular, can read `0.000` for sub-ms tests; affected by `vi.setSystemTime`). |
| `--reporter tap` | ✅ | TAP v13: `TAP version 13`, flat `1..N` plan over all tests, `ok N - name` / `not ok N - name`, `# SKIP` directive, YAML `message:` diagnostic block on failure. **Where:** `Reporter::Tap`. **Gap:** flat (no nested subtests); message newlines flattened to spaces. |
| `--reporter verbose` | ✅ | Per-file PASS/FAIL line + one `✓/✗/- name (Nms)` line per test. To stdout. **Where:** `Reporter::Verbose`. |
| `--reporter dot` | ✅ | One char per FILE (`.` pass / `x` fail / `!` load-error), not per-test (vitest is per-test). **Where:** `Reporter::Dot`. Documented divergence. |
| `--reporter html` / `tap-flat` / `hanging-process` / `basic` / unknown | 🟡 | Accepted-and-ignored → falls back to the default text reporter, never errors. |
| `--outputFile[.<reporter>] <path>` | 🟡 | `--outputFile <path>` writes the active artifact reporter (json/junit/tap) to disk; for text/dot/verbose it writes the plain PASS/FAIL lines. The vitest per-reporter dotted form `--outputFile.junit=…` (multi-reporter fan-out) is **not** parsed. **Where:** `src/bin/turbo_test.rs` `--outputFile` arm; `cli.js` value-flag regex. |
| dotted flags (`--coverage.reporter`, `--reporter.0`) | ❌ | turbo-test uses flat `--coverage-*`; vitest's dotted form unrecognized. |

---

## 4. Test / `vi` / `expect` API surface

Mostly strong. Notable gaps:

| API | status | Notes |
|---|---|---|
| `describe` / `.skip` / `.only` / `.each` | ✅ | `.each` template-name function form supported. |
| `describe.todo` / `.skipIf` / `.runIf` / `.concurrent` | ✅ | All four added (`runtime.js`). `todo` registers nothing; `skipIf`/`runIf` register the whole block only when the condition allows; `concurrent` is an accepted alias (tests still run **sequentially** within a file). |
| `it`/`test`, `.skip`/`.only`/`.todo`/`.each`/`.skipIf`/`.runIf` | ✅ | |
| `it.concurrent` | 🟡 | Accepted as alias; tests still run **sequentially** within a file. |
| `it.fails` | ✅ | Outcome inverted in `runSuite` (a throwing body passes; a clean body fails). Honors per-test `{ retry }`. |
| `it.extend` (fixtures) | 🟡 | Best-effort. Fixtures (plain values + `async ({deps}, use) => use(v)` functions) are resolved and passed as the test fn's first-arg context. `.skip/.only/.each/.extend` chain off the returned test. No per-fixture teardown after `use()`, no `{ auto }`/scoped fixtures, no `TestContext` extras (`task`, `expect`, `onTestFinished`, `annotate`). |
| `{ timeout }` per-test / `--testTimeout` | ✅ | **ENFORCED.** `runtime.js` `runSuite` races `t.fn()` against an INTERNAL one-shot timer (separate from the user/fake-timer queue → invisible to `vi.runAllTimers`/`getTimerCount`) that rejects with `test timed out in <ms>ms`. Per-test `{ timeout }` (and the numeric 3rd-arg form) wins; else `--testTimeout` (`TURBO_TEST_TIMEOUT` → `__TT_DEFAULT_TIMEOUT`); else vitest's 5000ms. A genuinely hung async test (`await new Promise(()=>{})`) now fails cleanly instead of hanging the worker — the drive loop advances the virtual clock to the internal timeout. |
| `{ retry }` per-test | ✅ | Honored in `runSuite`. |
| hooks `beforeAll/afterAll/beforeEach/afterEach` | ✅ | Throwing hooks recorded as failures, run settles. |
| `expect` + core matchers | ✅ | Large matcher set, `expect.extend`, `.soft`, asymmetric matchers, `expect.not`. |
| `expect.assertions(n)` / `expect.hasAssertions()` | ✅ | Enforced in `runSuite` after the test body. A per-test counter increments on each `expect(...)` / `expect.soft(...)` call (the `.assertions`/`.hasAssertions` calls themselves don't count); a mismatch / zero fails the test. Reset per test (and per retry attempt). |
| `expect(...).toMatchSnapshot()` | ✅ | File snapshots under `__snapshots__/<testfile>.snap`, keyed `<full describe>it name> <counter>`. Missing key or update-mode writes + passes; else compares with a readable diff. Pretty-format-ish serializer (primitives / arrays / objects (sorted keys) / Map / Set / Date / RegExp / Error / functions). Driven by `-u`/`--update` → `TURBO_UPDATE_SNAPSHOTS` env → `globalThis.__TT_UPDATE_SNAPSHOTS`. The test file path reaches the runtime as `globalThis.__ttFile` (set in `drive_tests`). |
| `expect(...).toMatchInlineSnapshot()` | 🟡 | **Compare path only**: comparing against the passed string (whitespace-normalized) works; with no arg it's a no-op pass. **AUTO-WRITING the inline snapshot back into the source on first run / `-u` is UNSUPPORTED** (no test-source rewriting) — pass the expected string explicitly. |
| `expect(...).toThrowErrorMatchingSnapshot()` / `toThrowErrorMatchingInlineSnapshot()` | ✅ / 🟡 | The thrown error's `message` is snapshotted via the same file path (✅) / inline compare path (🟡, no auto-write). |
| `vi.fn/spyOn/mock/unmock/doMock/mocked` | ✅ | |
| `vi.useFakeTimers` + advance/run/clear family, `setSystemTime` | ✅ | Full fake-timer set incl. async variants. |
| `vi.stubGlobal/stubEnv` + `unstubAllGlobals/unstubAllEnvs` | ✅ | |
| `vi.hoisted` | ✅ | Shared between mock-prepass & module. |
| `vi.waitFor/waitUntil` | ✅ | |
| `jest.*` alias | ✅ | Compatibility shim object. |
| `toMatchObject` / `toContainEqual` / `toSatisfy` / `toHaveBeenCalledOnce` / `toHaveBeenNthCalledWith` | ✅ | Present (verified by `test/compat-api.test.mjs`). |
| `bench()` | ❌ | No benchmark API. |

---

## 5. Config-file reading (`cli.js`)

| capability | status | Notes |
|---|---|---|
| auto-discover nearest `vitest.config.*` / `vite.config.*` | ✅ | walks up from cwd. |
| `test.include` / `test.exclude` globs | ✅ | string-scan (no TS eval); drives discovery. |
| `coverage.include/exclude/thresholds` | ✅ | string-scan; flags win over config. |
| anything requiring evaluating the config (functions, `defineConfig` logic, env interpolation, plugins, `setupFiles` array beyond first, aliases) | 🟡/❌ | Pure regex scan — dynamic config is invisible. |
| `test.environment`, `test.globals`, `test.testTimeout`, `test.retry`, `test.bail`, `test.pool`, `test.setupFiles`, `test.reporters` | ❌ | Not read from config (only include/exclude/coverage are). |

---

## 6. Prioritized backlog (fix order)

**P0 (drop-in `vitest run` for CI) — ✅ SHIPPED 2026-06-17:**
1. ✅ `-t, --testNamePattern <re>` — name filter.
2. ✅ `run`/`watch`/`dev` subcommand accepted as no-op + unknown-`--flag` ignore (no phantom-file load errors).
3. ✅ `--run`, `--pool`, `--silent`, etc. accepted-and-ignored (no-op pass-through via the unknown-flag arm).
4. ✅ `--passWithNoTests` — exit 0 on empty match.

All four are locked by `test/cli-compat.test.mjs` (`npm test`).

**P1 — ✅ ALL SHIPPED 2026-06-17** (across reporters / api / config-env / runflags batches):
- ✅ test `{ timeout }` enforcement + `--testTimeout`; ✅ `--bail <n>` (file-granular); ✅ `--retry`;
  ✅ `--silent`; ✅ `--allowOnly`/`--no-allowOnly` (partial, see §2); ✅ `--maxWorkers`/`--minWorkers`.
- ✅ `--reporter junit/tap/verbose/dot/default` + `--outputFile`.
- ✅ `-c/--config`; ✅ `--root`/`--dir`; ✅ `--isolate`/`--no-isolate`; ✅ `--changed`; ✅ `--environment`
  selection (`node` skips DOM); ✅ `--globals` accepted (`--no-globals` no-op, see §2).
- ✅ snapshots (`toMatchSnapshot` + `-u`); ✅ `expect.assertions`/`hasAssertions` enforcement;
  ✅ `it.fails`; ✅ `describe.todo/.skipIf/.runIf/.concurrent`.

**P2/P3:** see per-row priorities in §2/§4.

---

## Changelog of this file
- 2026-06-17 — Initial audit against v0.2.15. Matrix + P0 backlog established.
- 2026-06-17 — **P0 batch shipped**: `-t/--testNamePattern`, `run`/`watch`/`dev` subcommand strip,
  unknown-flag ignore, `--passWithNoTests`. Added `test/cli-compat.test.mjs` (the previously-missing
  `test/` dir that `npm test` expects). Next up: P1 (`--bail`, test `{ timeout }` enforcement,
  `--reporter junit` + `--outputFile`, `-c/--config`, snapshots).
- 2026-06-17 — **Reporters batch shipped**: `--reporter junit` (per-testcase XML), `tap` (TAP v13),
  `verbose`, `dot`, `default`, `json`-to-file via `--outputFile`. Unknown reporter values fall back
  to text (never error). Extended `TestReport`/`summary` with a per-test list
  (`tests[] = {name,status,duration_ms,message}`: `runtime.js runSuite` → `runner.rs` parse →
  `turbo_test.rs` reporters) — passing test names are now retained. Added
  `test/compat-reporters.test.mjs` + `fixtures/compat/mixed.test.ts` (pass+fail mix) and the
  `fixtures/compat/empty/` dir the existing `--passWithNoTests` test needs. `cli.js` value-flag
  regex now forwards `--outputFile`. **Gaps:** dotted per-reporter `--outputFile.junit=` fan-out
  unsupported; `dot` is per-file not per-test; durations are `Date.now()` ms-granular.
- 2026-06-17 — **test/expect API compat batch shipped** (§4): file snapshots (`toMatchSnapshot`,
  `toThrowErrorMatchingSnapshot`) + `-u/--update`; `toMatchInlineSnapshot`/`…InlineSnapshot`
  compare-only (no source auto-write); `expect.assertions(n)`/`hasAssertions()` enforcement;
  `it.fails`; `describe.todo/.skipIf/.runIf/.concurrent`; `it.extend` fixtures (best-effort,
  no teardown/scoped fixtures). Common matchers (`toMatchObject`, `toContainEqual`, `toSatisfy`,
  `toHaveBeenCalledOnce`, `toHaveBeenNthCalledWith`) confirmed present. Added
  `test/compat-api.test.mjs` + `fixtures/compat/*`. Snapshot file path plumbed via
  `globalThis.__ttFile` (set in `drive_tests`).
- 2026-06-17 — **config/environment batch shipped** (§2/§5): `-c/--config`, `--root`/`--dir`,
  `--isolate`/`--no-isolate` (→ `TURBO_NO_REUSE`/`TURBO_REUSE_ISOLATE`), `--changed [since]`
  (git diff ∩ discovered files), `--environment` + `// @vitest-environment` pragma (`node` skips
  DOM-global install via `TURBO_ENV`; jsdom/happy-dom not distinguished), `--globals` accepted
  (`--no-globals` no-op — globals always injected). Added `test/compat-config-env.test.mjs` +
  Rust pragma unit tests.
- 2026-06-17 — **Execution-control batch shipped**: `--testTimeout` + per-test `{ timeout }`
  ENFORCEMENT (internal-timer race, invisible to fake timers; hung tests fail cleanly), `--retry`
  global default, `--bail <n>` (file-granular cross-worker abort), `--maxWorkers` alias + `--minWorkers`
  no-op, `--silent` (call-time console no-op), `--allowOnly`/`--no-allowOnly` (per-file `.only` gate).
  Added `test/compat-runflags.test.mjs` (10 tests) + `fixtures/compat/{timeout,per-test-timeout,retry}.test.ts`
  and `fixtures/compat/{bail,silent,only}/`. cli.js value-flag regex extended for the new value flags.
