# Isolate-reuse — remaining failures: ideas & evaluation

Context: `TURBO_REUSE_ISOLATE` / vitest `isolate:false` mode reuses one V8 isolate per worker so
node_modules barrels evaluate once, not per file (~5× faster). **Fresh mode (`TURBO_NO_REUSE`) is
6189/0 on `ui-design-components`.** Reuse is best ~6180/9, varies 9–48 (parallel work-stealing).
This doc captures the brainstorm + evaluation for closing the gap to 6189/0 under reuse.

## The remaining failures

- **~5 deterministic** (consistent across runs):
  - `MarketingNavigation Analytics` ×2 — component passes `null` to `trackEvent` where the test
    asserts `expect.anything()`. A cached `MarketingNavigation` holds a stale/real marketing
    analytics object; `useAnalytics()` returns null.
  - `TimeSeriesBarChartCard` ×3 — recharts axis mock (`data-testid="x-axis"/"y-axis"`) not applied.
- **~4–40 flaky** — the 9→48 swing. Work-stealing hands each worker a random file set, so residual
  cross-file leaks surface non-deterministically.

### Root-cause families
1. **Cached-component mock-identity** — component C imported dep D in file A (real or mock-A); file
   B re-mocks D; C is cached holding stale D. Mock-graph invalidation handles `vi.mock` IF import
   edges are recorded, but edges are incomplete (ESM-static imports via `resolve_callback`, bundles)
   and `vi.spyOn` doesn't trigger it.
2. **Test-imported setup modules** — e.g. marketing `analytics-test-setup` is cached, so its
   `vi.mock` runs once; my per-file reset then evicts the mock → later files get the real module.
3. **Parallel non-determinism** — work-stealing exposes leaks per random distribution.
4. **jobs-1 hang** — a file leaks an unsettleable event loop (interval-clearing helped, not fully).

---

## Ideas (scored)

Hard constraint: fresh 6189/0 must NOT regress; reuse must not get worse.

| # | Idea | Fixes | Effort | Risk | Correctness | Verdict |
|---|------|-------|--------|------|-------------|---------|
| 1 | **Fresh-retry on failure** | all 9 + flaky (suite==fresh) | low-med | low | **guaranteed ==fresh** | ★★★★★ |
| 2 | **Generation-stamped timers** | jobs-1 hang + timer flakes | med | low | strong (hang) | ★★★★ |
| 3 | Edge-audit + `vi.spyOn` invalidation | TS×3, MN×2 at source | med | med | source fix | ★★★ |
| 4 | Re-run test-imported setup modules per file | MN class | med | med-high | source fix | ★★ |
| 5 | Deterministic partition (no work-steal) | 0 (enables repro) | low | low | — | ★★★ |
| 6 | Periodic isolate recycle every N files | bounds accumulation | low | low | — | ★★★ |
| 7 | Route mock-heavy files → fresh | TS, MN | low | low | partial | ★★ (subsumed by #1) |
| 8 | Leak detector (diff state vs baseline) | 0 (diagnostic) | med | none | — | ★★★ if going deep |

### 1. Fresh-isolate retry on reuse failure ★★★★★
Reuse = optimistic fast path. Any failure → re-run THAT file in a brand-new fresh isolate
(authoritative, 6189/0). Keep the fresh result. Reuse-fail that passes fresh = leak artifact →
count as pass; fails fresh too = genuine. **Suite correctness == fresh, at ~reuse speed** (only
failing files pay a fresh re-run). Neutralizes leaks instead of fixing them; makes reuse safe to
enable by default. Cost bounded (~9 fails → +~14s). Caveat: catches leak-induced *fails*, not
leak-induced *passes* (rare here — reuse has fewer passes than fresh, i.e. leaks break not fix).
Safety valve: if >X% of a worker's files fail, the isolate is poisoned → recycle (fresh).
Impl: thread-local `FORCE_FRESH` checked in `run_test_file` before `reuse_decision`; retry sets it,
calls the existing fresh body (new isolate, not the poisoned persistent one).

### 2. Generation-stamped timers ★★★★
Tag every scheduled timer/tick with a per-file generation id; bump per file; on reset drop
prior-generation timers (leaks), keep current-file pending one-shots. Kills the jobs-1 hang (a
leaked self-rescheduling timeout) + leftover-effect flakes, WITHOUT the "clear-all-timers broke 40
async tests" regression. Independent of #1 (a hang isn't a retryable "failure" — it stalls).

### 3. Complete import edges + `vi.spyOn` invalidation ★★★
Mock-graph invalidation only fires if edges exist. Record edges in `resolve_callback` (ESM static
imports), audit that `MarketingNavigation→analytics` and `TimeSeriesBar→recharts` edges exist
(log them). Add native `__ttOnSpyOn(obj)` → reverse-map obj→path via `cjs_exports` identity →
`invalidate_importers`. Fixes TS/MN at the source (reduces how often #1 fires). Needs #5 to repro.

### 4. Re-run test-imported setup modules per file ★★
Detect modules with top-level `vi.mock(`/`vi.hoisted(` imported by the entry → treat like global
setupFiles: evict + re-eval per file so hoisted mocks re-apply (matches vitest per-file scoping),
and keep/snapshot their mocks. RISKY: re-evaluating modules per file is what caused the emotion/MUI
accumulation; the hook-baseline attempt at this class already went catastrophic (2183 fails).

### 5. Deterministic partition ★★★
Replace work-stealing's shared atomic cursor with a stable duration-aware static partition
(file→worker by hash). Fixes nothing directly but makes failures deterministic → reproducible →
fixable, and the suite reproducible. Good debug/CI mode; could even be the reuse default.

### 6. Periodic isolate recycle ★★★
Drop+rebuild the isolate every N files (e.g. 25). Bounds leak accumulation; still ~5× amortized.
Cheap insurance; pairs with #1 (recycle a poisoned isolate).

### 7. Route mock-heavy files → fresh ★★
Static-detect files that `vi.mock` a bare node_modules specifier → run those fresh, rest reuse.
Simple hybrid, but #1 subsumes it more elegantly (re-run only on actual failure, not preemptively).

### 8. Leak detector (diagnostic) ★★★
After each file, diff globals/registry/DOM vs the post-setup baseline → log what leaked. Turns
"mystery flaky" into "file X leaks `document.fullscreenElement`." Accelerates #3/#4.

### Rejected / tried-and-failed
- Dir-based component eviction — tried, **hurt** (22→38).
- Hook-baseline restore — tried, **catastrophic** (dropped @testing-library cleanup, 2183 fails).
- V8 heap checkpoint/restore per file — infeasible (no mid-run restore API).
- Re-eval whole import subtree on mock — reintroduces the accumulation blowup.

---

## Recommended sequence
1. **Tier 1 (build, high confidence): #1 + #2** → reuse correct (==fresh) and non-hanging →
   shippable by default. ~80% of the value, low risk.
2. **Tier 2 (trim retry cost): #5 then #3** → repro first, then fix TS/MN at source.
3. **Tier 3 (insurance): #6 recycle, #8 detector.** Skip #4 unless residue remains.

Bottom line: **#1 is the move** — converts "9 flaky failures" into "0 failures, slightly slower,"
with a correctness guarantee, and unlocks reuse-by-default.
