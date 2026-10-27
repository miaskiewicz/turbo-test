# vitest.compat.md — vitest CLI/command compatibility audit & tracker

Living tracker of how closely **turbo-test** matches the `vitest` command-line surface and the
test/`vi` API surface. Goal: be a true drop-in for `vitest run` in CI.

- Status legend: ✅ supported · 🟡 partial / quirks · ❌ missing · 🔜 in progress · ⏸ won't-do (out of scope)
- Audit date: 2026-06-17 (against turbo-test v0.2.15). Update the matrix + the **Changelog of this file**
  section below whenever a gap is closed.

Sources of truth in this repo:
- CLI arg parser: `src/cli.rs` (`parse_args`, the `while let Some(a) = args.next()` match; `src/cli.rs` is a thin wrapper).
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
| `<filter…>` positional | ✅ | Filters, as vitest's: a discovered test file (vitest-style discovery + config `include`/`exclude`) runs when its root-relative path contains one of the arguments, case-insensitively (a file path, a directory, or any substring: `b` runs `a/b/y.test.ts`; an absolute path matches as a prefix). An argument that matches nothing is no warning; when nothing matches at all (or nothing is discovered): vitest's `No test files found, exiting with code 1` (0 with `--passWithNoTests`) on stderr, with its `filter:` / `include:` / `exclude:` lines. A file outside `include` does not run even when named. Files run in discovery order, named relative to the working directory (the JSON report names them by absolute path, as vitest's). With no argument, every discovered file runs. |
| `-j, --jobs N` | ✅ | turbo-test's worker count. ≈ vitest `--maxWorkers` (no separate min). Also `TURBO_JOBS` env. |
| `--shard i/n` | ✅ | Deterministic index partition. Matches vitest's `--shard`. |
| `--reporter <r>` | 🟡 | **Only `json` is honored** (terse vitest-ish JSON). Any other value → default text reporter. See §3. |
| `--coverage` | ✅ | Native V8 coverage. |
| `--coverage-dir DIR` | ✅ | lcov output dir (implies `--coverage`). |
| `--coverage-thresholds k=v,…` / `--coverage-threshold` | ✅ | `lines,functions,branches,statements`. Auto-read from config by `cli.js`. |
| `--coverage-per-file` | ✅ | Per-file threshold gate. |
| `--coverage-reporter[s] a,b` | ✅ | `lcov,json-summary,text,html`. |
| `--coverage-include GLOB` / `--coverage-exclude GLOB` | ✅ | Brace-alternation; auto-read from config. |

> Every value flag accepts both the space form and vitest's inline form (`--reporter json` ≡
> `--reporter=json`, `--testTimeout=5000`, `--shard=1/2`, `--outputFile=out/r.xml`, …; the value is
> everything after the first `=`). A value flag given WITHOUT a value (the last token, or followed
> by another `-` token) is dropped with a `turbo-test: ignoring '<flag>': missing value` warning
> (a value-less `--coverage-*` still implies `--coverage`); it never swallows a test-file path.
> A value that itself starts with `-` needs the inline form (`--testNamePattern=-x`).

> Note: turbo-test's coverage flags use a `--coverage-*` namespace, **not** vitest's dotted
> `--coverage.*` form (`--coverage.reporter`, `--coverage.thresholds.lines`). A vitest invocation
> using dotted coverage flags would not be understood. See §3 dotted-flag gap.

### Missing / partial — prioritized
| flag | status | Priority | Notes |
|---|---|---|---|
| `-t, --testNamePattern <re>` | ✅ | **P0 — DONE** | Filter tests by name (regex, unanchored, case-sensitive), matched against the full name: the `describe` names and the test's joined with a space, as vitest's `getTaskFullName`. `cli.rs` → `TURBO_TEST_NAME_PATTERN` env → `__TT_NAME_PATTERN` global → `runtime.js` `runSuite` filter. `cli.js` forwards the value. |
| `--run` | ✅ | **P0 — DONE** | `run`/`watch`/`dev` leading subcommand token stripped in `cli.js`; `--run` (and other unknown flags) accepted-and-ignored. |
| `--passWithNoTests` | ✅ | **P0 — DONE** | Launcher: exit 0 instead of 1 when discovery finds no files. Like vitest, it also covers a test file that collects NO test or suite at all: without the flag such a file fails the run with `No test suite found in file <abs path>` (reported as a file-level error: `ERROR` line, JSON `"error"`, a failing JUnit case, TAP `not ok`; exit 1); with it the file passes. A file whose tests are all skipped/todo (incl. `describe.skip`/`describe.todo`/`skipIf`) or filtered out by `-t` is NOT empty and passes. vitest's per-suite `No test found in suite <name>` (an empty `describe`) is not modeled. |
| `--bail <n>` | 🟡 | **P1 — DONE** | Stop after N **failed tests** total. `cli.rs`: shared `Arc<AtomicUsize>` failure counter incremented after each file; workers stop pulling new files once it reaches N. **File-granular**: a worker already mid-file finishes that file, so the final failed count can exceed N (esp. with multiple workers); partial results are still reported. `cli.js` forwards the value. |
| `--reporter junit` / `--outputFile` | ✅ | **P1 — DONE** | vitest's JUnit report (turbo-test#81) + `--outputFile` for json/junit/tap. See §3. |
| `--reporter` (verbose/dot/tap/default) | ✅ | **P1/P2 — DONE** | `default` / `verbose` are vitest 4.1.12's (turbo-test#94), the agent reporter without `--reporter` under an AI agent; `dot` is vitest's DotReporter (turbo-test#100); `tap` implemented; unknown values (`html`, `tap-flat`, …) accepted-and-ignored → the default reporter. See §3. |
| `-c, --config <path>` | ✅ | **P1 — DONE** | Forces that exact config file for include/exclude/coverage/`environment` reading instead of the walk-up search. `cli.js` `findConfig`/`vitestPatterns`/`vitestCoverage` take a `forced` arg; the file's dir becomes the discovery root for relative globs. Still a string-scan (no TS eval — same limits as auto-discovery). |
| `--root <path>` / `--dir <path>` | ✅ | **P2 — DONE** | Override the directory `discover()` walks (default `cwd`; last wins). `--root` is also vitest's root: files are named relative to it in every reporter (`TURBO_ROOT` → `cli.rs` `display_names`; the JSON report keeps absolute names), while `--dir` keeps working-directory names. The config is still looked up from the discovery directory. |
| `--environment <node\|jsdom\|happy-dom>` | 🟡 | **P1 — mostly DONE** | `cli.js` maps the flag (and config `test.environment`) → `TURBO_ENV`, read in `runner.rs` `needs_dom`/`forced_env`. `node` SKIPS the turbo-dom DOM-global install; `jsdom`/`happy-dom` both FORCE it on (turbo-dom is the single DOM impl — jsdom vs happy-dom are NOT distinguished). Per-file `// @vitest-environment <env>` pragma is honored and OVERRIDES the run-level env. |
| `--globals` / `--no-globals` | 🟡 | **P2 — accepted, gap documented** | Both spellings accepted (`cli.js`). Globals (`describe`/`it`/`expect`) are **always on** and injected unconditionally; `--no-globals` is a **no-op** — it CANNOT be honored because there is no `vitest` module-export shim to `import { describe } from 'vitest'` from. Honoring it would require shipping such a shim. |
| `--isolate` / `--no-isolate` | ✅ | **P1 — DONE** | CLI flag added: `cli.js` sets `--no-isolate` → `TURBO_REUSE_ISOLATE=1` (reuse one isolate per worker), `--isolate` → `TURBO_NO_REUSE=1` (force fresh) in the env before spawning the binary (which inherits it). Config `isolate:false` autodetect still applies when neither flag is passed. |
| `--pool <threads\|forks\|vmThreads>` | ⏸ | — | turbo-test has its own native worker model; flag is meaningless but should be accepted-and-ignored. |
| `--maxWorkers` / `--minWorkers` | ✅ | **P2 — DONE** | `--maxWorkers N` aliases `--jobs` (`cli.rs`). `--minWorkers` is accepted-and-ignored — turbo-test has no minimum-worker concept (work-stealing scales down naturally). `cli.js` forwards both values. |
| `--maxConcurrency <n>` | ❌ | P3 | Within-file concurrency; turbo-test runs a file's tests sequentially anyway. |
| `-u, --update` | ✅ | **P1 — DONE** | Snapshot update mode `all` (vitest's): changed and missing snapshots are written, obsolete ones removed and an emptied snapshot file deleted. Without it the mode is `new` (missing snapshots written), on CI `none`: a missing snapshot fails. CI is std-env's `isCI` (`CI` non-empty, `false` included, or the first matching row of its provider table, `src/ci_providers.json`; `src/ci.rs`, checked by `scripts/check-ci-detection.mjs`). `cli.rs` → `TURBO_UPDATE_SNAPSHOTS` → `__TT_UPDATE_SNAPSHOTS`; CI → `__TT_CI`. |
| `--retry <n>` | ✅ | **P2 — DONE** | Global default retry. `cli.rs` → `TURBO_TEST_RETRY` env → `__TT_DEFAULT_RETRY` global → `runtime.js` `runSuite` (per-test `{ retry }` still wins). `cli.js` forwards the value. |
| `--includeTaskLocation` | ✅ | **DONE** | `cli.rs` → `TURBO_TASK_LOCATION` (also set by the launcher from config `test.includeTaskLocation: true`) → `__TT_TASK_LOCATION`: each test records the original `{ line, column }` of the call that registered it (`location.rs`), written by `--reporter json`. |
| `--silent` | ✅ | **P2 — DONE** | `cli.rs` → `TURBO_TEST_SILENT` → `__TT_SILENT` global; `runtime.js` console.log/info/warn/error become no-ops (checked at call time, since the global is injected after the runtime module is snapshot-evaluated). |
| `--changed [since]` | 🟡 | **P2 — DONE (direct filter)** | `cli.js` `gitChanged()`: `git diff --name-only [since]` + `--cached` + untracked (`ls-files --others`), intersected with discovered test files (absolute paths). `since` arg is optional (working-tree vs HEAD/index by default). When nothing changed → exit 0 (running nothing is not a failure). When git is unavailable / not a repo → runs all. **Gap:** this is a direct changed-*file* filter, NOT an affected-graph — a test that imports a changed *source* file but is itself unchanged is NOT re-run (no import graph built; the `m5-affected` graph idea is still unwired). |
| `--allowOnly` / `--no-allowOnly` | ✅ | **P3 — DONE** | Both flags accepted (`cli.rs`, the last one wins). Default `!isCI` as vitest (`src/ci.rs`): `.only` is allowed locally and forbidden on CI (turbo-test#75). allowOnly off → `TURBO_TEST_FORBID_ONLY` → `__TT_FORBID_ONLY`; `runtime.js` `__interpretOnly` fails each `.only` task in place as vitest's `checkAllowOnly` (turbo-test#76): a `.only` test is a failed test that does not run (no `beforeEach`/`afterEach`, no `duration`), a `describe.only` a failed suite whose hooks do not run and whose tests are skipped; nothing inside `describe.skip`/`describe.todo` is checked. Message: `[Vitest] Unexpected .only modifier. Remove it or pass --allowOnly argument to bypass this error`. |
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
Previously the CLI arg loop's (then `src/cli.rs`, now `src/cli.rs`) `_ =>` arm did `files.push(PathBuf::from(a))` for **any**
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
| default (`--reporter default`, or no `--reporter`) | ✅ | vitest 4.1.12's DefaultReporter as it prints to a pipe without colours, byte for byte but for the timings (turbo-test#94, `tests/cli_human.rs` against vitest's output): stdout has the ` RUN  v4.1.12 <root>` banner, each console entry as a `stdout \| file > task` block, a module line per file (` ✓ a.test.ts (2 tests) 3ms`, ` ❯ … (7 tests \| 3 failed \| 1 skipped \| 1 todo)`, ` ↓` for a file none of whose tests ran), a failed file's tests indented by their `describe` depth, a test slower than 300ms on a line of its own even in a passing file, `(retry xN)`, and the footer: Snapshots (written / failed / updated / files obsolete or removed / obsolete or removed keys with `↳ ./file` and `· key`), `Test Files` / `Tests` (vitest's `getStateString`: `failed \| passed \| expected fail \| skipped \| todo (total)` without the zero parts, `no tests`, turbo-test#78), `Start at` (local time) and `Duration … (transform 0ms, setup 0ms, import …, tests …, environment …)`. stderr has the `stderr \| …` blocks and the Failed Suites / Failed Tests sections: ` FAIL  file > test` (a load error `file [ file ]`), the error with vite's `  Plugin:` / `  File: <module>[:line:col]` / code frame lines for a module that does not parse or transform (turbo-test#113), its diff, frames mapped to the source and the code frame of the nearest one, identical errors merged under their tasks, `[n/total]` dividers. **Agent:** without `--reporter`, when std-env's `isAgent` holds (`AI_AGENT`, `CLAUDECODE`, `CLAUDE_CODE`, `REPL_ID`, `GEMINI_CLI`, `CODEX_SANDBOX`, `CODEX_THREAD_ID`, `OPENCODE`, `AUGMENT_AGENT`, `GOOSE_PROVIDER`, `JUNIE_DATA`, `JUNIE_SHIM_PATH`, `COPILOT_AGENT`, `COPILOT_CLI`, `CURSOR_AGENT`, a `PATH` with `.pi/agent`, an `EDITOR` with `devin`, Kiro off a terminal; `src/agent.rs`), vitest's agent reporter: failed files and their failed tests only, failed tasks' console output only, each when its task finishes. **Where:** `src/human.rs`; console entries from `runtime.js` `__ttLog`. **Colours:** as vitest's — on unless `NO_COLOR` / `--no-color`, `TERM=dumb` or `FORCE_TTY=false` (without `FORCE_COLOR` / `CI` / `--color`), or an AI agent; tinyrainbow's codes (`src/colors.rs`), code frames highlighted as tinyhighlight does (`src/highlight.rs`, js-tokens), console values coloured as Node's `util.inspect`, diffs coloured (the changed part of a string inverse; `tests/data/human-color`). **Differences:** an `Error` logged to the console keeps its stack uncoloured (Node grays its internal / node_modules frames); the `transform` / `setup` / `environment` parts of the breakdown are turbo-test's own measures; a thrown `Error` with an empty message prints `Error: ` where vitest dumps its serialized error object with its parsed frames (a thrown primitive is `Unknown Error: <value>`, a value without a message its serialized form, as vitest, turbo-test#97); `--slowTestThreshold`, `--hideSkippedTests` and `--logHeapUsage` are not read; a module's parse / transform error has no frames, where vitest lists vite's and rolldown's own (`transformWithOxc node_modules/vite/dist/…`) and a blank line after them; the colours of annotations, skip notes and `Serialized Error` follow vitest's code but are not checked against its coloured output, and a serialized value without a message prints uncoloured; `--outputFile` writes nothing for it (as vitest). `TURBO_LEGACY_REPORTER=1` brings back turbo-test's own `PASS/FAIL file (n passed, n failed)` lines and its `n files \| … \| cache n% hit` line. |
| `--reporter json` | ✅ | vitest 4.1.12's JSON report, key order included: `numTotal/Passed/Failed/PendingTestSuites` (the files and their `describe` blocks), `numTotal/Passed/Failed/Pending/TodoTests`, `snapshot`, `startTime`, `success`, and per file `assertionResults[]` (`ancestorTitles`, `fullName`, `status` passed/failed/skipped/todo, `title`, `duration` for a test that ran, `failureMessages` = each error's stack, else its message, else `null` (turbo-test#97), `location` `{ line, column }` under `--includeTaskLocation` / `test.includeTaskLocation`, `meta`, `tags`), `startTime`/`endTime`, `status`, `message` (a load error's message as vitest gives it: a static import that does not resolve is `Cannot find module '<as written>' imported from <importer>` / `Cannot find package '<name>' imported from …`; a `.js` / `.mjs` / `.cjs` module that does not parse is vite's error — its import analysis's when es-module-lexer rejects the source, else rolldown's `Parse failure: …` with the code frame of the module as vite's import analysis rewrote it (its imports as `"/<root-relative url>"`, `"/@fs/<path>"` outside the root) and `At file: /<path>:<line>:<column>` (turbo-test#108, #113); a `.ts` / `.tsx` / `.mts` / `.cts` / `.jsx` module oxc does not parse (or whose syntax its semantic check rejects) is vite's `Transform failed with N error(s):` with rolldown's ariadne diagnostics, coloured as vitest gives them even under NO_COLOR (turbo-test#113); other compile errors are V8's SyntaxError) and `name` (the absolute path). Skipped, todo, `-t`-filtered and `describe.skip`/`describe.todo` tests are listed, as in vitest. To **stdout** (summary line AND test `console.*` output moved to stderr to keep stdout clean — same for tap printed to stdout; junit keeps the output in the report) or to `--outputFile`. **Differences:** `meta` is always `{}`; durations are whole milliseconds; `failureMessages` stacks carry turbo-test's frames; `location` is the registering call's V8 position mapped back through the transform's source map (native emitter / oxc) — a test file that took an esbuild fallback (decorator metadata, `--coverage`) has none. **Where:** `src/cli.rs` `render_json`; per-test records from `runtime.js` `__testRecord`. |
| `--reporter junit` + `--outputFile` | ✅ | vitest 4.1.12's JUnit report byte for byte but for the timings (turbo-test#81): `<testsuites name="vitest tests" tests failures errors time>`, a `<testsuite name timestamp hostname tests failures errors skipped time>` per file, a `<testcase classname name time>` per test — skipped / todo with `<skipped/>`, the test's console output as `<system-out>` / `<system-err>` (captured, not printed), a failure as `<failure message type>` (each attribute left out when the error has no message / name: a thrown primitive has no `type`) holding the error as vitest prints it (`Name: message` — `Unknown Error: <value>` for a thrown primitive, the serialized value `{ a: 1, stacks: [] }` for one without a message —, the expected / actual diff, ` ❯ [fn ]file:line:col` frames outside node_modules mapped back to the source of the test file and of every app module, the error's own properties as `Serialized Error: { … }` under a divider, `Caused by:`, turbo-test#97) — then a case per suite whose `beforeAll` / `afterAll` threw; a file that failed to load is one failing case named after the file. Four-space indentation, vitest's escaping. With `--outputFile` stdout reads `JUNIT report written to <path>` (json: `JSON report written to <path>`) and no footer. **Where:** `src/cli.rs` `render_junit`, `src/failure.rs`; errors / logs from `runtime.js` `__errorInfo` / `__ttLog`. Annotations (`ctx.annotate`, turbo-test#96) are `<properties>`. **Differences:** `time` is wall-clock milliseconds; the `classnameTemplate` / `suiteName` / `addFileAttribute` options are not read. |
| `--reporter tap` | ✅ | TAP v13: `TAP version 13`, flat `1..N` plan over all tests, `ok N - name` / `not ok N - name`, `# SKIP` directive, YAML `message:` diagnostic block on failure. A file that fails to LOAD is one `not ok N - <file>` point with the load error as its `message:` (like JUnit's failing case for it). **Where:** `Reporter::Tap`. **Gap:** flat (no nested subtests); message newlines flattened to spaces. |
| `--reporter verbose` | ✅ | vitest 4.1.12's VerboseReporter (turbo-test#94): the default reporter's banner, console blocks, error sections and footer, with a ` ✓ file > describe > test 3ms` line per test (`×` failed, `↓` skipped, `□` todo) and each failure's error messages under it as `   → message` (`undefined` for an error without one), a test's annotations under it, a test skipped at run time with its duration and `[note]` (turbo-test#96); the errors in the Failed Tests section with their properties (`Serialized Error: { … }`, turbo-test#97). **Where:** `src/human.rs`. |
| `--reporter dot` | ✅ | vitest 4.1.12's DotReporter (turbo-test#100): RUN banner, console blocks, one char per test (`·` / `x` / `-`, coloured as vitest), the Failed Suites / Failed Tests sections and the default footer. `TURBO_LEGACY_REPORTER=1` keeps the old one char per file. **Where:** `src/human.rs` `Kind::Dot`. |
| `--reporter html` / `tap-flat` / `hanging-process` / `basic` / unknown | 🟡 | Accepted-and-ignored → falls back to the default reporter, never errors. |
| `--outputFile[.<reporter>] <path>` | 🟡 | `--outputFile <path>` writes the active artifact reporter (json/junit/tap) to disk; the default / verbose / dot reporters write nothing (as vitest's), the legacy reporters (`TURBO_LEGACY_REPORTER=1`) the plain PASS/FAIL lines. Missing parent dirs are created for every reporter; a failed write prints `turbo-test: failed to write --outputFile '<path>': <err>` to stderr and the run exits 1. The vitest per-reporter dotted form `--outputFile.junit=…` (multi-reporter fan-out) is **not** parsed. **Where:** `src/cli.rs` `--outputFile` arm; `cli.js` value-flag regex. |
| dotted flags (`--coverage.reporter`, `--reporter.0`) | ❌ | turbo-test uses flat `--coverage-*`; vitest's dotted form unrecognized. |

---

## 4. Test / `vi` / `expect` API surface

Mostly strong. Notable gaps:

| API | status | Notes |
|---|---|---|
| `describe` / `.skip` / `.only` / `.each` | ✅ | `.each` titles are formatted as vitest's `formatTitle`: printf placeholders (`%s %d %i %f %j %o %O %c %%`), `%#` / `%$`, `$key` / `$a.b` / `$0` attributes and the tagged-template table, with values rendered as vitest's loupe `inspect` / `objDisplay`. A function title is its `name`, as in vitest. A `test.only` inside `describe.skip` / `describe.todo` never runs and is reported `pending`, as vitest leaves it (turbo-test#77). |
| `describe.todo` / `.skipIf` / `.runIf` / `.concurrent` | ✅ | All four added (`runtime.js`). `todo` registers nothing; `skipIf`/`runIf` register the whole block only when the condition allows; `concurrent` is an accepted alias (tests still run **sequentially** within a file). |
| `it`/`test`, `.skip`/`.only`/`.todo`/`.each`/`.for`/`.skipIf`/`.runIf` | ✅ | `.for` (also `describe.for`) passes each row as one argument, then the test context (with the fixtures under `it.extend`); `.each` handlers get the row only. |
| TestContext (turbo-test#96) | ✅ | Every test function, beforeEach and afterEach hook gets vitest's context: `task` (`name`, `id`, `fullName`, `fullTestName`, `mode`, `meta`, `suite` / `file` tasks, `result` with `state` / `retryCount` while it runs, `annotations`), `skip(note?)` / `skip(condition, note?)` (skipped at run time with its note; a caught PendingError still skips), `onTestFinished` / `onTestFailed` (vitest's order; a throw fails the test; registering one inside one throws), `signal` (aborted when the test times out), `expect`, `annotate(message, type?)` (junit `<properties>`, verbose / Failed Tests lines). Calling it is the deprecated `done` callback (throws). Not modelled: `ctx.expect` counts assertions with the global `expect` (vitest keeps a state per context), annotation attachments are kept but not written anywhere, `task.suite.tasks` / `task.file.tasks` list the collected tasks but `task.result` is set only for the running test. |
| `it.concurrent` | 🟡 | Accepted as alias; tests still run **sequentially** within a file. |
| `it.fails` | ✅ | Outcome inverted in `runSuite` (a throwing body passes; a clean body fails). Honors per-test `{ retry }`. |
| `it.extend` (fixtures) | 🟡 | Best-effort. Fixtures (plain values + `async ({deps}, use) => use(v)` functions) are resolved and passed as the test fn's first-arg context. `.skip/.only/.each/.extend` chain off the returned test. No per-fixture teardown after `use()`, no `{ auto }`/scoped fixtures. The fixtures are set on the test's TestContext (the fixture functions get it, with `task`, `skip`, …). |
| `{ timeout }` per-test / `--testTimeout` | ✅ | **ENFORCED.** `runtime.js` `runSuite` races `t.fn()` against an INTERNAL one-shot timer (separate from the user/fake-timer queue → invisible to `vi.runAllTimers`/`getTimerCount`) that rejects with vitest's `Test timed out in <ms>ms.` / `If this is a long-running test, …` error, whose frame is the `it(` call (turbo-test#102). Per-test `{ timeout }` (and the numeric 3rd-arg form) wins; else `--testTimeout` (`TURBO_TEST_TIMEOUT` → `__TT_DEFAULT_TIMEOUT`); else vitest's 5000ms. A genuinely hung async test (`await new Promise(()=>{})`) now fails cleanly instead of hanging the worker — the drive loop advances the virtual clock to the internal timeout. The drive loop never sleeps: it jumps its clock to the next timer, and a test's reported duration adds those jumps to the real time (turbo-test#95), so `await new Promise(r => setTimeout(r, 320))` reports ~320ms as on Node. |
| `{ retry }` per-test | ✅ | Honored in `runSuite`. Every failed attempt's error is kept (json `failureMessages`, junit, the Failed Tests section) and the retries counted (`(retry x1)`), as vitest's `retryCount`. |
| hooks `beforeAll/afterAll/beforeEach/afterEach` | ✅ | Throwing hooks recorded as failures, run settles. As in vitest, a throwing `beforeAll` / `afterAll` fails its suite (and file) without counting as a failed test, and a throwing `beforeAll` skips the suite's tests and nested suites (its `afterAll` still runs). An empty `describe` fails with `No test found in suite <name>`. |
| `expect` + core matchers | ✅ | Large matcher set, `expect.extend`, `.soft`, asymmetric matchers, `expect.not`. `toEqual` / `toStrictEqual` and the matchers built on them compare as vitest's `equals` with each matcher's testers (Dates, RegExps, boxed primitives, Sets / Maps, symbol keys, Errors, typed arrays, circular structures); `toStrictEqual` checks the types as vitest's `typeEquality` (same `constructor` on both sides: a class instance is not a plain object), holes and ArrayBuffer bytes. `toThrow(error / object)` compares the thrown value with the same `equals` + `iterableEquality` (turbo-test#111). Values in messages print as vitest's loupe, a URL as Node's inspect (turbo-test#112). |
| `expect.assertions(n)` / `expect.hasAssertions()` | ✅ | Enforced in `runSuite` after the test body. A per-test counter increments on each `expect(...)` / `expect.soft(...)` call (the `.assertions`/`.hasAssertions` calls themselves don't count); a mismatch / zero fails the test. Reset per test (and per retry attempt). |
| `expect` + core matchers | ✅ | Large matcher set, `expect.extend`, `.soft`, asymmetric matchers, `expect.not`. `toStrictEqual` checks the types as vitest's `typeEquality` (same `constructor` on both sides: a class instance is not a plain object). |
| `expect.assertions(n)` / `expect.hasAssertions()` | ✅ | Enforced in `runSuite` after the test body. A per-test counter increments on each `expect(...)` / `expect.soft(...)` call (the `.assertions`/`.hasAssertions` calls themselves don't count); a mismatch / zero fails the test with vitest's `expected number of assertions to be <n>, but got <m>` / `expected any number of assertion, but got none` (framed where `hasAssertions` was called, turbo-test#102). Reset per test (and per retry attempt). |
| `expect(...).toMatchSnapshot()` / `toMatchFileSnapshot()` | ✅ | vitest 4.1.12's snapshot files byte for byte (turbo-test#73): `__snapshots__/<testfile>.snap` with vitest's header, keys `<describe > test> [> <hint>] <n>` in natural order, values serialized as vitest's snapshot serializer (pretty-format with `printBasicPrototype: false`, `escapeString: false`, `printFunctionName: false`, `escapeRegex: true`, the `[MockFunction]` serializer), extra line breaks around multi-line values, backtick escaping; property matchers (`toMatchSnapshot({ id: expect.any(Number) })`). The file is written once, after the file's tests; obsolete snapshots are reported (`snapshot.unchecked` in the JSON report) and removed by `-u`; skipped tests' snapshots are not obsolete. `toMatchFileSnapshot(path)` compares the value (a string as is) with the whole file. The JSON report's `snapshot` summary counts them as vitest does. |
| `expect(...).toMatchInlineSnapshot()` | ✅ | Compared as vitest compares (indentation stripped, trimmed); a new one (or under `-u` a changed one) is written back into the test file as vitest writes it (template indented like its line), via the call site (`location.rs`). Property matchers and hints supported. |
| `expect(...).toThrowErrorMatchingSnapshot()` / `toThrowErrorMatchingInlineSnapshot()` | ✅ | The thrown value is snapshotted, as in vitest (`[TypeError: boom]`). |
| `vi.fn/spyOn/mock/unmock/doMock/mocked` | ✅ | A factory-less `vi.mock(path)` automocks the real module on both loader paths (vitest's algorithm: functions → `vi.fn()` returning `undefined`, classes keep mocked statics + prototype methods, objects / class instances mocked deeply, arrays emptied, primitives kept). A factory result's `default` key is the mock's default export. A `vi.mock`/`vi.doMock` factory that throws (or rejects) fails the import of its module with `[vitest] There was an error when mocking a module. … Caused by: <error>` (`jest.mock`: the factory's own error); a module the file never imports never runs into it. Module-runner (esbuild project): reading a name the factory did not return throws `[vitest] No "x" export is defined on the "<spec>" mock…` (`jest.mock`: reads `undefined`); on the ESM-graph / legacy-bundle paths an ES import binding cannot throw, so the missing name links and reads `undefined`. A setup file's `vi.mock` is hoisted above the setup file's own imports (module-runner). A hoisted factory runs once per file (also without esbuild, where factories see the file's `vi.hoisted` values and async factories are awaited). A hoisted factory (and what it returns) sees the test file's top-level bindings — `let`/`var`/`const`, function and class declarations, enums, imports — once the file has run (`mock_scope.rs`: the prepass runs inside a scope proxy the file's own binder statement fills); read before that, a binding throws `Cannot access '<name>' before initialization`, as in its temporal dead zone (vitest instead initialises function declarations first). Without esbuild the prepass gets the file's own TypeScript / JSX transform (oxc; its JSX runtime resolves from the test file, `import.meta` is the test file's), the `vi.mock` calls are read from the file's AST, and a prepass that cannot compile fails the file with its SyntaxError rather than leaving the real modules in place. `vi.doMock` then `import()` returns the mock on every loader path; an async factory is awaited first (a later `doMock` / `doUnmock` of the module wins over one still pending). A `?query` specifier (`./x.js?raw`) is a module id of its own, in a setup file too. Specifiers are read with JS string escapes (`vi.mock("./a\"b.js")`). A setup file that throws fails every test file using it. |
| `vi.useFakeTimers` + advance/run/clear family, `setSystemTime` | ✅ | Full fake-timer set incl. async variants. |
| `vi.stubGlobal/stubEnv` + `unstubAllGlobals/unstubAllEnvs` | ✅ | |
| `vi.hoisted` | ✅ | Shared between mock-prepass & module. |
| `vi.waitFor/waitUntil` | ✅ | |
| `vi.dynamicImportSettled` | ✅ | vitest's `waitForImportsToResolve`: one macrotask on the real clock (fake timers do not hold it), then every pending `import()` until none is left. |
| `jest.*` alias | ✅ | Compatibility shim object. |
| `toMatchObject` / `toContainEqual` / `toSatisfy` / `toHaveBeenCalledOnce` / `toHaveBeenNthCalledWith` | ✅ | Present (verified by `test/compat-api.test.mjs`). |
| `bench()` | ❌ | No benchmark API. |

### Node built-in modules

Tests run in bare V8, so Node's builtins are runtime shims (`src/runtime.js` `__nodeBuiltins`).
`import x from 'node:x'`, `import { a } from 'x'` and `require('x')` all get the same object on both
loader paths (ESM graph and module runner); `node:` prefixes are accepted. Every listed builtin
has a non-empty shim (a unit test enforces it); an unlisted one fails to resolve. An `import()`
that does not resolve rejects as vitest's does: `Cannot find module '/sub/x.js' imported from
<importer>` (a relative specifier named root-relative, as vite resolves it), `Cannot find package
'x' imported from <importer>` (code `ERR_MODULE_NOT_FOUND`), or `No such built-in module: node:x`
(`ERR_UNKNOWN_BUILTIN_MODULE`).

| builtin | provided |
|---|---|
| `buffer` | `Buffer` (the runtime's global `Buffer`), `atob`, `btoa`, `Blob`, `File` (`text()` UTF-8-decodes the bytes of all parts at once, stripping one leading BOM, as Node 26 (turbo-test#120)). |
| `fs`, `fs/promises` | Native-backed, with Node's semantics and errors (`code`, `errno`, `syscall`, `path`, message): `existsSync`, `readFileSync` (a `Buffer` without an encoding), `writeFileSync` / `appendFileSync` (strings or byte views; a real `O_APPEND` append; no implicit parent directories), `mkdirSync` (`recursive` returns the first directory created), `rmSync` (`recursive`, `force`; `ERR_FS_EISDIR`), `rmdirSync`, `unlinkSync`, `readdirSync` (Node's sorted order, `withFileTypes` Dirents, `recursive`), `statSync` / `lstatSync` (`Stats` with Node's fields; `throwIfNoEntry`), `symlinkSync`, `Dirent`, `Stats`; every path argument may be a `file:` URL. `promises` (= `fs/promises`): `readFile`, `writeFile`, `appendFile`, `mkdir`, `rm`, `rmdir`, `unlink`, `readdir`, `stat`, `lstat`, `symlink`. `constants` (the platform-independent ones: `F_OK`/`R_OK`/`W_OK`/`X_OK`, `O_RDONLY`/`O_WRONLY`/`O_RDWR`, `S_IF*`, `COPYFILE_*`). No callback-style API, no streams, no file descriptors. |
| `process` | `globalThis.process`. `process.stdout.write` / `process.stderr.write` write their text (string or bytes) to stdout / stderr as is (stdout text goes to stderr while a json/junit/tap report owns stdout; `--silent` drops it), return `true` and call a trailing callback. |
| `timers`, `timers/promises` | The global timers (fake timers drive the same functions); `timers.promises` = `timers/promises`: `setTimeout`, `setImmediate`, `setInterval` (async iterator), `scheduler.wait`/`scheduler.yield`, all honouring `{ signal }` (`AbortError`, code `ABORT_ERR`). |
| `assert`, `assert/strict` | Node's API: `ok`, `equal`/`notEqual`, `strictEqual`/`notStrictEqual`, `deepEqual`/`deepStrictEqual` (+ `not…`; Map/Set/Date/RegExp/Error/typed arrays/boxed primitives/prototypes/cycles), `throws`/`doesNotThrow`, `rejects`/`doesNotReject` (class / RegExp / validation function / object matchers), `match`/`doesNotMatch`, `ifError`, `fail`, `AssertionError` (`code: 'ERR_ASSERTION'`, `actual`/`expected`/`operator`). `assert/strict` = `assert.strict` (the `equal`/`deepEqual` family is strict). Generated messages are shorter than Node's diffs. |
| `querystring` | Node's `parse`/`decode` (incl. `sep`/`eq`/`maxKeys`/`decodeURIComponent`), `stringify`/`encode`, `escape`, `unescape`. |
| `string_decoder` | `StringDecoder` for `utf8`, `utf16le`/`ucs2`, `latin1`/`binary`, `ascii`, `hex`, `base64`, `base64url`, buffering partial characters across `write()`s like Node (U+FFFD for invalid / truncated UTF-8). |
| `vm` | `runInThisContext`, `Script#runInThisContext`, `compileFunction` (current context), `isContext` (always `false` for objects). `createContext`, `runInContext`, `runInNewContext` (and the `Script` variants) throw: tests run in a single V8 context. A `Script`'s syntax error surfaces when it runs, not at construction. |
| `worker_threads` | The main-thread view: `isMainThread: true`, `parentPort: null`, `threadId: 0`, `workerData: null`, `resourceLimits: {}`, `MessageChannel`/`MessagePort` (the runtime's), `get`/`setEnvironmentData`, `markAsUntransferable`/`isMarkedAsUntransferable`, `SHARE_ENV`. `new Worker()` throws (one thread). |
| `inspector` | No inspector is active: `url()` is `undefined`, `waitForDebugger()` throws `ERR_INSPECTOR_NOT_ACTIVE`, `Session#post` throws `ERR_INSPECTOR_NOT_CONNECTED`, `open()`/`Session#connect()` throw; `console`, `close()`. |
| `constants` | Node's merge of `os.constants` (`errno`, `priority`, `signals`), `fs.constants` and `crypto.constants`. `os.constants.signals`/`errno` carry the POSIX numbers Linux and macOS share (empty on Windows). |
| `url` | `URL` (the runtime's WHATWG-style URL: relative resolution, userinfo, path / query / fragment percent-encoding, the query kept as parsed; inspected, JSON-serialized and diffed as Node's / vitest's), `URLSearchParams` (the global, checked against Node 26 (turbo-test#117): string / record / pair-sequence / URLSearchParams init with TypeError for a bad pair, USVString names and values in every method, `has` / `delete` value argument, stable code-unit `sort`, `size`, live `forEach` and iterators, form-urlencoded `toString`, write-back to the owning URL), `fileURLToPath` / `pathToFileURL` with Node's decoding, encoding and errors (`ERR_INVALID_URL_SCHEME`, `ERR_INVALID_FILE_URL_HOST`, `ERR_INVALID_FILE_URL_PATH`). |
| `util` | `inspect` / `format` / `formatWithOptions` as Node's (`inspect.custom`, depth / breakLength / compact options; no colours, no `showHidden`); `promisify`, `callbackify`, `inherits`, `deprecate`, `types`, `TextEncoder` / `TextDecoder` (also globals; checked against Node 26 (turbo-test#118, #119): U+FFFD for lone surrogates and per maximal invalid UTF-8 subpart, `encodeInto`, `fatal`, BOM handling and `ignoreBOM`, `stream`, any BufferSource input; `TextDecoder` takes every WHATWG Encoding label Node does (the `replacement` labels throw RangeError) and decodes the legacy encodings through encoding_rs — single-byte, UTF-16LE/BE, gbk / gb18030 and iso-2022-jp decode as Node's on every 1- and 2-byte input, but Node's ICU big5 / euc-jp / euc-kr / shift_jis decoders are not the Encoding Standard's and differ on some bytes, where turbo-test follows the standard: e.g. euc-kr `81 41` is `갂` (Node `\x81A`), shift_jis `80` is U+0080 (Node U+FFFD) and `1A` / `1C` / `7F` stay themselves (Node swaps them), big5 / euc-jp / euc-kr `80` is U+FFFD (Node U+0080) and big5 HKSCS pairs are the standard's code points, not ICU's private-use ones). `console.*` formats its arguments with `format`. |
| `path`, `os`, `events`, `stream`, `crypto`, `module`, `child_process`, `perf_hooks`, `http`, `https`, `http2`, `net`, `tls`, `zlib`, `dns`, `dns/promises`, `tty`, `dgram`, `readline`, `v8`, `diagnostics_channel`, `async_hooks` | Load-time surfaces (enough for libraries to import and construct); see `src/runtime.js`. No real networking, processes or compression. |

Native addons (`.node`) load through turbo-test's own Node-API host, which implements every
Node-API function up to version 10 with Node 26's semantics; see
[docs/node-api.md](docs/node-api.md) for the inventory and the few host differences.
Matchers build their failure message only when the assertion fails, as vitest does: a passing
assertion never formats its operands. A failed built-in matcher throws what vitest 4.1.12 throws
(turbo-test#71): chai's `AssertionError` (`actual` / `expected` / `showDiff`) with vitest's message —
chai templates with values shown as chai's `objDisplay`, and for the spy matchers vitest's call /
return listings and `diff` of pretty-format output, without colours (as under `NO_COLOR`);
`toSatisfy` / `expect.extend` matchers throw a plain `Error`, a snapshot mismatch
``Error: Snapshot `<name> <n>` mismatched``. Spies record a throwing call
(`mock.results` `{ type: 'throw' }`) and a `vi.spyOn` spy is named after its method.

---

## 5. Config-file reading (`cli.js`)

| capability | status | Notes |
|---|---|---|
| auto-discover nearest `vitest.config.*` / `vite.config.*` | ✅ | walks up from cwd. `-c/--config <path>` forces an exact file (skips the walk-up; its dir becomes the discovery root). |
| `test.include` / `test.exclude` globs | ✅ | string-scan (no TS eval); drives discovery. Either one alone applies with vitest's default for the other (`**/*.{test,spec}.?(c|m)[jt]s?(x)` / `**/node_modules/**, **/.git/**`). `node_modules` and `.git` are skipped only by the effective exclude: a config `exclude` replaces vitest's default, so a directory it does not list is searched (discovery prunes a directory a `<prefix>/**` exclude covers). |
| `coverage.include/exclude/thresholds` | ✅ | string-scan; flags win over config. |
| `test.environment` | ✅ | string-scan; sets the run default env when `--environment` is absent → `TURBO_ENV`. Per-file pragma still wins. |
| anything requiring evaluating the config (functions, `defineConfig` logic, env interpolation, plugins, `setupFiles` array beyond first, aliases) | 🟡/❌ | Pure regex scan — dynamic config is invisible. |
| `test.setupFiles` | ✅ | read by the runner (`runner.rs`) from the nearest config, every name vitest resolves in its order (`vitest.config` then `vite.config`, each `.ts .mts .cts .js .mjs .cjs`); a jest config's `setupFiles` / `setupFilesAfterEnv` otherwise. |
| `test.globals`, `test.testTimeout`, `test.retry`, `test.bail`, `test.pool`, `test.reporters` | ❌ | Not read from config. (`test.isolate:false` IS autodetected separately in `runner.rs`.) |

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
  ✅ `--silent`; ✅ `--allowOnly`/`--no-allowOnly` (default `!isCI`, see §2); ✅ `--maxWorkers`/`--minWorkers`.
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
  `cli.rs` reporters) — passing test names are now retained. Added
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
- 2026-06-17 — **Execution-control batch shipped**: `--testTimeout` + per-test `{ timeout }`
  ENFORCEMENT (internal-timer race, invisible to fake timers; hung tests fail cleanly), `--retry`
  global default, `--bail <n>` (file-granular cross-worker abort), `--maxWorkers` alias + `--minWorkers`
  no-op, `--silent` (call-time console no-op), `--allowOnly`/`--no-allowOnly` (per-file `.only` gate).
  Added `test/compat-runflags.test.mjs` (10 tests) + `fixtures/compat/{timeout,per-test-timeout,retry}.test.ts`
  and `fixtures/compat/{bail,silent,only}/`. cli.js value-flag regex extended for the new value flags.
