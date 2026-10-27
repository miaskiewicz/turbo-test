//! The `expect` surface against vitest 4.1.12 (turbo-test#152–#155): the members of `expect`,
//! `expect.not`, every asymmetric matcher and the assertion chains (`expect(x)`, `.not`,
//! `.resolves`, `.rejects`, `expect.soft`), and what each new member does. The data in
//! `tests/data/expect_surface` was recorded with vitest 4.1.12 on Node 26 (`*.vitest.*`).
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};
use serde_json::Value;
use std::collections::BTreeSet;

/// Members vitest has and turbo-test does not, each tracked by its own issue (vitest.compat.md):
/// none since expect.poll (turbo-test#161) and expect.addSnapshotSerializer (#162).
const KNOWN_MISSING: [(&str, &str); 0] = [];

#[test]
fn expect_members_are_vitests() {
    // turbo-test#153 / #154 / #155: expect.closeTo / schemaMatching / addEqualityTesters / fail /
    // unreachable / getState / setState / toSatisfy were missing, expect.extend made no
    // asymmetric matchers, the asymmetric matchers had none of vitest's members, and 17
    // matchers (toBeCalled, toHaveReturnedTimes, toHaveResolved*, toHaveBeenCalledBefore, ...)
    // were missing while a stray `toStrictEqual_alias` existed. turbo-test#160: chai's BDD chain
    // (`to`, `be`, `deep`, `equal`, `include`, ...), its sinon-style members, `expect.assert`
    // and chai's `__flags` were missing.
    let p = Project::new("rc-expect-members");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "members.test.js",
        include_str!("data/expect_surface/members.test.js"),
    );
    let r = turbo_test(&p.dir, &["members.test.js"], &[("OUT", "tt.json")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got: Value =
        serde_json::from_str(&std::fs::read_to_string(p.path("tt.json")).unwrap()).unwrap();
    let want: Value =
        serde_json::from_str(include_str!("data/expect_surface/members.vitest.json")).unwrap();
    let set = |v: &Value| -> BTreeSet<String> {
        v.as_array()
            .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    };
    let mut missing = Vec::new();
    let mut extra = Vec::new();
    let mut compare = |label: &str, w: &Value, g: &Value| {
        let (w, g) = (set(w), set(g));
        for m in w.difference(&g) {
            missing.push(format!("{label} {m}"));
        }
        for m in g.difference(&w) {
            extra.push(format!("{label} {m}"));
        }
    };
    compare("expect", &want["expect"], &got["expect"]);
    compare("expect.not", &want["expectNot"], &got["expectNot"]);
    for (k, w) in want["asym"].as_object().unwrap() {
        compare(&format!("asym.{k}"), w, &got["asym"][k]);
    }
    // the chains: every member of vitest's chai Assertion — its matchers, chai's BDD chain and
    // the sinon-style members (turbo-test#160)
    for k in ["expect", "not", "resolves", "rejects", "soft"] {
        compare(&format!("chain.{k}"), &want["chain"][k], &got["chain"][k]);
    }
    let known: Vec<String> = KNOWN_MISSING
        .iter()
        .map(|(l, m)| format!("{l} {m}"))
        .collect();
    assert_eq!(missing, known, "missing members");
    assert_eq!(extra, Vec::<String>::new(), "members vitest does not have");
    assert_eq!(got["asymStr"], want["asymStr"]);
}

#[test]
fn expect_members_behave_as_vitests() {
    // Each row is [case, result] as vitest 4.1.12 records it: 'assertion' when a matcher passes
    // (it returns the assertion), [name, message] when it throws, ['value', json] otherwise.
    // Covers the asymmetric matchers' members and errors (#154), closeTo / schemaMatching /
    // toSatisfy / expect.extend asymmetric forms and their printing (#153, #155), a user object
    // with asymmetricMatch, expect.addEqualityTesters in every equality matcher (#155), fail /
    // unreachable / getState / setState, the return / resolve / call-order matchers and the
    // spies' invocationCallOrder / settledResults, chai's `Invalid Chai property` for an unknown
    // member, .resolves / .rejects on a non-promise, and toMatchObject's subset actual (#152).
    for esbuild in [false, true] {
        let p = Project::new("rc-expect-surface");
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "type": "module" }"#);
        p.file(
            "surface.test.js",
            include_str!("data/expect_surface/surface.test.js"),
        );
        let r = turbo_test(&p.dir, &["surface.test.js"], &[]);
        assert_eq!(r.code, Some(0), "{}", r.all());
        let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
        let want = include_str!("data/expect_surface/surface.vitest.txt");
        for (g, w) in got.lines().zip(want.lines()) {
            assert_eq!(g, w, "esbuild={esbuild}");
        }
        assert_eq!(got.lines().count(), want.lines().count());
    }
}

#[test]
fn to_match_object_diff_is_vitests() {
    // turbo-test#152: a failed toMatchObject printed `+ Received:` / `undefined` instead of the
    // diff against vitest's getObjectSubset of the received value; an asymmetric closeTo diffs
    // as its toAsymmetricMatcher text. Expected: vitest 4.1.12's default reporter (NO_COLOR).
    let p = Project::new("rc-match-object-diff");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "diff.test.js",
        include_str!("data/expect_surface/diff.test.js"),
    );
    let r = turbo_test(
        &p.dir,
        &["diff.test.js", "--reporter", "default"],
        &[("NO_COLOR", "1"), ("TURBO_LEGACY_REPORTER", "0")],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    for block in [
        "AssertionError: expected { x: 3, y: { z: 1, w: 2 }, q: 1 } to match object { x: Any<String>, y: { z: 1 } }\n(1 matching property omitted from actual)\n\n- Expected\n+ Received\n\n  {\n-   \"x\": Any<String>,\n+   \"x\": 3,\n    \"y\": {\n      \"z\": 1,\n    },\n  }\n",
        "AssertionError: expected { a: 1.1 } to deeply equal { a: NumberCloseTo 1 (2 digits) }\n\n- Expected\n+ Received\n\n  {\n-   \"a\": NumberCloseTo 1 (2 digits),\n+   \"a\": 1.1,\n  }\n",
    ] {
        assert!(r.all().contains(block), "missing {block:?} in\n{}", r.all());
    }
}

#[test]
fn chai_chain_and_assert_are_vitests() {
    // turbo-test#160: vitest's expect(x) is a chai Assertion. turbo-test had no BDD chain
    // (`expect(1).to.equal(1)`: Invalid Chai property: to), no sinon-style members, no
    // `assert` / `chai` / `should` exports of `vitest` and no expect.assert, and its matcher
    // errors were not chai AssertionErrors. Rows recorded with vitest 4.1.12 (as
    // expect_members_behave_as_vitests).
    for esbuild in [false, true] {
        let p = Project::new("rc-chai-chain");
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "type": "module" }"#);
        p.file(
            "chain.test.js",
            include_str!("data/expect_surface/chain.test.js"),
        );
        let r = turbo_test(&p.dir, &["chain.test.js"], &[]);
        assert_eq!(r.code, Some(0), "{}", r.all());
        let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
        let want = include_str!("data/expect_surface/chain.vitest.txt");
        for (g, w) in got.lines().zip(want.lines()) {
            assert_eq!(g, w, "esbuild={esbuild}");
        }
        assert_eq!(got.lines().count(), want.lines().count());
    }
}

#[test]
fn a_missing_chai_fails_loudly() {
    // turbo-test#160: the runtime is built on chai; without it (TURBO_CHAI unset or wrong — cli.js
    // passes the chai this package depends on) the run stops with an error, never a fallback.
    let p = Project::new("rc-no-chai");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "a.test.js",
        "import { test, expect } from 'vitest'\ntest('a', () => { expect(1).to.equal(1) })\n",
    );
    let missing = p.path("no-such-chai/index.js");
    let r = turbo_test(
        &p.dir,
        &["a.test.js"],
        &[("TURBO_CHAI", missing.to_str().unwrap())],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    let want = format!(
        "turbo-test: cannot read chai at {}: No such file or directory (os error 2)\nturbo-test needs the `chai` package it depends on (vitest's expect is chai): run it through its npm launcher (cli.js), which passes TURBO_CHAI, or set TURBO_CHAI to chai's index.js.\n",
        missing.display()
    );
    assert_eq!(r.stderr, want, "{}", r.all());
    assert!(!r.stdout.contains("passed"), "{}", r.all());
    let advice = "turbo-test needs the `chai` package it depends on (vitest's expect is chai): run it through its npm launcher (cli.js), which passes TURBO_CHAI, or set TURBO_CHAI to chai's index.js.\n";
    // TURBO_CHAI unset
    let r = common::run_exe_unset(
        env!("CARGO_BIN_EXE_turbo-test"),
        &p.dir,
        &["a.test.js"],
        &[],
        &["TURBO_CHAI"],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        r.stderr,
        format!("turbo-test: TURBO_CHAI is not set\n{advice}"),
        "{}",
        r.all()
    );
    // a file that is not an ES module the emitter can lower
    let bad = p.file("bad-chai/index.js", "export const = ;\n");
    let r = turbo_test(
        &p.dir,
        &["a.test.js"],
        &[("TURBO_CHAI", bad.to_str().unwrap())],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        r.stderr,
        format!(
            "turbo-test: cannot lower chai at {} to CommonJS\n{advice}",
            bad.display()
        ),
        "{}",
        r.all()
    );
}

/// Runs `file` (from `tests/data/expect_surface`) in a fresh project, with and without esbuild,
/// and compares the `out.txt` rows it writes with vitest 4.1.12's, line by line.
fn rows_are_vitests(name: &str, file: &str, src: &str, want: &str) {
    for esbuild in [false, true] {
        let p = Project::new(name);
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "type": "module" }"#);
        p.file(file, src);
        let r = turbo_test(&p.dir, &[file], &[("NO_COLOR", "1")]);
        assert_eq!(r.code, Some(0), "{}", r.all());
        let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
        for (g, w) in got.lines().zip(want.lines()) {
            assert_eq!(g, w, "esbuild={esbuild}");
        }
        assert_eq!(
            got.lines().count(),
            want.lines().count(),
            "esbuild={esbuild}"
        );
    }
}

#[test]
fn expect_poll_is_vitests() {
    // turbo-test#161: expect.poll was undefined. vitest 4.1.12's rows: the matcher re-runs `fn`
    // every `interval` (50) ms until it passes or `timeout` (1000) ms ran out, then throws the
    // last error with the cause `Matcher did not succeed in time.`; the message option prefixes
    // it; a throwing / rejecting fn is retried; the result is a thenable (`[object Promise]`);
    // the snapshot / throw matchers and .resolves / .rejects are a SyntaxError; poll counts one
    // assertion; outside a test it throws. The interval and timeout run on the virtual clock
    // (Date.now() moves by them); under vi.useFakeTimers each retry advances the fake clock by
    // `interval` (`[4,150]`: a fake 120 ms timer seen on the 4th call).
    rows_are_vitests(
        "rc-expect-poll",
        "poll.test.js",
        include_str!("data/expect_surface/poll.test.js"),
        include_str!("data/expect_surface/poll.vitest.txt"),
    );
}

#[test]
fn expect_poll_not_awaited_fails_the_test() {
    // turbo-test#161: a poll matcher that is not awaited never runs and fails its test once the
    // test is done, with vitest 4.1.12's error for each one (the last one first) at its call site.
    let p = Project::new("rc-expect-poll-unawaited");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "unawaited.test.js",
        include_str!("data/expect_surface/poll_unawaited.test.js"),
    );
    let r = turbo_test(
        &p.dir,
        &["unawaited.test.js", "--reporter", "default"],
        &[("NO_COLOR", "1"), ("TURBO_LEGACY_REPORTER", "0")],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    let out = r.all();
    let not = "Error: expect.poll(assertion).not.toBe() was not awaited. This assertion is asynchronous and must be awaited; otherwise, it is not executed to avoid unhandled rejections:\n\nawait expect.poll(assertion).not.toBe()\n";
    let plain = "Error: expect.poll(assertion).toBe() was not awaited. This assertion is asynchronous and must be awaited; otherwise, it is not executed to avoid unhandled rejections:\n\nawait expect.poll(assertion).toBe()\n";
    // each at its matcher's call site, as vitest's
    let not = format!("{not}\n ❯ unawaited.test.js:5:28\n");
    let plain = format!("{plain}\n ❯ unawaited.test.js:4:40\n");
    let (a, b) = (out.find(&not), out.find(&plain));
    assert!(a.is_some() && b.is_some() && a < b, "{out}");
    assert!(out.contains("Tests  1 failed | 1 passed (2)"), "{out}");
}

#[test]
fn snapshot_serializers_are_vitests() {
    // turbo-test#162: expect.addSnapshotSerializer was undefined. A serializer (`serialize`, or
    // the old `print` API) applies to every snapshot from then on, the newest first, inside
    // objects too, with pretty-format's config / indentation / depth / refs / printer; it does
    // not change toEqual's message; a serializer that cannot print fails the snapshot with
    // PrettyFormatPluginError. Expected: the inline snapshots and the .snap file vitest 4.1.12
    // wrote for the same file, and its rows.
    let p = Project::new("rc-snapshot-serializers");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "serializer.test.js",
        include_str!("data/expect_surface/serializer.test.js"),
    );
    let want_src = include_str!("data/expect_surface/serializer.vitest.js");
    let want_snap = include_str!("data/expect_surface/serializer.vitest.snap");
    let want_rows = include_str!("data/expect_surface/serializer.vitest.txt");
    for pass in ["write", "check"] {
        let r = turbo_test(&p.dir, &["serializer.test.js"], &[("NO_COLOR", "1")]);
        assert_eq!(r.code, Some(0), "{pass}: {}", r.all());
        assert_eq!(
            std::fs::read_to_string(p.path("serializer.test.js")).unwrap(),
            want_src,
            "{pass}"
        );
        assert_eq!(
            std::fs::read_to_string(p.path("__snapshots__/serializer.test.js.snap")).unwrap(),
            want_snap,
            "{pass}"
        );
        assert_eq!(
            std::fs::read_to_string(p.path("out.txt")).unwrap(),
            want_rows,
            "{pass}"
        );
    }
}

#[test]
fn matcher_utils_are_vitests() {
    // turbo-test#163: a custom matcher's `this.utils` was JSON-based. vitest 4.1.12's (NO_COLOR):
    // its members, printReceived / printExpected / stringify through pretty-format (trailing
    // spaces as `·`, maxDepth, maxLength halving, options), matcherHint's every option, diff /
    // printDiffOrStringify / printWithType, and the messages of matchers built on them.
    rows_are_vitests(
        "rc-matcher-utils",
        "utils.test.js",
        include_str!("data/expect_surface/utils.test.js"),
        include_str!("data/expect_surface/utils.vitest.txt"),
    );
}

#[test]
fn snapshot_serializers_config_is_vitests() {
    // turbo-test#167: the config's `snapshotSerializers` (paths relative to the root, an ES
    // module's or a CommonJS module's default export) were not read. vitest 4.1.12 adds each one
    // before the file runs, so they apply like expect.addSnapshotSerializer's (one added in the
    // test comes first). Expected: the inline snapshots and the .snap file vitest wrote.
    let p = Project::new("rc-snapshot-serializers-config");
    p.file("package.json", r#"{ "type": "module" }"#);
    for (name, src) in [
        (
            "vitest.config.mjs",
            include_str!("data/expect_surface/serializer_config/vitest.config.mjs"),
        ),
        (
            "money-serializer.js",
            include_str!("data/expect_surface/serializer_config/money-serializer.js"),
        ),
        (
            "tagged-serializer.cjs",
            include_str!("data/expect_surface/serializer_config/tagged-serializer.cjs"),
        ),
        (
            "cfg.test.js",
            include_str!("data/expect_surface/serializer_config/cfg.test.js"),
        ),
    ] {
        p.file(name, src);
    }
    for pass in ["write", "check"] {
        let r = turbo_test(&p.dir, &["cfg.test.js"], &[("NO_COLOR", "1")]);
        assert_eq!(r.code, Some(0), "{pass}: {}", r.all());
        assert_eq!(
            std::fs::read_to_string(p.path("cfg.test.js")).unwrap(),
            include_str!("data/expect_surface/serializer_config/cfg.vitest.js"),
            "{pass}"
        );
        assert_eq!(
            std::fs::read_to_string(p.path("__snapshots__/cfg.test.js.snap")).unwrap(),
            include_str!("data/expect_surface/serializer_config/cfg.vitest.snap"),
            "{pass}"
        );
    }
    // a serializer module without a default object, or one that cannot print: vitest's errors
    let bad = Project::new("rc-snapshot-serializers-config-bad");
    bad.file("package.json", r#"{ "type": "module" }"#);
    bad.file(
        "vitest.config.mjs",
        "export default { test: { snapshotSerializers: ['./bad.js'] } }\n",
    );
    bad.file(
        "a.test.js",
        "import { test, expect } from 'vitest'\ntest('a', () => { expect(1).toBe(1) })\n",
    );
    let bad_js = bad.path("bad.js");
    for (src, want) in [
        (
            "export const x = 1\n",
            format!(
                "Error: invalid snapshot serializer file {}. Must export a default object",
                bad_js.display()
            ),
        ),
        (
            "export default { test: () => true }\n",
            format!("TypeError: invalid snapshot serializer in {}. Must have a 'test' method along with either a 'serialize' or 'print' method.", bad_js.display()),
        ),
    ] {
        bad.file("bad.js", src);
        let r = turbo_test(&bad.dir, &["a.test.js"], &[("NO_COLOR", "1")]);
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert!(r.all().contains(&want), "missing {want:?} in\n{}", r.all());
    }
}
