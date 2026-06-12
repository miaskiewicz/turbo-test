# Coverage backlog — function + branch coverage

v0.2.1 shipped **line coverage**; **v0.2.2 adds function + branch coverage** (`src/coverage.rs` +
`src/coverage_branch.rs`). Both are DONE. Remaining: a per-worker speed optimization (below).

## Status

- [x] **Function coverage** (v0.2.2) — from each V8 function's outer range + count → lcov
      `FN`/`FNDA`/`FNF`/`FNH`.
- [x] **Branch coverage** (v0.2.2) — oxc AST decision points (if/else, ?:, &&/||/??, switch)
      correlated with V8 block ranges mapped back through the source map → lcov
      `BRDA`/`BRF`/`BRH`. Implicit-else taken = block − then. Verified exact on a fixture.
- [ ] **Per-worker collection (speed).** Coverage currently attaches the inspector + calls
      `takePreciseCoverage` per file under fresh isolation (~2–3× a plain run). Collecting once per
      worker (start coverage at isolate creation, take at worker teardown) would cut the per-file
      inspector-attach + serialize cost. Needs reuse mode (persistent isolate per worker), which is
      only test-correct on `isolate: false` projects — so safe to auto-enable only there. For
      fresh-isolation suites it stays per-file.

## Original spike notes (kept for reference)

### Function coverage
- [x] **Function coverage (EASY — ~1h).** The `takePreciseCoverage` result already gives, per
      script, `functions: [{ functionName, ranges: [...], isBlockCoverage }]`. The FIRST range of
      each function entry is the function body with its invocation `count`. So:
      - function is "hit" when its outer-range count > 0.
      - map the outer range's `startOffset` → original line (same source-map path `map_script`
        already uses) for the `FN:<line>,<name>` record.
      - emit lcov `FN:`, `FNDA:<count>,<name>`, `FNF:` (total), `FNH:` (hit), and add a function-%
        column to the summary.
      Data is all in hand — this is pure plumbing in `coverage.rs`.

- [ ] **Branch coverage (HARD — real spike).** V8 block coverage (we already request
      `detailed:true`) returns nested ranges; a sub-range with `count: 0` inside a covered
      function is an *untaken block* — i.e. a branch arm that didn't run. But V8 reports ranges,
      not branch *grouping* (which arms belong to which decision), and lcov `BRDA:<line>,<block>,
      <branch>,<taken>` wants that structure. Two approaches:
      - **(a) Approximate (medium):** treat each `count: 0` sub-range (nested inside a `count > 0`
        parent) as one untaken branch at its start line; everything executed = taken. Cheap, no
        AST, but branch numbering is synthetic and `if/else`/ternary/`??`/`&&` arms aren't grouped
        the way Istanbul reports them.
      - **(b) Accurate (hard):** parse each source file with oxc (already a dep) to enumerate
        decision points (if/else, ?:, &&/||/??, switch, optional chaining, default params), then
        correlate each branch arm's source span with the V8 block ranges (offset → which range)
        to get per-arm taken counts. This is essentially what c8 + `@bcoe/v8-coverage` /
        `v8-to-istanbul` do. The offset↔AST correlation across our wrapper + esbuild transform
        (TS/JSX → CJS) is the fiddly part — branch spans must be mapped through the same source
        map, and esbuild can reshape expressions (e.g. JSX, optional chaining lowering) so some
        source branches don't survive 1:1 into the generated ranges.
      Recommendation: if/when we do branches, port the proven `v8-to-istanbul` mapping logic
      rather than reinvent — it already handles the V8-ranges → Istanbul-branch translation.

## Notes / gotchas for the spike

- Offsets are **UTF-16 code units** into the *wrapped* script source (the CJS `(function(...){…})`
  wrapper adds exactly one line — `coverage.rs::map_script` already accounts for this with the
  `genLine = wrappedLine - 1` shift).
- We only emit maps + name scripts under `--coverage` (gated), and the post-esbuild rewrites
  (`hoist_mock_setup`, `shared_mock_lets`) only fire on `vi.mock`/test files — which are excluded
  from the report — so source files keep a valid 1:1 line map. Branch work must preserve that
  invariant (don't try to branch-cover hoist-rewritten files).
- `--coverage` forces fresh isolation today; if coverage moves to per-worker (reuse) for speed,
  re-confirm the byte-offset → source mapping still holds (it should: same compiled script bytes).

## Related

- Line coverage impl: `src/coverage.rs`, wired in `src/runner.rs` (`coverage_accumulate`,
  `run_test_file` collector hooks) + `src/bin/turbo_test.rs` (`--coverage` / `--coverage-dir`).
- Perf: see the coverage speed work (per-worker collection + source-map memoization) — the
  execution overhead from `detailed:true` block coverage is the dominant cost.
