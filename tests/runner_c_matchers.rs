//! Failed matchers throw what vitest 4.1.12 throws: chai's `AssertionError` with vitest's message
//! for every built-in matcher, positive and negated (spy listings and diffs included), snapshot
//! mismatches and custom (`expect.extend`) matchers. The corpus (`tests/data/matchers`) records
//! each case's error name and message; `vitest.json` is vitest 4.1.12's (NO_COLOR) record.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};
use serde_json::Value;

const FILES: [(&str, &str); 6] = [
    ("cases.js", include_str!("data/matchers/cases.js")),
    ("cases2.js", include_str!("data/matchers/cases2.js")),
    ("m.test.js", include_str!("data/matchers/m.test.js")),
    ("m2.test.js", include_str!("data/matchers/m2.test.js")),
    ("s.test.js", include_str!("data/matchers/s.test.js")),
    (
        "__snapshots__/s.test.js.snap",
        include_str!("data/matchers/s.test.js.snap"),
    ),
];

/// A record's (label, error name, message) — or (label, "passed", "").
fn rows(v: &Value) -> Vec<(String, String, String)> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let s = |k: &str| r[k].as_str().unwrap_or_default().to_string();
            if r["passed"] == true {
                (s("label"), "passed".into(), String::new())
            } else {
                (s("label"), s("name"), s("message"))
            }
        })
        .collect()
}

#[test]
#[ignore = "turbo-test#98: fails until the fix"]
fn failed_matchers_throw_what_vitest_throws() {
    // turbo-test#71: a failed matcher threw a plain `Error` with turbo-test's own wording
    // (`expected 1 to be 2`, `expected {"a":1} to equal {"a":2}`, `expected spy called 3x, got 2`).
    // turbo-test#98: toStrictEqual passed for a class instance against a plain object.
    let want: Value = serde_json::from_str(include_str!("data/matchers/vitest.json")).unwrap();
    for esbuild in [false, true] {
        let p = Project::new("rc-matchers");
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "type": "module" }"#);
        for (name, src) in FILES {
            p.file(name, src);
        }
        let r = turbo_test(&p.dir, &["m.test.js", "m2.test.js", "s.test.js"], &[]);
        assert_eq!(r.code, Some(0), "{}", r.all());
        for (out, key) in [("out.json", "m"), ("out2.json", "m2"), ("out3.json", "s")] {
            let got: Value =
                serde_json::from_str(&std::fs::read_to_string(p.path(out)).unwrap()).unwrap();
            let (got, want) = (rows(&got), rows(&want[key]));
            assert_eq!(got.len(), want.len(), "esbuild={esbuild} {out}");
            for (g, w) in got.iter().zip(&want) {
                assert_eq!(g, w, "esbuild={esbuild} {out}");
            }
        }
    }
}

#[test]
fn snapshot_files_are_vitests() {
    // turbo-test#73: turbo-test wrote (and expected) its own snapshot format, so a file vitest
    // wrote mismatched. `f.test.js.snap` is the file vitest 4.1.12 writes for `f.test.js`.
    let test_file = include_str!("data/snapshots/f.test.js");
    let vitest_snap = include_str!("data/snapshots/f.test.js.snap");
    let p = Project::new("rc-snapshots");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file("f.test.js", test_file);
    let snap = p.path("__snapshots__/f.test.js.snap");
    // written fresh, and rewritten by -u: vitest's bytes
    for args in [&[][..], &["-u"]] {
        let r = turbo_test(&p.dir, &[&["f.test.js"][..], args].concat(), &[]);
        assert_eq!(r.code, Some(0), "{args:?}\n{}", r.all());
        assert_eq!(
            std::fs::read_to_string(&snap).unwrap(),
            vitest_snap,
            "{args:?}"
        );
    }
    // vitest's own file passes
    p.file("__snapshots__/f.test.js.snap", vitest_snap);
    let r = turbo_test(&p.dir, &["f.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    // and a changed value is a mismatch of that snapshot
    p.file(
        "__snapshots__/f.test.js.snap",
        &vitest_snap.replace("exports[`t 9 1`] = `9`;", "exports[`t 9 1`] = `8`;"),
    );
    let r = turbo_test(&p.dir, &["f.test.js"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.contains("✗ t 9: Snapshot `t 9 1` mismatched"),
        "{}",
        r.all()
    );
}

/// The JSON report's `snapshot` summary (vitest's key order) with the project path as `<dir>`.
fn snapshot_summary(p: &Project, r: &common::Run) -> String {
    let j: Value = serde_json::from_str(r.stdout.trim())
        .unwrap_or_else(|e| panic!("not a JSON report ({e}):\n{}", r.all()));
    j["snapshot"]
        .to_string()
        .replace(&p.dir.display().to_string(), "<dir>")
}

#[test]
fn snapshot_modes_obsolete_snapshots_and_file_snapshots_follow_vitest() {
    // turbo-test#73: no `toMatchFileSnapshot`, no CI mode, obsolete snapshots neither reported nor
    // removed by -u, and the JSON `snapshot` summary all zeros. Expected = vitest 4.1.12 on the
    // same steps (its summaries, and the files it leaves).
    let p = Project::new("rc-snapshot-modes");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "a.test.js",
        "import { test, expect } from 'vitest'\n\
         test('keep', () => { expect(1).toMatchSnapshot() })\n\
         test('file', async () => {\n\
           await expect('raw text\\nline2').toMatchFileSnapshot('./out/raw.txt')\n\
           await expect({ a: 1 }).toMatchFileSnapshot('./out/obj.txt')\n\
         })\n",
    );
    let snap = p.path("__snapshots__/a.test.js.snap");
    let read = |rel: &str| std::fs::read_to_string(p.path(rel)).unwrap();
    let run = |args: &[&str], env: &[(&str, &str)]| {
        let r = turbo_test(&p.dir, &[&["--reporter", "json"][..], args].concat(), env);
        (r.code, snapshot_summary(&p, &r), r)
    };
    let summary = |added: u32,
                   f_added: u32,
                   f_removed: u32,
                   f_unmatched: u32,
                   f_updated: u32,
                   matched: u32,
                   unchecked: u32,
                   keys: &str,
                   unmatched: u32,
                   updated: u32,
                   did_update: bool| {
        format!(
            "{{\"added\":{added},\"failure\":false,\"filesAdded\":{f_added},\"filesRemoved\":{f_removed},\"filesRemovedList\":[],\"filesUnmatched\":{f_unmatched},\"filesUpdated\":{f_updated},\"matched\":{matched},\"total\":{},\"unchecked\":{unchecked},\"uncheckedKeysByFile\":[{keys}],\"unmatched\":{unmatched},\"updated\":{updated},\"didUpdate\":{did_update}}}",
            added + matched + unmatched + updated
        )
    };
    let gone = r#"{"filePath":"<dir>/a.test.js","keys":["gone 1"]}"#;
    // new: everything written
    let (code, s, r) = run(&[], &[]);
    assert_eq!(code, Some(0), "{}", r.all());
    assert_eq!(s, summary(3, 1, 0, 0, 0, 0, 0, "", 0, 0, false));
    assert_eq!(
        read("__snapshots__/a.test.js.snap"),
        "// Vitest Snapshot v1, https://vitest.dev/guide/snapshot.html\n\nexports[`keep 1`] = `1`;\n"
    );
    assert_eq!(read("out/raw.txt"), "raw text\nline2");
    assert_eq!(read("out/obj.txt"), "{\n  \"a\": 1,\n}");
    // again: all matched
    assert_eq!(
        run(&[], &[]).1,
        summary(0, 0, 0, 0, 0, 3, 0, "", 0, 0, false)
    );
    // an obsolete snapshot is reported, and removed by -u
    std::fs::write(
        &snap,
        read("__snapshots__/a.test.js.snap") + "\nexports[`gone 1`] = `5`;\n",
    )
    .unwrap();
    assert_eq!(
        run(&[], &[]).1,
        summary(0, 0, 0, 0, 0, 3, 1, gone, 0, 0, false)
    );
    assert!(read("__snapshots__/a.test.js.snap").contains("gone 1"));
    assert_eq!(
        run(&["-u"], &[]).1,
        summary(0, 0, 0, 0, 0, 3, 1, gone, 0, 0, true)
    );
    assert!(!read("__snapshots__/a.test.js.snap").contains("gone 1"));
    // a changed file snapshot fails, -u rewrites it
    std::fs::write(p.path("out/raw.txt"), "changed\n").unwrap();
    let (code, s, r) = run(&[], &[]);
    assert_eq!(code, Some(1), "{}", r.all());
    assert_eq!(s, summary(0, 0, 0, 1, 0, 1, 0, "", 1, 0, false));
    assert!(
        r.stdout.contains("Snapshot `file 1` mismatched"),
        "{}",
        r.all()
    );
    assert_eq!(
        run(&["-u"], &[]).1,
        summary(0, 0, 0, 0, 1, 2, 0, "", 0, 1, true)
    );
    assert_eq!(read("out/raw.txt"), "raw text\nline2");
    // on CI a missing snapshot fails and nothing is written
    std::fs::remove_dir_all(p.path("__snapshots__")).unwrap();
    std::fs::remove_dir_all(p.path("out")).unwrap();
    let (code, s, r) = run(&[], &[("CI", "true")]);
    assert_eq!(code, Some(1), "{}", r.all());
    assert_eq!(s, summary(0, 0, 0, 1, 0, 0, 0, "", 2, 0, false));
    assert!(!snap.exists() && !p.path("out").exists());
    // a file that no longer has snapshots: reported, and its snapshot file removed by -u
    run(&[], &[]);
    p.file(
        "a.test.js",
        "import { test } from 'vitest'; test('x', () => {})\n",
    );
    let keep = r#"{"filePath":"<dir>/a.test.js","keys":["keep 1"]}"#;
    assert_eq!(
        run(&["-u"], &[]).1,
        summary(0, 0, 1, 0, 0, 0, 0, keep, 0, 0, true)
    );
    assert!(!snap.exists());
}

#[test]
fn inline_snapshots_are_written_back_as_vitest_writes_them() {
    // turbo-test#73: a new inline snapshot passed without being written, and -u could not update
    // one. vitest 4.1.12 writes them into the test file: `inline.new.ts` is the file after a
    // plain run of `inline.test.ts` (its changed ones fail), `inline.updated.ts` after -u.
    let source = include_str!("data/snapshots/inline.test.ts");
    for esbuild in [false, true] {
        let p = Project::new("rc-inline-snapshots");
        if esbuild {
            p.with_esbuild();
        }
        p.file("package.json", r#"{ "type": "module" }"#);
        p.file("i.test.ts", source);
        let read = || std::fs::read_to_string(p.path("i.test.ts")).unwrap();
        let r = turbo_test(&p.dir, &["i.test.ts"], &[]);
        assert_eq!(r.code, Some(1), "{}", r.all());
        assert!(
            r.stdout
                .contains("✗ inline > changed: Snapshot `inline > changed 1` mismatched"),
            "{}",
            r.all()
        );
        assert_eq!(
            read(),
            include_str!("data/snapshots/inline.new.ts"),
            "esbuild={esbuild}"
        );
        let r = turbo_test(&p.dir, &["-u", "i.test.ts"], &[]);
        assert_eq!(r.code, Some(0), "{}", r.all());
        assert_eq!(
            read(),
            include_str!("data/snapshots/inline.updated.ts"),
            "esbuild={esbuild}"
        );
        // the written snapshots pass
        assert_eq!(turbo_test(&p.dir, &["i.test.ts"], &[]).code, Some(0));
    }
}

#[test]
fn snapshot_ci_mode_follows_std_env_is_ci() {
    // turbo-test#74: vitest 4.1.12 picks snapshot update mode `none` (a missing snapshot fails,
    // nothing is written) when std-env's isCI holds. Checked against vitest: CI=false and
    // NOW_BUILDER=1 fail; VERCEL=1, NETLIFY_LOCAL=1, TF_BUILD=True and CI= write the snapshot.
    let p = Project::new("rc-snapshot-is-ci");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "s.test.js",
        "import { test, expect } from 'vitest'\ntest('snap', () => { expect({ a: 1 }).toMatchSnapshot() })\n",
    );
    let snap = p.path("__snapshots__/s.test.js.snap");
    let cases: [(&[(&str, &str)], bool); 8] = [
        (&[("CI", "false")], true),
        (&[("CI", "0")], true),
        (&[("NOW_BUILDER", "1")], true),
        (&[("CI", ""), ("GITHUB_ACTIONS", "true")], true),
        (&[("CI", "")], false),
        (&[("VERCEL", "1")], false),
        (&[("NETLIFY_LOCAL", "1")], false),
        (&[("TF_BUILD", "True")], false),
    ];
    for (env, ci) in cases {
        let _ = std::fs::remove_dir_all(p.path("__snapshots__"));
        let r = turbo_test(&p.dir, &["s.test.js"], env);
        assert_eq!(r.code, Some(if ci { 1 } else { 0 }), "{env:?}: {}", r.all());
        assert_eq!(snap.exists(), !ci, "{env:?}: {}", r.all());
    }
}
