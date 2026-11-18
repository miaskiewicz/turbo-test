# Flux APIs final compatibility evidence

The full baseline completed. The final impact pass produced results for **1,403 files; 128 remain unknown**. No chunks are unstarted, but this does **not** establish that all 1,531 files executed. Backend green is not a v0.8.4 release gate.

| Run | Passed files | Test-failing files | Load errors | Unknown |
| --- | ---: | ---: | ---: | ---: |
| Baseline `0961554` | 1,359 | 90 | 82 | 0 |
| Impact `8146128` | 1,266 | 65 | 72 | 128 |

| Reported test cases | Registered | Passed | Failed | Skipped |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 32,890 | 31,956 | 911 | 23 |
| Impact | 30,789 | 30,211 | 555 | 23 |

These counts exclude unreported tests in load-error/unknown files; they are test cases, not individual assertions. The harness added no skips.

Focused real Jest controls passed **172/172 affected files and 7,123/7,123 tests**, with zero skips/timeouts. This validates only that subset, not a full-suite Jest run. Among reported impact outcomes, 29 previously nonpassing files now pass and 257 previously failed test IDs pass. No formerly passing file is newly reported failed/load-error, but 128 unknown files and 167 previously registered IDs not re-reported prevent a blanket regression conclusion.

Both candidates used the same fresh canonical Jest-discovered 1,531-file manifest, unchanged PR #1196 source/lock/config, launcher, sanitized environment and at most two workers. Full source and binary hashes, per-file/test-ID transitions and receipt locations are in `${VALIDATION_ROOT}/flux-harness/results/impact-8146128-node26-pr1196/FINAL_SUMMARY.json` and `${OUTPUT_ROOT}/flux-apis-evidence/artifact-index.json`. No performance comparison is claimed.

The original chunk 01 timed out after 180seconds; cleanup then raised `Operation not permitted`. Chunk 02 was canceled with its observed owned groups cleaned. Chunk 01's receipt did not establish detached-child quiescence. Its launcher/group were subsequently absent, and root independently verified zero live native processes for the exact artifact before permitting chunks 03–23 to finish. The two original chunks remain unknown; no stale process-group signals were sent.

**Automatic approval review rejected** the optional 128-file fallback as exceeding its interpretation of one approved eight-file batch and the time budget. The command did not execute. There was no retry, workaround or request to broaden scope.

Known larger work remains deferred to v0.8.5, including #391 and #401–#406. #402's real Jest hook behavior prevents a safe literal alias. Newly observed error messages are preserved in the receipts without further cause investigation. No additional tests, probes or fixes followed this final pass; no full-Flux-green, CI, tag or publication claim is made.

`${VALIDATION_ROOT}` denotes the validation workspace containing `flux-harness/`.
`${OUTPUT_ROOT}` denotes the preserved `outputs/` evidence directory. Raw receipts
remain outside this repository; this Markdown summary records their scope and limits.
