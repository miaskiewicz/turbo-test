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
| `-c, --config <path>` | ❌ | P1 | Config path is auto-discovered (nearest `vitest/vite.config.*`); cannot override. |
| `--root <path>` / `--dir <path>` | ❌ | P2 | No root override; discovery is `cwd`-rooted. |
| `--environment <node\|jsdom\|happy-dom>` | 🟡 | P1 | Env is effectively fixed (turbo-dom DOM globals always installed). Not selectable per-run; `// @vitest-environment` pragma not honored. |
| `--globals` / `--no-globals` | 🟡 | P2 | Globals are **always on**; cannot disable. `import { describe } from 'vitest'` interop relies on this. |
| `--isolate` / `--no-isolate` | 🟡 | P1 | Controlled by `TURBO_REUSE_ISOLATE`/`TURBO_NO_REUSE` env + config `isolate:false` autodetect — **no CLI flag**. |
| `--pool <threads\|forks\|vmThreads>` | ⏸ | — | turbo-test has its own native worker model; flag is meaningless but should be accepted-and-ignored. |
| `--maxWorkers` / `--minWorkers` | 🟡 | P2 | Map `--maxWorkers` → `--jobs`. No min. |
| `--maxConcurrency <n>` | ❌ | P3 | Within-file concurrency; turbo-test runs a file's tests sequentially anyway. |
| `-u, --update` | ❌ | P1 | Snapshot update. Blocked on snapshot support (see §4). |
| `--retry <n>` | ❌ | P2 | Global retry. Per-test `{ retry }` option **is** honored; no CLI/global form. |
| `--silent` | ❌ | P2 | Suppress test `console.*` output. |
| `--changed [since]` | ❌ | P2 | Run only tests affected by git changes. Logic exists in `m5-affected`, unwired. |
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

**P1 (next):**
- `--bail <n>`; test `{ timeout }` enforcement + `--testTimeout`; `--reporter junit` + `--outputFile`;
  `-c/--config`; `--environment` selection; snapshots (`toMatchSnapshot` + `-u`); `--maxWorkers` alias.

**P2/P3:** see per-row priorities in §2/§4.

---

## Changelog of this file
- 2026-06-17 — Initial audit against v0.2.15. Matrix + P0 backlog established.
- 2026-06-17 — **P0 batch shipped**: `-t/--testNamePattern`, `run`/`watch`/`dev` subcommand strip,
  unknown-flag ignore, `--passWithNoTests`. Added `test/cli-compat.test.mjs` (the previously-missing
  `test/` dir that `npm test` expects). Next up: P1 (`--bail`, test `{ timeout }` enforcement,
  `--reporter junit` + `--outputFile`, `-c/--config`, snapshots).
</content>
</invoke>
