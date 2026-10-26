//! Run-control knobs the runner plumbs from env (set by the CLI) into each fresh context:
//! snapshots and `-u`, `-t` name patterns, `--testTimeout`, `--retry`, `--silent`,
//! `--no-allowOnly`, TURBO_STACK, V8 flags, the snapshot code-handling switch, DOM on/off, and the
//! persisted CJS bytecode cache.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn plain(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", &format!(r#"{{"name":"{name}"}}"#));
    p
}

#[test]
fn snapshots_are_written_compared_and_updated() {
    let p = plain("rb-flags-snap");
    p.file(
        "snap.test.js",
        "test('snap', () => { expect({ v: globalThis.__V ?? 1 }).toMatchSnapshot(); });\n",
    );
    let snap = p.path("__snapshots__/snap.test.js.snap");
    let first = turbo_test(&p.dir, &[], &[]);
    assert_eq!(first.code, Some(0), "{}", first.all());
    let written = std::fs::read_to_string(&snap).expect("snapshot file written");
    assert!(written.contains("\"v\": 1"), "{written}");

    // change the value: the stored snapshot no longer matches
    p.file(
        "snap.test.js",
        "test('snap', () => { expect({ v: 2 }).toMatchSnapshot(); });\n",
    );
    let mismatch = turbo_test(&p.dir, &[], &[]);
    assert_eq!(mismatch.code, Some(1), "{}", mismatch.all());
    assert!(mismatch.stdout.contains("snap: "), "{}", mismatch.all());
    assert_eq!(std::fs::read_to_string(&snap).unwrap(), written);

    // -u rewrites it
    let update = turbo_test(&p.dir, &["-u"], &[]);
    assert_eq!(update.code, Some(0), "{}", update.all());
    let updated = std::fs::read_to_string(&snap).unwrap();
    assert!(updated.contains("\"v\": 2"), "{updated}");
}

#[test]
fn name_pattern_timeout_retry_silent_and_only() {
    let p = plain("rb-flags-knobs");
    p.file(
        "knobs.test.js",
        r#"describe('grp', () => {
  it('alpha [x]', () => { console.log('LOUD-LOG'); expect(1).toBe(1); });
  it('beta', () => { expect(1).toBe(1); });
});
"#,
    );
    p.file(
        "slow.test.js",
        "test('hangs', async () => { await new Promise((r) => setTimeout(r, 50)); });\n",
    );
    p.file(
        "flaky.test.js",
        "globalThis.__n = 0;\ntest('third time lucky', () => { globalThis.__n++; expect(globalThis.__n).toBeGreaterThanOrEqual(3); });\n",
    );
    p.file(
        "only.test.js",
        "test.only('focused', () => {});\ntest('other', () => {});\n",
    );

    // -t: a regex with metacharacters only runs the matching test
    let r = turbo_test(
        &p.dir,
        &[
            "--reporter",
            "verbose",
            "-t",
            "alpha \\[x\\]",
            "knobs.test.js",
        ],
        &[],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("(1 passed, 0 failed)"), "{}", r.all());
    assert!(r.stdout.contains("grp > alpha [x]"), "{}", r.all());
    // the filtered-out test is reported skipped, as vitest's (`↓ grp > beta`)
    assert!(r.stdout.contains("   - grp > beta (0ms)"), "{}", r.all());
    assert!(!r.stdout.contains("✓ grp > beta"), "{}", r.all());

    // console output is printed by default and suppressed by --silent
    let loud = turbo_test(&p.dir, &["knobs.test.js"], &[]);
    assert!(loud.stdout.contains("LOUD-LOG"), "{}", loud.all());
    let quiet = turbo_test(&p.dir, &["--silent", "knobs.test.js"], &[]);
    assert_eq!(quiet.code, Some(0), "{}", quiet.all());
    assert!(!quiet.stdout.contains("LOUD-LOG"), "{}", quiet.all());

    // --testTimeout below the test's duration fails it; a non-numeric env value is ignored
    let to = turbo_test(&p.dir, &["--testTimeout", "10", "slow.test.js"], &[]);
    assert_eq!(to.code, Some(1), "{}", to.all());
    assert!(to.stdout.contains("timed out"), "{}", to.all());
    let bad = turbo_test(&p.dir, &["slow.test.js"], &[("TURBO_TEST_TIMEOUT", "soon")]);
    assert_eq!(bad.code, Some(0), "{}", bad.all());

    // --retry 2 lets the flaky test pass on its third attempt; without it, it fails
    let flaky = turbo_test(&p.dir, &["flaky.test.js"], &[]);
    assert_eq!(flaky.code, Some(1), "{}", flaky.all());
    let retried = turbo_test(&p.dir, &["--retry", "2", "flaky.test.js"], &[]);
    assert_eq!(retried.code, Some(0), "{}", retried.all());
    let bad = turbo_test(&p.dir, &["flaky.test.js"], &[("TURBO_TEST_RETRY", "x")]);
    assert_eq!(bad.code, Some(1), "{}", bad.all());

    // .only is allowed by default and rejected by --no-allowOnly
    let only = turbo_test(&p.dir, &["only.test.js"], &[]);
    assert_eq!(only.code, Some(0), "{}", only.all());
    let forbid = turbo_test(&p.dir, &["--no-allowOnly", "only.test.js"], &[]);
    assert_eq!(forbid.code, Some(1), "{}", forbid.all());
}

#[test]
fn turbo_stack_appends_the_failure_stack() {
    let p = plain("rb-flags-stack");
    p.file(
        "boom.test.js",
        "function deepHelper() { throw new Error('kaboom'); }\ntest('boom', () => { deepHelper(); });\n",
    );
    let plain_run = turbo_test(&p.dir, &[], &[]);
    assert_eq!(plain_run.code, Some(1));
    assert!(
        plain_run.stdout.contains("boom: kaboom"),
        "{}",
        plain_run.all()
    );
    assert!(!plain_run.stdout.contains("@ "), "{}", plain_run.all());
    let stacked = turbo_test(&p.dir, &[], &[("TURBO_STACK", "1")]);
    assert_eq!(stacked.code, Some(1));
    assert!(stacked.stdout.contains("@ "), "{}", stacked.all());
    assert!(stacked.stdout.contains("deepHelper"), "{}", stacked.all());
}

#[test]
fn v8_flags_and_snapshot_code_handling() {
    let p = plain("rb-flags-v8");
    let with_gc = |want: &str| {
        p.file(
            "gc.test.js",
            &format!(
                "test('gc exposure', () => {{ expect(typeof globalThis.gc).toBe('{want}'); }});\n"
            ),
        );
    };
    with_gc("function");
    let r = turbo_test(&p.dir, &[], &[("TURBO_V8_FLAGS", "--expose-gc")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    with_gc("undefined");
    let r = turbo_test(&p.dir, &[], &[("TURBO_V8_FLAGS", "  ")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    // Keeping the framework bytecode in the snapshot changes nothing observable
    let r = turbo_test(&p.dir, &[], &[("TURBO_SNAP_KEEP", "1")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn dom_is_installed_only_for_files_that_need_it() {
    let p = plain("rb-flags-dom");
    p.file(
        "dom.test.js",
        "test('dom', () => { document.body.innerHTML = '<p id=x>hi</p>'; expect(document.getElementById('x').textContent).toBe('hi'); });\n",
    );
    p.file(
        "nodom.test.js",
        "test('no dom', () => { expect(typeof globalThis.HTMLElement).toBe('undefined'); });\n",
    );
    p.file(
        "forced-node.test.js",
        "// @vitest-environment node\ntest('no dom', () => { expect(typeof globalThis.HTMLElement).toBe('undefined'); });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);
    assert_eq!(j["numPassedTests"], 3, "{}", r.all());
    assert_eq!(j["success"], true);
}

fn code_cache_files(p: &Project) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(p.path(".tt-cache"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|f| {
                    let n = f.file_name().unwrap().to_string_lossy().to_string();
                    n.starts_with("cc-") && n.ends_with(".bin")
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn cjs_bytecode_cache_is_written_reused_and_survives_corruption() {
    let p = plain("rb-flags-cc");
    p.with_esbuild();
    p.file(
        "src/lib.ts",
        "export const add = (a: number, b: number) => a + b;\n",
    );
    p.file(
        "cc.test.ts",
        "import { add } from './src/lib';\ntest('add', () => { expect(add(2, 3)).toBe(5); });\n",
    );
    let off = turbo_test(&p.dir, &[], &[("TURBO_NO_CODE_CACHE", "1")]);
    assert_eq!(off.code, Some(0), "{}", off.all());
    assert!(
        code_cache_files(&p).is_empty(),
        "TURBO_NO_CODE_CACHE writes no bytecode"
    );

    let cold = turbo_test(&p.dir, &[], &[]);
    assert_eq!(cold.code, Some(0), "{}", cold.all());
    let files = code_cache_files(&p);
    assert!(!files.is_empty(), "cold run persists bytecode");
    let sizes: Vec<u64> = files.iter().map(|f| f.metadata().unwrap().len()).collect();
    assert!(sizes.iter().all(|&s| s > 0));

    // warm: consumed, identical result, nothing rewritten
    let warm = turbo_test(&p.dir, &[], &[]);
    assert_eq!(warm.code, Some(0), "{}", warm.all());
    assert_eq!(code_cache_files(&p), files);

    // empty cache entries are ignored and regenerated
    for f in &files {
        std::fs::write(f, b"").unwrap();
    }
    let empty = turbo_test(&p.dir, &[], &[]);
    assert_eq!(empty.code, Some(0), "{}", empty.all());
    assert!(files.iter().all(|f| f.metadata().unwrap().len() > 0));

    // garbage is rejected by V8, which recompiles
    for f in &files {
        std::fs::write(f, b"definitely not v8 bytecode").unwrap();
    }
    let junk = turbo_test(&p.dir, &[], &[]);
    assert_eq!(junk.code, Some(0), "{}", junk.all());
}

#[test]
#[ignore = "turbo-test#75: fails until the fix"]
fn only_is_forbidden_on_ci_unless_allow_only() {
    // turbo-test#75: vitest 4.1.12 defaults allowOnly to !isCI (std-env): on CI a stray `.only`
    // fails the run unless --allowOnly is passed. Checked against vitest: CI=1 and
    // GITHUB_ACTIONS=1 exit 1, `CI=1 --allowOnly` and no CI variable exit 0.
    let p = plain("rb-flags-only-ci");
    p.file(
        "o.test.js",
        "test.only('one', () => { expect(1).toBe(1); });\ntest('two', () => {});\n",
    );
    for env in [[("CI", "1")], [("GITHUB_ACTIONS", "1")]] {
        let r = turbo_test(&p.dir, &["o.test.js"], &env);
        assert_eq!(r.code, Some(1), "{env:?}: {}", r.all());
        let r = turbo_test(&p.dir, &["o.test.js", "--allowOnly"], &env);
        assert_eq!(r.code, Some(0), "{env:?} --allowOnly: {}", r.all());
        let r = turbo_test(
            &p.dir,
            &["o.test.js", "--allowOnly", "--no-allowOnly"],
            &env,
        );
        assert_eq!(r.code, Some(1), "{env:?} --no-allowOnly last: {}", r.all());
    }
    let r = turbo_test(&p.dir, &["o.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}
