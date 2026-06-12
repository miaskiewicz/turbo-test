# Spec coverage matrix

Tracks every requirement in `turbo-test-spec.md` so nothing is skipped. Status:
✅ done & verified · 🟡 partial/spike · ⬜ not started. Each line says where it lives.

## §3 Reuse vs build
| item | status | where |
|---|---|---|
| oxc transform (TS/JSX) | ✅ | `src/transform.rs` |
| oxc_resolver (module resolution) | ✅ | `runner.rs` resolver() — zod/node_modules/exports maps, full utils dir parity |
| Vite/Node resolution semantics | ✅ | matched (types-condition + package.json type bugs found & fixed) |
| Read existing vitest/vite config | ⬜ | M6 remaining |
| Native worker pool / module runner / isolation / scheduler | ✅ | M0+M1+M4 (parallel CLI, work-stealing) |
| Native module loader + CJS/ESM interop | ✅ | `src/runner.rs` |
| Native event loop + timer bindings | ✅ | M2 (`runtime.js` __loop) |
| Transform cache + resolution graph orchestration | ✅ | M4 (cache) + M5 (graph) |
| Reuse real @vitest/expect, snapshot, spy, chai, sinon fake-timers, vi | 🟡 | minimal shim now (`runtime.js`); real bake = M3 |

## §4 Architecture
| item | status | where |
|---|---|---|
| Node-API (napi) host — load native .node addons | ✅ | `src/napi_host.rs` — proven loading turbo-dom's native parser + parseBuffer |
| Snapshot isolation (context-from-snapshot per file) | ✅ | M0 spike + **M3 in real runner** (`framework_snapshot()`, 0.58ms/file) |
| Content-addressed transform cache | ✅ | `runner.rs` read_transformed — warm 100% hit |
| Resolve-once graph + reverse query | ✅ | `src/graph.rs` affected-test detection |
| N isolates across cores reusing one snapshot | ✅ | parallel CLI, 2.7× at 10 files |
| Result channel | ✅ | in-process TestReport (IPC unneeded for thread model) |

## §5 Fake timers + event-loop ordering (M2, hard gate) — PASS
| item | status | where |
|---|---|---|
| Native event loop, Node-compatible macro/micro/nextTick ordering | ✅ | `runtime.js` `__loop` + `runner.rs` drive loop |
| Fake-timer logic bound to native loop | ✅ | `runtime.js` (native equiv; real @sinonjs swap = M3) |
| vi.useFakeTimers/advanceTimersByTime/runAllTimers/setSystemTime/Date mocking | ✅ | `runtime.js` `vi.*` |
| Differential ordering battery | ✅ | `fixtures/battery/` — 4/4 parity vs node oracle |
| Fake-timer suite vs stock Vitest | ✅ | `fixtures/tests/timers.test.ts` — 5/5 === vitest |

## §6 Compatibility gauntlet
| item | status | where |
|---|---|---|
| Fixed corpus harness | 🟡 | runs real corpus files now via `turbo-test`; formal harness = task #8 |
| (a) Vitest own suite | ⬜ | resolver ready; needs real @vitest framework bundle |
| (b) RTL + user-event | ✅ | render/getByRole/userEvent.setup work (turbo-dom napi + mocks) |
| (c) 15–20 popular libs | ⬜ | M4 |
| (d) large monorepo (payroll-app, Next.js) | ✅ | 984 files run; 3460 pass; tail = app test infra |
| (e) timer-heavy suite | ⬜ | M2 |
| (f) mock-heavy suite | ✅ | path-keyed vi.mock survives bundling (externalize+drain+prepass+lenient exports) |

## §7 Milestones
| milestone | status | gate result |
|---|---|---|
| **M0** snapshot-isolation spike | ✅ | PASS — snapshot fresh-context 84–1215× cheaper than stock-vitest env; full isolation kept (`docs/M0-FINDINGS.md`) |
| **M1** module loader + CJS/ESM interop | ✅ (in scope) | PASS — torture 12/12; real corpus 66/66 where loadable, bit-exact vs vitest; bare-specifier load = M4 (`docs/M1-PROGRESS.md`) |
| **M2** event loop + fake timers | ✅ | PASS — battery 4/4 order parity vs node; fake-timer suite 5/5 === vitest (`docs/M2-FINDINGS.md`) |
| **M3** framework layer in snapshot | 🟡 | mechanism PASS — bake + context-from-snapshot + guardrail 0.58ms (`docs/M3-FINDINGS.md`); real @vitest/* + turbo-dom bundle = remaining content swap (same seam) |
| **M4** transform cache, oxc_resolver, scheduler, result channel | ✅ | PASS — utils dir 145/145 parity, warm cache 100%, scheduler 435=145×3 no drops (`docs/M4-FINDINGS.md`) |
| **M5** watch mode + affected-test detection | ✅ | PASS — affected set verified superset & precise; ~1-6ms analysis (`docs/M5-FINDINGS.md`) |
| **M6** coverage, source maps, reporters, modifiers, sharding, config | 🟡 | modifiers/sharding/JSON-reporter DONE; coverage/source-maps/config/TAP remain (`docs/M6-FINDINGS.md`) |

## §8 KPIs
| KPI | status |
|---|---|
| Gauntlet pass-rate vs vitest | 🟡 ui-design 96.6% (5833/6040); payroll 3460 pass; pure-logic 100% bit-exact |
| Snapshot byte-compat (.snap) | ⬜ M3 |
| Resolution divergences vs Vite = 0 | ✅ M4 (full utils dir parity) |
| Timer/microtask order parity 100% | ✅ M2 (battery 4/4 vs node oracle) |
| Source-map correctness 100% | ⬜ M6 |
| Speed ≥5× cold / ≥10× warm | ✅ env-setup 84×+; warm cache 100%; parallel 2.7× |
| Per-file env setup ~snapshot-instantiate | ✅ M0 (0.38ms vs 32–465ms) |
| Watch incremental ≥5× | ✅ M5 (ms-scale affected analysis vs vitest watch re-run) |

## Build artifacts
- `m0-spike` — isolation benchmark (M0)
- `m1-esm`, `m1-cjs`, `m1-transform` — layer proofs (M1)
- `turbo-test <files...>` — consolidated runner (M1 end-to-end)
- Fixtures: `fixtures/{esm,esm-ts,cjs,interop,ts,tests}/`
