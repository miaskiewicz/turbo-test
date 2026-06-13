# Spike: can isolate-reuse be enabled by default? — VERDICT: NO (accuracy)

Goal: evaluate `TURBO_REUSE_ISOLATE` (one persistent isolate+context per worker, node_modules
evaluated once across files — vitest `isolate:false` semantics) as a DEFAULT, to skip re-executing
the node_modules graph per file (the dominant redundant cost). Hard gate: 100% accuracy parity
with the fresh path.

## Result
- **ui-design-components: reuse = 7006/0, identical to fresh, and faster (~53s).** Clean.
- **payroll-app: reuse = 10368/221 vs fresh 10580/1.** 221 failures. Per-file diff on a calm
  100-file subset showed deterministic whole-file flips (NOT load flakiness):
  - `ImssWorkRiskCard.test.tsx` 15/0 → 0/15
  - `useBulkModals.test.tsx` 24/0 → 0/24
  - `useTimesheetEditHandlers.test.ts` 14/0 → 0/14
  - `WizardLayout.test.tsx` 27/0 → 25/2
- Reproduces **standalone** (single file, no prior file): ImssWorkRiskCard alone under reuse =
  0/15, every test failing with `useApiContext must be used within an ApiProvider`.

## Root cause (fundamental, not a fixable bug)
The failing tests do per-file `vi.mock('<node_modules pkg>', …)` — e.g.
`vi.mock('@flux-payroll/flux-payroll-ui', …)` which supplies a fake `useApiContext` so the
component renders without a real `ApiProvider`. payroll mocks node_modules packages heavily
(`next-intl`, `@mui/material`, `@flux-payroll/flux-payroll-ui`).

- FRESH: every file gets a fresh isolate that reloads the whole graph, so the per-file mock of a
  node_modules package takes effect → test passes.
- REUSE: node_modules modules are evaluated ONCE and KEPT across files — that caching IS the
  speedup. A per-file `vi.mock` of an already-cached node_modules package can't override it, so
  the REAL module runs → `useApiContext` throws → whole file fails.

This is an inherent tension: "cache node_modules across files" (the reuse win) vs "per-file
`vi.mock` of node_modules" (needs per-file reload of that module). You cannot have both without
per-module cache invalidation+reload whenever a file mocks it — which defeats the reuse benefit
exactly for the modules tests touch most.

The fresh-isolate retry net (re-run a failing file fresh, adopt that result) does NOT rescue these
— the failure survives in the final output, so reuse-by-default would ship wrong results.

ui passes only because its tests mock app modules (which the reuse path DOES reset per-file) rather
than node_modules packages. So reuse safety is workload-dependent → unsafe as a generic default.

## Decision
Keep reuse OPT-IN (`TURBO_REUSE_ISOLATE` / vitest `isolate: false`), as it already is. A project
that opts in is declaring it doesn't per-file-mock node_modules. It must NOT be a default.

## If someone wants to make reuse safe later (large, out of spike scope)
- Detect per-file `vi.mock`/`jest.mock` of a node_modules specifier and, for those files only,
  fall back to the fresh path (or invalidate+reload just that module subtree under reuse). Needs
  the mock-specifier scan (already have `extract_mocks`) wired to a per-file reuse-vs-fresh routing
  decision, plus a real fix to why the existing fresh-retry doesn't rescue these.
- Or move to a process/fork worker pool so a per-worker reset is a real fresh process (kills the
  process-global turbo-dom DOM-addon corruption angle too) — major architecture change.
Either is a project, not a tweak, with no guarantee of full 10k-test parity.
