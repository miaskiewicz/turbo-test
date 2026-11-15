# Flux APIs backend validation — v0.8.4, 2026-11-15

Recorded 2026-11-15T12:29:54.667548+00:00. This is a preservation report from existing runs, not a new investigation. No failures were suppressed and no application tests were changed.

## Scope and disposition

- The 16-file representative selection is green: **595 passed, 0 failed, 0 load errors**.
- The latest completed additional selection has **64 files, 1,197 passed tests, 8 failed tests, and 8 load-error files**. The 8 failures are in one file; they are separate from the 8 load errors.
- The second additional 64-file selection hit its 180-second overall cap and has no complete JSON report. Its test outcomes are **unknown**, not passing.
- The separate CFDI certifier selection has **6 passed / 13 failed / 19 registered**, with no load errors. The remaining native-addon link problem is deferred to [turbo-test #391, v0.8.5](https://github.com/miaskiewicz/turbo-test/issues/391).
- The additional-run failures below are **untriaged**. No matching Jest counterfactual was completed for those exact 64-file selections; do not label every one a proven Turbo regression.
- Per the user, backend discovery/fix expansion stopped at **2026-11-15 12:24:37 UTC**. Finish existing fixes and their validation only. Do not start fixing this backlog automatically.

## Exact inputs

- Turbo source: `cf9a0a577a642c4ff925a10ad3c5c61f790385e2` (known fixes, **before** runtime-profile split).
- Optimized binary SHA-256: `038a97aeb6ef5f010e79f16b8e612cffa51626875b82cec1b90f4a1085eb6fd7`.
- Flux checkout: [PR #1196](https://github.com/flux-payroll/flux-apis/pull/1196), branch `codex/libxmljs2-node-compat`.
- Flux commit: `2d94cb3e8082161b8ea305dd83614eec173c2a47`.
- Flux lock SHA-256: `c15ebdd6ba802c5f982b9bb280d7cced92e3cb8710c540c053d2452dd607d8a0`.
- Node `v26.11.0`; real `libxmljs2@0.37.0`, rebuilt from source with the PR override `nan@2.30.0`.
- Real native XML parse and invalid-XML XSD validation work in Node. The unchanged CFDI certifier Jest control passes **19/19**; the same representative Jest selection passes **595/595**.
- Original Flux source, specs, and configuration are unchanged. A credential-free environment and external native launcher were used.
- Each native run used `--jobs 1 --reporter json`; recorded elapsed times are correctness-run metadata, **not isolated speed comparisons**.

## Run receipts

| Selection | Registered tests | Passed | Failed | Load-error files | Completion |
|---|---:|---:|---:|---:|---|
| Representative (16 files) | 595 | 595 | 0 | 0 | exit 0; 43.595 s |
| Additional 00 (64 files) | 1205 | 1197 | 8 | 8 | exit 1; 109.81 s |
| Additional 01 (64 files) | unknown | unknown | unknown | unknown | overall timeout; 180.343 s |
| CFDI certifier (1 file) | 19 | 6 | 13 | 0 | exit 1; 84.099 s |

## Additional 00: all 8 load errors

| Flux spec | Exact recorded load message |
|---|---|
| `src/audit/auth-matrix/build-auth-matrix.spec.ts` | `Invalid or unexpected token` |
| `src/audit/auth-matrix/flat-gate-employment-grants.spec.ts` | `Invalid or unexpected token` |
| `src/audit/auth-matrix/orphan-permissions.spec.ts` | `Invalid or unexpected token` |
| `src/audit/auth-matrix-builder.spec.ts` | `Invalid or unexpected token` |
| `src/audit/data-compliance/analyzer.spec.ts` | `Cannot read properties of undefined (reading 'native')` |
| `src/audit/perf/analyzer.spec.ts` | `Cannot read properties of undefined (reading 'native')` |
| `src/audit/scale/analyzer.spec.ts` | `Cannot read properties of undefined (reading 'native')` |
| `src/audit/tenant-isolation/analyzer.spec.ts` | `Cannot read properties of undefined (reading 'native')` |

The report contains no underlying origin stack for these load messages. Syntax-token and `native` property errors are observations, not established root causes.

## Additional 00: all 8 failed tests

### PropelAuthUserWebhookController acks and ignores a non-sync event type (e.g. user.deleted)

Spec: `src/auth/controllers/propelauth-user-webhook.controller.spec.ts`

```text
UnauthorizedException
    at PropelAuthUserWebhookController.handle (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.ts:86:19)
    at call (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:39:39)
    at ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:61:16
    at body (<anonymous>:12664:12)
    at <anonymous>:12666:72
    at <anonymous>:12483:17
    at new Promise (<anonymous>)
    at __vtSettle (<anonymous>:12478:10)
    at <anonymous>:12666:43
    at <anonymous>:12483:17
```

### PropelAuthUserWebhookController ignores a user.updated event missing user_id or email

Spec: `src/auth/controllers/propelauth-user-webhook.controller.spec.ts`

```text
UnauthorizedException
    at PropelAuthUserWebhookController.handle (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.ts:86:19)
    at call (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:39:39)
    at ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:69:16
    at body (<anonymous>:12664:12)
    at <anonymous>:12666:72
    at <anonymous>:12483:17
    at new Promise (<anonymous>)
    at __vtSettle (<anonymous>:12478:10)
    at <anonymous>:12666:43
    at <anonymous>:12483:17
```

### PropelAuthUserWebhookController syncs the confirmed email on user.updated

Spec: `src/auth/controllers/propelauth-user-webhook.controller.spec.ts`

```text
UnauthorizedException
    at PropelAuthUserWebhookController.handle (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.ts:86:19)
    at call (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:39:39)
    at ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:76:16
    at body (<anonymous>:12664:12)
    at <anonymous>:12666:72
    at <anonymous>:12483:17
    at new Promise (<anonymous>)
    at __vtSettle (<anonymous>:12478:10)
    at <anonymous>:12666:43
    at <anonymous>:12483:17
```

### PropelAuthUserWebhookController also handles user.created and the userId field fallback

Spec: `src/auth/controllers/propelauth-user-webhook.controller.spec.ts`

```text
UnauthorizedException
    at PropelAuthUserWebhookController.handle (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.ts:86:19)
    at call (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:39:39)
    at ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:85:16
    at body (<anonymous>:12664:12)
    at <anonymous>:12666:72
    at <anonymous>:12483:17
    at new Promise (<anonymous>)
    at __vtSettle (<anonymous>:12478:10)
    at <anonymous>:12666:43
    at <anonymous>:12483:17
```

### PropelAuthUserWebhookController acks OK even when no local person changed (idempotent no-op)

Spec: `src/auth/controllers/propelauth-user-webhook.controller.spec.ts`

```text
UnauthorizedException
    at PropelAuthUserWebhookController.handle (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.ts:86:19)
    at call (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:39:39)
    at ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:95:16
    at body (<anonymous>:12664:12)
    at <anonymous>:12666:72
    at <anonymous>:12483:17
    at new Promise (<anonymous>)
    at __vtSettle (<anonymous>:12478:10)
    at <anonymous>:12666:43
    at <anonymous>:12483:17
```

### PropelAuthUserWebhookController ignores an event with no event_type field (falls back to empty type)

Spec: `src/auth/controllers/propelauth-user-webhook.controller.spec.ts`

```text
UnauthorizedException
    at PropelAuthUserWebhookController.handle (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.ts:86:19)
    at call (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:39:39)
    at ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:102:16
    at body (<anonymous>:12664:12)
    at <anonymous>:12666:72
    at <anonymous>:12483:17
    at new Promise (<anonymous>)
    at __vtSettle (<anonymous>:12478:10)
    at <anonymous>:12666:43
    at <anonymous>:12483:17
```

### PropelAuthUserWebhookController verifies against empty payload + headers when both are absent

Spec: `src/auth/controllers/propelauth-user-webhook.controller.spec.ts`

```text
AssertionError: expected "vi.fn()" to be called with arguments: [ '', { 'svix-id': '', …(2) } ]

Number of calls: 0

    at new __ChaiAssertionError (<anonymous>:10480:5)
    at Object.toHaveBeenCalledWith (<anonymous>:8404:13)
    at Proxy.<anonymous> (<anonymous>:8632:102)
    at Proxy.methodWrapper (<anonymous>:1409:23)
    at ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:110:18
    at async <anonymous>:12666:66
```

### PropelAuthUserWebhookController ignores an event with no event_type field (falls back to empty string)

Spec: `src/auth/controllers/propelauth-user-webhook.controller.spec.ts`

```text
UnauthorizedException
    at PropelAuthUserWebhookController.handle (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.ts:86:19)
    at call (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:39:39)
    at ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/auth/controllers/propelauth-user-webhook.controller.spec.ts:118:16
    at body (<anonymous>:12664:12)
    at <anonymous>:12666:72
    at <anonymous>:12483:17
    at new Promise (<anonymous>)
    at __vtSettle (<anonymous>:12478:10)
    at <anonymous>:12666:43
    at <anonymous>:12483:17
```

## Additional 01: incomplete run and cleanup caveat

- There is **no complete `turbo-result.json`** for this selection. Do not infer test/file counts from console output.
- Overall cap: 180 s; receipt elapsed: 180.343 s.
- Receipt records `launchError`: `[Errno 1] Operation not permitted` during timeout handling, so it does not establish clean teardown.
- Follow-up host inspection found neither recorded launcher PID (57681 / 57683) live and no process using the exact unique frozen native artifact remained. This is not proof that arbitrary detached grandchildren were inventoried; no unrelated process was killed.
- Preserve this run as **unknown/incomplete**, and retain the cleanup caveat when comparing future runs.

## CFDI certifier: deferred native-addon boundary

- All 19 cases register. The 13 failures share the native library loading error below; this is distinct from the prior fixed `Invalid URL: main.js`, zlib-constant, and anonymous-CJS-filename blockers.
- The Node baseline with the same installed native library passes 19/19.
- Observed missing C++ symbol: `_ZN2v811HandleScope16DeleteExtensionsEPNS_7IsolateE` (`HandleScope::DeleteExtensions`); macOS reports the flat-namespace form with an extra underscore.
- Node reports V8 `14.6.202.34-node.37`, modules ABI `147`; Turbo uses `turbo-dom-v8 150.4.2`. These versions are context, not a complete diagnosis of every ABI requirement.
- This is **v0.8.5 #391**. Do not try an unsafe one-symbol forwarding shim or claim broad NAN compatibility.

### Representative exact failure

```text
PaymentReceiptError: The CFDI could not be validated before sending: validateCfdiXml is unavailable: the libxmljs2 native module could not be loaded (see cause). It is an optional dependency with prebuilt binaries downloaded at install time. (cause: native addon ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/node_modules/libxmljs2/build/Release/xmljs.node: it imports _ZN2v811HandleScope16DeleteExtensionsEPNS_7IsolateE, which nothing loaded provides (dlopen dlopen(${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/node_modules/libxmljs2/build/Release/xmljs.node, 0x0006): symbol not found in flat namespace '__ZN2v811HandleScope16DeleteExtensionsEPNS_7IsolateE'))
    at MexicoCfdiReceiptCertifier.validate (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/hris/models/payment-receipt/countries/mexico/mexico-cfdi-receipt.certifier.ts:145:19)
    at MexicoCfdiReceiptCertifier.prepare (${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/hris/models/payment-receipt/countries/mexico/mexico-cfdi-receipt.certifier.ts:130:30)
    at async ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196/src/hris/models/payment-receipt/countries/mexico/mexico-cfdi-receipt.certifier.spec.ts:98:21
```

### All 13 affected test names

- MexicoCfdiReceiptCertifier prepare returns the signed, unstamped XML as the document to be submitted
- MexicoCfdiReceiptCertifier prepare flags submission as skipped for an entity not cleared to stamp
- MexicoCfdiReceiptCertifier validation logs nothing at boot when the validator loads
- MexicoCfdiReceiptCertifier simulate stamps through the mock with the folio and a SAT wall-clock time
- MexicoCfdiReceiptCertifier simulate maps the simulated stamp onto the generic fields and the Mexico blob
- MexicoCfdiReceiptCertifier submit maps the TFD onto the generic fields and the Mexico blob
- MexicoCfdiReceiptCertifier submit reads FechaTimbrado in the SAT zone, not the server zone
- MexicoCfdiReceiptCertifier submit takes a stamp date that carries its own offset at face value
- MexicoCfdiReceiptCertifier submit translates a PAC rejection into a definite rejection
- MexicoCfdiReceiptCertifier submit leaves a timeout untranslated so the outcome stays unknown
- MexicoCfdiReceiptCertifier submit leaves a SOAP fault untranslated so the outcome stays unknown
- MexicoCfdiReceiptCertifier submit leaves a transport error untranslated so the outcome stays unknown
- MexicoCfdiReceiptCertifier submit leaves a stamp with no TFD untranslated so the outcome stays unknown

## Reproduction and evidence

`FLUX_VALIDATION_ROOT` is the preserved external validation directory containing `flux-harness/`, `release/`, and `flux-node26-pr-1196/`. Raw receipts and frozen binaries remain there; they are not committed to this repository. Set this variable to that directory before reproducing. Local filesystem prefixes in recorded stacks are normalized below; the original JSON receipts are authoritative.

```sh
export FLUX_VALIDATION_ROOT=/path/to/preserved/work/recovered
```

These commands reproduce the **recorded pre-split artifact**, not a future candidate. Run only when intentionally resuming this backlog. Keep the source/binary/config/lock receipts together.

### representative

```sh
python3 ${FLUX_VALIDATION_ROOT}/flux-harness/run-batch-owned.py turbo --root ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196 --turbo-launcher ${FLUX_VALIDATION_ROOT}/flux-harness/run-local-turbo.cjs --binary ${FLUX_VALIDATION_ROOT}/flux-harness/artifacts/cf9a0a5-known-fixes/turbo-test --expected-binary-sha 038a97aeb6ef5f010e79f16b8e612cffa51626875b82cec1b90f4a1085eb6fd7 --source-sha cf9a0a577a642c4ff925a10ad3c5c61f790385e2 --package-root ${FLUX_VALIDATION_ROOT}/release --selection-file ${FLUX_VALIDATION_ROOT}/flux-harness/representative-tests.json --timeout 180 --output-dir ${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-16-REPRO-USE-NEW-DIRECTORY
```

- Metadata: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-16/turbo-metadata.json`
- Standard error: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-16/turbo-stderr.log`
- Standard output: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-16/turbo-stdout.log`
- Full JSON, including all assertions and failure messages: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-16/turbo-result.json`

### additional00

```sh
python3 ${FLUX_VALIDATION_ROOT}/flux-harness/run-batch-owned.py turbo --root ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196 --turbo-launcher ${FLUX_VALIDATION_ROOT}/flux-harness/run-local-turbo.cjs --binary ${FLUX_VALIDATION_ROOT}/flux-harness/artifacts/cf9a0a5-known-fixes/turbo-test --expected-binary-sha 038a97aeb6ef5f010e79f16b8e612cffa51626875b82cec1b90f4a1085eb6fd7 --source-sha cf9a0a577a642c4ff925a10ad3c5c61f790385e2 --package-root ${FLUX_VALIDATION_ROOT}/release --selection-file ${FLUX_VALIDATION_ROOT}/flux-harness/chunked-node26-pr-1196/chunk-00.json --timeout 180 --output-dir ${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-00-REPRO-USE-NEW-DIRECTORY
```

- Metadata: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-00/turbo-metadata.json`
- Standard error: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-00/turbo-stderr.log`
- Standard output: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-00/turbo-stdout.log`
- Full JSON, including all assertions and failure messages: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-00/turbo-result.json`

### additional01

```sh
python3 ${FLUX_VALIDATION_ROOT}/flux-harness/run-batch-owned.py turbo --root ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196 --turbo-launcher ${FLUX_VALIDATION_ROOT}/flux-harness/run-local-turbo.cjs --binary ${FLUX_VALIDATION_ROOT}/flux-harness/artifacts/cf9a0a5-known-fixes/turbo-test --expected-binary-sha 038a97aeb6ef5f010e79f16b8e612cffa51626875b82cec1b90f4a1085eb6fd7 --source-sha cf9a0a577a642c4ff925a10ad3c5c61f790385e2 --package-root ${FLUX_VALIDATION_ROOT}/release --selection-file ${FLUX_VALIDATION_ROOT}/flux-harness/chunked-node26-pr-1196/chunk-01.json --timeout 180 --output-dir ${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-01-REPRO-USE-NEW-DIRECTORY
```

- Metadata: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-01/turbo-metadata.json`
- Standard error: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-01/turbo-stderr.log`
- Standard output: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-additional-01/turbo-stdout.log`
- Full JSON: absent; timeout/incomplete.

### certifier

```sh
python3 ${FLUX_VALIDATION_ROOT}/flux-harness/run-batch-owned.py turbo --root ${FLUX_VALIDATION_ROOT}/flux-node26-pr-1196 --turbo-launcher ${FLUX_VALIDATION_ROOT}/flux-harness/run-local-turbo.cjs --binary ${FLUX_VALIDATION_ROOT}/flux-harness/artifacts/cf9a0a5-known-fixes/turbo-test --expected-binary-sha 038a97aeb6ef5f010e79f16b8e612cffa51626875b82cec1b90f4a1085eb6fd7 --source-sha cf9a0a577a642c4ff925a10ad3c5c61f790385e2 --package-root ${FLUX_VALIDATION_ROOT}/release --selection-file ${FLUX_VALIDATION_ROOT}/flux-harness/pr-1196-certifier-selection.json --timeout 180 --output-dir ${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-certifier-REPRO-USE-NEW-DIRECTORY
```

- Metadata: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-certifier/turbo-metadata.json`
- Standard error: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-certifier/turbo-stderr.log`
- Standard output: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-certifier/turbo-stdout.log`
- Full JSON, including all assertions and failure messages: `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-cf9a0a5-turbo-certifier/turbo-result.json`

## Follow-up checklist

- [ ] Keep #391 deferred to v0.8.5 unless explicitly reprioritized.
- [ ] Before diagnosing additional 00, capture a matching Jest result for each affected spec on the pinned PR/lock.
- [ ] Investigate the four syntax-token load errors as one candidate cluster, without assuming a common cause.
- [ ] Investigate the four `.native` property load errors as one candidate cluster, without assuming a common cause.
- [ ] Investigate the webhook mock/authorization failures with the existing mock mapping intact; do not weaken authentication assertions.
- [ ] Resolve the additional 01 timeout and cleanup uncertainty before treating that batch as complete.
- [ ] Re-run affected cases on a future frozen artifact; do not combine passing results from different builds into a full-suite pass claim.

## Representative rerun after the runtime split

The frozen reviewed candidate `f1708af6ee947d87e1d354504ad375b7a8318e50`
(binary SHA-256 `d0a8fee9797dd43c96c37d288c6536e891f03c0c24da1a6bec3c729fc328b798`)
passed all **595 tests in the same 16-file Flux selection**, with no failed assertions or
load errors. The Flux PR, lockfile, selection, and external launcher were unchanged.
The receipt is `${FLUX_VALIDATION_ROOT}/flux-harness/results/pr-1196-f1708af-runtime-reviewed-16/`.
This was a bounded regression rerun; neither the full backend suite nor the previously
failing additional batches were rerun. Their unresolved results above remain historical
evidence, not a claim about the new artifact.

The runtime split also has a committed 28-combination Vitest/Jest settings matrix
(40 JS tests), mixed-profile and same-profile reuse tests, failed-load child cleanup,
snapshot-on/off controls, and mutable-export regression tests. The extra review found
and fixed nearest-config precedence and internal resource-constructor problems. The
reuse edge tests additionally exposed CommonJS caller provenance losing file-owned
hook cleanup; the reviewed candidate retains those caller filenames.

All four full frontend suites passed on this same binary:

| Repository | Passed | Skipped | Files |
| --- | ---: | ---: | ---: |
| payroll-app | 26,360 | 0 | 2,051 |
| ui-design-components | 7,408 | 0 | 464 |
| standalone-chat-app | 785 | 0 | 68 |
| website-global | 1,647 | 5 | 114 |

Payroll uses app commit `49da952428cff8c9aec3bf46b56d3f317378e009` over staging
`9ca35e9`: two Node-only E2E-helper unit-test files declare the Node environment,
and a setup entry loads browser fixtures only for DOM tests. Both turbo-test and
original Vitest passed the same 18 targeted Node/DOM controls. No package or lockfile
version was changed. Website has two additional passing HMAC tests that previously
skipped; the total test identities are unchanged. No tests were excluded to obtain
these results, and these correctness runs make no performance claim.

The consolidated frontend receipt is
`${FLUX_VALIDATION_ROOT}/bench/latest/final-f1708af-four-app-correctness.json`.
