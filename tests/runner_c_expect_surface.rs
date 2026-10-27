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
/// chai's BDD chain (`expect(x).to.be.equal(...)`), chai's `assert` and chai's internal `__flags`,
/// `expect.poll`, `expect.addSnapshotSerializer`.
const KNOWN_MISSING: [(&str, &str); 4] = [
    ("expect", "__flags:object"),
    ("expect", "addSnapshotSerializer:function"),
    ("expect", "assert:function"),
    ("expect", "poll:function"),
];

#[test]
fn expect_members_are_vitests() {
    // turbo-test#153 / #154 / #155: expect.closeTo / schemaMatching / addEqualityTesters / fail /
    // unreachable / getState / setState / toSatisfy were missing, expect.extend made no
    // asymmetric matchers, the asymmetric matchers had none of vitest's members, and 17
    // matchers (toBeCalled, toHaveReturnedTimes, toHaveResolved*, toHaveBeenCalledBefore, ...)
    // were missing while a stray `toStrictEqual_alias` existed.
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
    let mut compare = |label: &str, w: &Value, g: &Value, keep: &dyn Fn(&str) -> bool| {
        let (w, g) = (set(w), set(g));
        for m in w.difference(&g).filter(|m| keep(m)) {
            missing.push(format!("{label} {m}"));
        }
        for m in g.difference(&w) {
            extra.push(format!("{label} {m}"));
        }
    };
    let all = |_: &str| true;
    compare("expect", &want["expect"], &got["expect"], &all);
    compare("expect.not", &want["expectNot"], &got["expectNot"], &all);
    for (k, w) in want["asym"].as_object().unwrap() {
        compare(&format!("asym.{k}"), w, &got["asym"][k], &all);
    }
    // the chains: vitest's matchers and not / resolves / rejects (chai's BDD words are KNOWN)
    let matcher = |m: &str| {
        let name = m.split(':').next().unwrap();
        (name.starts_with("to") && name != "to") || ["not", "resolves", "rejects"].contains(&name)
    };
    for k in ["expect", "not", "resolves", "rejects", "soft"] {
        compare(
            &format!("chain.{k}"),
            &want["chain"][k],
            &got["chain"][k],
            &matcher,
        );
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
