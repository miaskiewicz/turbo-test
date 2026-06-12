# turbo-test backlog

## Node-runtime fallback for Node-only test files
**Idea:** Some test files genuinely cannot run in the V8 embedding because they pull in
Node-only native libraries (e.g. `@playwright/test` → `playwright-core` → `agent-base`
`class extends http.Agent`, plus `net`/`child_process.spawn`/TLS + a browser binary). Shimming
these is bottomless and pointless — they also drive live processes/network at runtime.

**Proposal:** Let a test file opt into a real-Node execution path via a magic comment, e.g.

```ts
// @turbo-runtime node
```

When the scheduler sees that pragma (or a config glob like `e2e/**`), it runs the file with a
spawned `node` (vitest/tsx) subprocess instead of the in-process V8 runner, then parses the
result back into the same `TestReport` channel. Keeps the fast V8 path as default; node-only
files get correctness via a slower fallback. Flag could also be `--node-fallback` CLI + a
`turbo-test.config` `nodeRuntime` glob list.

Known current case: `payroll-app/e2e/src/helpers/data-seed-client.test.ts` (the 1 remaining
load-error after the flux-ui/recharts chain + global-shim work).
