//! Runner flags end-to-end: --bail, --shard, --retry, --testTimeout (flag and config), -t,
//! -u, --silent, --allowOnly/--no-allowOnly, worker-count flags, unknown-flag warnings, the
//! duration history (load → slowest-first schedule → save) and the transient / fresh-isolate
//! retry loops.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project, Run};

fn tt(p: &Project, args: &[&str]) -> Run {
    turbo_test(&p.dir, args, &[])
}

fn names(r: &Run) -> Vec<String> {
    json_report(r)["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

/// A test file that appends `tag` to `<project>/run.log` each time it is evaluated. (Run these
/// with `--jobs 1`: the runtime's `appendFileSync` is not atomic across concurrent workers.)
fn logging(tag: &str, body: &str) -> String {
    format!("import fs from 'node:fs'\nfs.appendFileSync('run.log', '{tag}\\n')\n{body}\n")
}

fn log(p: &Project) -> Vec<String> {
    std::fs::read_to_string(p.path("run.log"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

const PASS: &str = "test('p', () => { expect(1).toBe(1) })";
const FAIL: &str = "test('f', () => { expect(1).toBe(2) })";

#[test]
fn bail_stops_after_n_failures() {
    let p = Project::new("flag-bail");
    for f in ["f1", "f2", "f3"] {
        p.file(&format!("{f}.test.ts"), FAIL);
    }
    let r = tt(&p, &["--bail", "1", "--jobs", "1"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout
            .contains("\n Test Files  1 failed (3)\n      Tests  1 failed (1)\n"),
        "{}",
        r.all()
    );
    // without --bail (or --bail 0 / invalid) every file runs
    for args in [
        &["--jobs", "1"][..],
        &["--bail", "0", "-j", "1"],
        &["--bail", "x", "-j", "1"],
    ] {
        let r = tt(&p, args);
        assert!(
            r.stdout
                .contains(" Test Files  3 failed (3)\n      Tests  3 failed (3)\n"),
            "{args:?}: {}",
            r.all()
        );
    }
}

#[test]
fn shard_partitions_discovered_files() {
    let p = Project::new("flag-shard");
    for f in ["a", "b", "c", "d"] {
        p.file(&format!("{f}.test.ts"), PASS);
    }
    let r = tt(&p, &["--shard", "1/2", "--reporter", "json"]);
    assert_eq!(names(&r), ["a.test.ts", "c.test.ts"]);
    assert_eq!(json_report(&r)["numTotalTestSuites"], 2);
    let r = tt(&p, &["--shard", "2/2", "--reporter", "json"]);
    assert_eq!(names(&r), ["b.test.ts", "d.test.ts"]);
    // invalid shards run everything
    for s in ["3/2", "0/2", "x/2", "12"] {
        let r = tt(&p, &["--shard", s, "--reporter", "json"]);
        assert_eq!(names(&r).len(), 4, "{s}");
        assert_eq!(r.code, Some(0));
    }
}

#[test]
fn retry_count() {
    let p = Project::new("flag-retry");
    p.file(
        "flaky.test.ts",
        "let n = 0\ntest('flaky', () => { n++; expect(n).toBe(3) })\n",
    );
    let r = tt(&p, &["--retry", "2"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    for args in [&[][..], &["--retry", "1"], &["--retry", "many"]] {
        let r = tt(&p, args);
        assert_eq!(r.code, Some(1), "{args:?}: {}", r.all());
    }
}

#[test]
fn test_timeout_flag_and_config() {
    let p = Project::new("flag-timeout");
    p.file(
        "slow.test.ts",
        "test('slow', async () => { await new Promise(r => setTimeout(r, 300)) })\n",
    );
    let r = tt(&p, &["--testTimeout", "50"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains("✗ slow: test timed out in 50ms"),
        "{}",
        r.all()
    );
    // an invalid value is ignored (vitest's 5000ms default applies)
    assert_eq!(tt(&p, &["--testTimeout", "soon"]).code, Some(0));
    // the config's test.testTimeout is honoured; an explicit flag wins over it
    p.file(
        "vitest.config.ts",
        "export default { test: { testTimeout: 40 } }\n",
    );
    let r = tt(&p, &[]);
    assert!(r.stdout.contains("timed out in 40ms"), "{}", r.all());
    assert_eq!(tt(&p, &["--testTimeout=5000"]).code, Some(0));
    // turbo-test#32 item 2: the inline form is honoured too (and, being passed, the config's
    // value is not injected over it)
    let r = tt(&p, &["--testTimeout=60"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(r.stdout.contains("timed out in 60ms"), "{}", r.all());
}

#[test]
fn inline_value_flags_end_to_end() {
    // turbo-test#32 item 2: `--reporter=json` was warned about and ignored (text reporter ran).
    let p = Project::new("flag-inline");
    p.file("a.test.js", "test('a', () => {})\n");
    let r = tt(&p, &["--reporter=json", "a.test.js"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        !r.stderr.contains("ignoring unsupported flag"),
        "{}",
        r.all()
    );
    assert_eq!(json_report(&r)["numPassedTests"], 1);
    let r = tt(
        &p,
        &["--reporter=tap", "--outputFile=out/r.tap", "a.test.js"],
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    let tap = std::fs::read_to_string(p.path("out/r.tap")).unwrap();
    assert!(tap.starts_with("TAP version 13\n1..1\nok 1 - a"), "{tap}");
}

#[test]
fn name_pattern_filter() {
    let p = Project::new("flag-t");
    p.file(
        "a.test.ts",
        "test('alpha', () => {})\ntest('beta', () => { expect(1).toBe(2) })\n",
    );
    for flag in ["-t", "--testNamePattern"] {
        let r = tt(&p, &[flag, "alph", "--reporter", "json"]);
        assert_eq!(r.code, Some(0), "{}", r.all());
        assert_eq!(json_report(&r)["numPassedTests"], 1);
        assert_eq!(json_report(&r)["numFailedTests"], 0);
    }
    assert_eq!(tt(&p, &[]).code, Some(1));
}

#[test]
fn update_snapshots() {
    let p = Project::new("flag-u");
    p.file(
        "s.test.ts",
        "test('snap', () => { expect({ v: 1 }).toMatchSnapshot() })\n",
    );
    assert_eq!(tt(&p, &[]).code, Some(0), "first run writes the snapshot");
    let snap = p.path("__snapshots__/s.test.ts.snap");
    assert!(std::fs::read_to_string(&snap).unwrap().contains("\"v\": 1"));
    p.file(
        "s.test.ts",
        "test('snap', () => { expect({ v: 2 }).toMatchSnapshot() })\n",
    );
    let r = tt(&p, &[]);
    // vitest's message (turbo-test#71)
    assert!(
        r.stdout.contains("✗ snap: Snapshot `snap 1` mismatched"),
        "{}",
        r.all()
    );
    for flag in ["-u", "--update"] {
        assert_eq!(tt(&p, &[flag]).code, Some(0), "{flag}");
        assert!(std::fs::read_to_string(&snap).unwrap().contains("\"v\": 2"));
    }
    assert_eq!(tt(&p, &[]).code, Some(0));
}

#[test]
fn silent_and_allow_only() {
    let p = Project::new("flag-silent");
    p.file(
        "log.test.ts",
        "test('l', () => { console.log('NOISY-LINE') })\n",
    );
    p.file(
        "only.test.ts",
        "test.only('o', () => {})\ntest('p', () => {})\n",
    );
    let r = tt(&p, &["log.test.ts"]);
    assert!(r.stdout.contains("NOISY-LINE"), "{}", r.all());
    let r = tt(&p, &["--silent", "log.test.ts"]);
    assert_eq!(r.code, Some(0));
    assert!(
        !r.stdout.contains("NOISY-LINE") && !r.stderr.contains("NOISY-LINE"),
        "{}",
        r.all()
    );

    assert_eq!(tt(&p, &["only.test.ts"]).code, Some(0));
    assert_eq!(
        tt(&p, &["only.test.ts", "--no-allowOnly", "--allowOnly"]).code,
        Some(0)
    );
    let r = tt(&p, &["only.test.ts", "--allowOnly", "--no-allowOnly"]);
    assert_eq!(r.code, Some(1));
    assert!(
        r.stdout
            .contains("Unexpected .only modifier. Remove it or pass --allowOnly argument"),
        "{}",
        r.all()
    );
}

#[test]
fn worker_count_flags_and_unknown_flag_warning() {
    let p = Project::new("flag-jobs");
    p.file("a.test.ts", PASS);
    p.file("b.test.ts", PASS);
    let jobs = |args: &[&str], env: &[(&str, &str)]| {
        let r = turbo_test(&p.dir, args, env);
        assert_eq!(r.code, Some(0), "{}", r.all());
        let s = r
            .stdout
            .split(" jobs |")
            .next()
            .unwrap()
            .rsplit("| ")
            .next()
            .unwrap()
            .to_string();
        s.parse::<usize>().unwrap()
    };
    assert_eq!(jobs(&[], &[]), 2, "TURBO_JOBS=2 from the harness");
    assert_eq!(jobs(&[], &[("TURBO_JOBS", "1")]), 1);
    assert!(
        jobs(&[], &[("TURBO_JOBS", "x")]) >= 1,
        "unparsable TURBO_JOBS → host cores (capped by files)"
    );
    assert_eq!(jobs(&["--jobs", "1"], &[]), 1);
    assert_eq!(jobs(&["-j", "8"], &[]), 2, "capped at the file count");
    assert_eq!(jobs(&["--maxWorkers", "1", "--minWorkers", "4"], &[]), 1);
    assert_eq!(jobs(&["--no-file-parallelism"], &[]), 1);
    assert_eq!(jobs(&["--jobs", "0"], &[]), 1, "at least one worker");
    assert_eq!(jobs(&["--jobs", "lots"], &[]), 2);

    let r = tt(&p, &["--frobnicate", "-Z", "a.test.ts"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stderr.contains("turbo-test: ignoring unsupported flag '--frobnicate'\nturbo-test: ignoring unsupported flag '-Z'\n"), "{}", r.all());
    assert!(r.stdout.contains("1 files | 1 passed"), "{}", r.all());
}

#[test]
fn duration_history_orders_slowest_first_and_is_saved() {
    let p = Project::new("flag-hist");
    p.file("a.test.ts", &logging("a", PASS));
    p.file("b.test.ts", &logging("b", PASS));
    p.file("c.test.ts", &logging("c", PASS));
    let hist = p.path(".tt-tmp/turbo-test-cache/durations.tsv");
    std::fs::create_dir_all(hist.parent().unwrap()).unwrap();
    let abs = |f: &str| p.path(f).display().to_string();
    std::fs::write(
        &hist,
        format!(
            "{}\t5\n{}\t900\nbogus line\n",
            abs("a.test.ts"),
            abs("c.test.ts")
        ),
    )
    .unwrap();
    let r = tt(&p, &["--jobs", "1"]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(
        log(&p),
        ["c", "a", "b"],
        "slowest recorded first, unknown last"
    );
    // the history now has every file that ran, with fresh (finite) durations
    let saved = std::fs::read_to_string(&hist).unwrap();
    for f in ["a.test.ts", "b.test.ts", "c.test.ts"] {
        let line = saved
            .lines()
            .find(|l| l.starts_with(&format!("{}\t", abs(f))))
            .unwrap_or_else(|| panic!("{f}: {saved}"));
        let ms: f64 = line.split('\t').nth(1).unwrap().parse().unwrap();
        assert!((0.0..900.0).contains(&ms), "{line}");
    }
    assert!(!saved.contains("bogus"));
}

#[test]
fn transient_and_fresh_isolate_retries() {
    let p = Project::new("flag-transient");
    p.file("fail.test.ts", &logging("fail", FAIL));
    p.file(
        "partial.test.ts",
        &logging("partial", &format!("{PASS}\n{FAIL}")),
    );
    p.file("pass.test.ts", &logging("pass", PASS));
    let count = |p: &Project, tag: &str| log(p).iter().filter(|l| *l == tag).count();

    // fresh isolates: an all-failing file is re-run twice (3 evaluations); others once
    let r = tt(&p, &["--isolate", "--jobs", "1"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        (count(&p, "fail"), count(&p, "partial"), count(&p, "pass")),
        (3, 1, 1),
        "{:?} {}",
        log(&p),
        r.all()
    );

    // isolate reuse: a still-failing file also gets the authoritative fresh-isolate re-run
    std::fs::remove_file(p.path("run.log")).unwrap();
    let r = tt(&p, &["--no-isolate", "--jobs", "1"]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains("FAIL  ") && r.stdout.contains("(0 passed, 1 failed)"),
        "{}",
        r.all()
    );
    assert!(
        count(&p, "fail") > 3,
        "fresh retry under reuse: {:?}",
        log(&p)
    );
    assert!(
        count(&p, "partial") > 1,
        "partial failure gets a fresh retry under reuse: {:?}",
        log(&p)
    );
    assert_eq!(count(&p, "pass"), 1);

    // --isolate wins over a config `isolate: false`
    p.file(
        "vitest.config.ts",
        "export default { test: { isolate: false } }\n",
    );
    std::fs::remove_file(p.path("run.log")).unwrap();
    tt(&p, &["--isolate", "--jobs", "1", "fail.test.ts"]);
    assert_eq!(count(&p, "fail"), 3);
}
