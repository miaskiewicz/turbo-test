//! Failed matchers throw what vitest 4.1.12 throws: chai's `AssertionError` with vitest's message
//! for every built-in matcher, positive and negated (spy listings and diffs included), snapshot
//! mismatches and custom (`expect.extend`) matchers. The corpus (`tests/data/matchers`) records
//! each case's error name and message; `vitest.json` is vitest 4.1.12's (NO_COLOR) record.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};
use serde_json::Value;

const FILES: [(&str, &str); 7] = [
    ("cases.js", include_str!("data/matchers/cases.js")),
    ("cases2.js", include_str!("data/matchers/cases2.js")),
    ("m.test.js", include_str!("data/matchers/m.test.js")),
    ("m2.test.js", include_str!("data/matchers/m2.test.js")),
    ("s.test.js", include_str!("data/matchers/s.test.js")),
    ("eq.test.js", include_str!("data/matchers/eq.test.js")),
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
fn failed_matchers_throw_what_vitest_throws() {
    // turbo-test#71: a failed matcher threw a plain `Error` with turbo-test's own wording
    // (`expected 1 to be 2`, `expected {"a":1} to equal {"a":2}`, `expected spy called 3x, got 2`).
    // turbo-test#98: toStrictEqual passed for a class instance against a plain object.
    // turbo-test#103: toEqual / toStrictEqual (and the matchers on the same `equals`) passed for
    // different Dates, Maps, Sets, RegExps, boxed primitives, symbol keys, Errors.
    // turbo-test#111: toThrow(error / object) compared the message only.
    // turbo-test#112: a URL in a message printed as `URL{ protocol: 'http:', …(8) }`.
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
        let r = turbo_test(
            &p.dir,
            &["m.test.js", "m2.test.js", "s.test.js", "eq.test.js"],
            &[],
        );
        assert_eq!(r.code, Some(0), "{}", r.all());
        for (out, key) in [
            ("out.json", "m"),
            ("out2.json", "m2"),
            ("out3.json", "s"),
            ("out4.json", "eq"),
        ] {
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

#[test]
fn urls_inspect_serialize_and_diff_like_node_and_vitest() {
    // turbo-test#112: the URL / URLSearchParams globals printed their internals
    // (`URL{ protocol: 'http:', …(8) }`, `URLSearchParams{ _: [ … ] }`) in util.inspect, JSON and
    // vitest's diffs. Expected values: vitest 4.1.12 (diffs) and Node 26 (inspect, JSON).
    let p = Project::new("rc-urls");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "u.test.js",
        r#"import { test, expect } from 'vitest'
import * as fs from 'node:fs'
import util from 'node:util'
test('shown', () => {
  const u = new URL('http://a.com:8080/p/q?x=1&y=2#h')
  fs.writeFileSync('out.txt', [
    util.inspect(u), util.inspect({ a: { b: u } }), util.inspect({ a: { b: { c: u } } }),
    util.inspect(new URLSearchParams('aaaaaaaaaaaaaaaaaaaaa=bbbbbbbbbbbbbbbbbbbbbbbbbbbb&cccccccccccccccccccccc=dddddddddddddddddddddddddddd&e=f')),
    util.inspect(new URLSearchParams("q=it's")), util.inspect([new URLSearchParams('a=1')], { depth: 0 }),
    JSON.stringify({ u: new URL('http://a/?x=1') }), Object.keys(new URLSearchParams('a=1')).length,
    Object.prototype.toString.call(u), Object.prototype.toString.call(u.searchParams),
  ].join('\n'))
})
test('url diff', () => { expect(new URL('http://a/')).toStrictEqual(new URL('http://b/')) })
test('params diff', () => { expect({ s: new URLSearchParams('a=1'), n: 1 }).toEqual({ s: new URLSearchParams('a=2'), n: 2 }) })
"#,
    );
    let r = turbo_test(
        &p.dir,
        &["u.test.js", "--reporter", "default"],
        &[("NO_COLOR", "1"), ("TURBO_LEGACY_REPORTER", "0")],
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    let url = "URL {
  href: 'http://a.com:8080/p/q?x=1&y=2#h',
  origin: 'http://a.com:8080',
  protocol: 'http:',
  username: '',
  password: '',
  host: 'a.com:8080',
  hostname: 'a.com',
  port: '8080',
  pathname: '/p/q',
  search: '?x=1&y=2',
  searchParams: URLSearchParams { 'x' => '1', 'y' => '2' },
  hash: '#h'
}";
    let nested = format!(
        "{{\n  a: {{\n    b: {}\n  }}\n}}",
        url.replace('\n', "\n    ")
    );
    let want = [
        url.to_string(),
        nested,
        "{ a: { b: { c: URL {} } } }".into(),
        "URLSearchParams {
  'aaaaaaaaaaaaaaaaaaaaa' => 'bbbbbbbbbbbbbbbbbbbbbbbbbbbb',
  'cccccccccccccccccccccc' => 'dddddddddddddddddddddddddddd',
  'e' => 'f' }"
            .into(),
        r#"URLSearchParams { 'q' => "it's" }"#.into(),
        "[ [Object] ]".into(),
        r#"{"u":"http://a/?x=1"}"#.into(),
        "0".into(),
        "[object URL]".into(),
        "[object URLSearchParams]".into(),
    ]
    .join("\n");
    assert_eq!(std::fs::read_to_string(p.path("out.txt")).unwrap(), want);
    let out = r.all();
    assert!(
        out.contains("- Expected\n+ Received\n\n- \"http://b/\"\n+ \"http://a/\"\n"),
        "{out}"
    );
    assert!(
        out.contains("  {\n-   \"n\": 2,\n+   \"n\": 1,\n    \"s\": URLSearchParams {},\n  }\n"),
        "{out}"
    );
}

#[test]
fn url_userinfo_and_own_keys_are_nodes() {
    // turbo-test#114: the URL global took `user:pass@host` whole as the host (username /
    // password undefined) and kept its parsed parts as own enumerable keys. Expected: Node 26.
    let p = Project::new("rc-url-userinfo");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "u.test.js",
        r#"import { test } from 'vitest'
import * as fs from 'node:fs'
test('userinfo', () => {
  const rows = ['http://u:p@a.com:8080/x', 'http://u@a/', 'http://:p@a/', 'http://a b:c@d/', 'http://u:p:q@a/',
    'http://a@b@c/', 'http://u:@a/', 'file:///x', 'mailto:x@y'].map((s) => {
    const u = new URL(s)
    return JSON.stringify([s, u.href, u.username, u.password, u.host, u.hostname, u.port, u.origin])
  })
  const r = new URL('y?q=1#h', 'http://u:p@a/x/')
  rows.push(JSON.stringify([r.href, r.username, r.password]))
  rows.push(`${Object.keys(new URL('http://u:p@a/')).length} ${JSON.stringify({ ...new URL('http://a/') })}`)
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"#,
    );
    let r = turbo_test(&p.dir, &["u.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let want = r#"["http://u:p@a.com:8080/x","http://u:p@a.com:8080/x","u","p","a.com:8080","a.com","8080","http://a.com:8080"]
["http://u@a/","http://u@a/","u","","a","a","","http://a"]
["http://:p@a/","http://:p@a/","","p","a","a","","http://a"]
["http://a b:c@d/","http://a%20b:c@d/","a%20b","c","d","d","","http://d"]
["http://u:p:q@a/","http://u:p%3Aq@a/","u","p%3Aq","a","a","","http://a"]
["http://a@b@c/","http://a%40b@c/","a%40b","","c","c","","http://c"]
["http://u:@a/","http://u@a/","u","","a","a","","http://a"]
["file:///x","file:///x","","","","","","null"]
["mailto:x@y","mailto:x@y","","","","","","null"]
["http://u:p@a/x/y?q=1#h","u","p"]
0 {}"#;
    assert_eq!(std::fs::read_to_string(p.path("out.txt")).unwrap(), want);
}

#[test]
fn url_query_is_kept_as_parsed_like_nodes() {
    // turbo-test#115: the URL global re-serialized its query through URLSearchParams
    // (`?q` -> `?q=`, `%20` -> `+`, empty pairs dropped). WHATWG keeps the parsed query
    // (percent-encoded with the query set) until the searchParams change. Expected: Node 26.
    let p = Project::new("rc-url-query");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "u.test.js",
        r#"import { test } from 'vitest'
import * as fs from 'node:fs'
test('query', () => {
  const rows = ['http://a/?q', 'http://a/?a%20b=c d', 'http://a/?x=1&&y', 'http://a/?', "http://a/?it's", "foo://a/?it's", 'http://a/?€', 'http://a/?a+b', 'http://a/?"<>`', 'mailto:x?a b'].map((s) => {
    const u = new URL(s)
    return JSON.stringify([s, u.href, u.search, [...u.searchParams]])
  })
  const u = new URL('http://a/?q&x=1')
  const sp = u.searchParams
  sp.append('y', 'a b'); rows.push(JSON.stringify([u.href, u.search]))
  sp.delete('q'); sp.delete('x'); sp.delete('y'); rows.push(JSON.stringify([u.href, u.search]))
  u.search = 'z z'; rows.push(JSON.stringify([u.href, u.search, sp.get('z z'), u.searchParams === sp]))
  u.search = ''; rows.push(JSON.stringify([u.href, u.search, [...sp]]))
  u.search = '?'; rows.push(JSON.stringify([u.href, u.search]))
  u.href = 'http://b/?k'; rows.push(JSON.stringify([u.href, sp.get('k'), u.searchParams === sp]))
  sp.sort(); rows.push(JSON.stringify([u.href]))
  sp.set('k', '1'); rows.push(JSON.stringify([u.href]))
  rows.push(JSON.stringify([new URL('?q', 'http://a/p').href, new URL('#h', 'http://a/p?q').href, new URL('x', 'http://a/p?q').href]))
  rows.push(String(Object.keys(sp).length))
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"#,
    );
    let r = turbo_test(&p.dir, &["u.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let want = r#"["http://a/?q","http://a/?q","?q",[["q",""]]]
["http://a/?a%20b=c d","http://a/?a%20b=c%20d","?a%20b=c%20d",[["a b","c d"]]]
["http://a/?x=1&&y","http://a/?x=1&&y","?x=1&&y",[["x","1"],["y",""]]]
["http://a/?","http://a/?","",[]]
["http://a/?it's","http://a/?it%27s","?it%27s",[["it's",""]]]
["foo://a/?it's","foo://a/?it's","?it's",[["it's",""]]]
["http://a/?€","http://a/?%E2%82%AC","?%E2%82%AC",[["€",""]]]
["http://a/?a+b","http://a/?a+b","?a+b",[["a b",""]]]
["http://a/?\"<>`","http://a/?%22%3C%3E`","?%22%3C%3E`",[["\"<>`",""]]]
["mailto:x?a b","mailto:x?a%20b","?a%20b",[["a b",""]]]
["http://a/?q=&x=1&y=a+b","?q=&x=1&y=a+b"]
["http://a/",""]
["http://a/?z%20z","?z%20z","",true]
["http://a/","",[]]
["http://a/?",""]
["http://b/?k","",true]
["http://b/?k="]
["http://b/?k=1"]
["http://a/p?q","http://a/p?q#h","http://a/x"]
0"#;
    assert_eq!(std::fs::read_to_string(p.path("out.txt")).unwrap(), want);
}

#[test]
fn url_search_params_set_replaces_in_place_like_nodes() {
    // turbo-test#116: URLSearchParams.set removed every pair of the name and appended one, so
    // the pair moved to the end. WHATWG: the first pair's value is replaced in place and the
    // others removed; a new name is appended. Expected: Node 26.
    let p = Project::new("rc-usp-set");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "u.test.js",
        r#"import { test } from 'vitest'
import * as fs from 'node:fs'
test('set', () => {
  const rows = []
  const p = new URLSearchParams('a=1&b=2&a=3&c=4&a=5')
  p.set('a', '9'); rows.push(p.toString())
  p.set('d', 'x'); rows.push(p.toString())
  p.set('b', 7); rows.push(JSON.stringify([...p]))
  const u = new URL('http://h/?x=1&y=2&x=3')
  u.searchParams.set('x', 'z'); rows.push(u.href)
  const q = new URLSearchParams('k=1')
  q.set('k', 'v'); rows.push(q.toString())
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"#,
    );
    let r = turbo_test(&p.dir, &["u.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let want = r#"a=9&b=2&c=4
a=9&b=2&c=4&d=x
[["a","9"],["b","7"],["c","4"],["d","x"]]
http://h/?x=z&y=2
k=v"#;
    assert_eq!(std::fs::read_to_string(p.path("out.txt")).unwrap(), want);
}

#[test]
fn url_search_params_surface_is_nodes() {
    // turbo-test#117 and the rest of the URLSearchParams surface, table-driven: names and
    // values go through USVString conversion (String(), lone surrogates -> U+FFFD) in every
    // method; constructor from string / record / sequence of pairs (bad pairs throw TypeError)
    // / another URLSearchParams; has/delete value arg; stable code-unit sort; size; live
    // forEach and iterators; form-urlencoded toString; write-back to the owning URL.
    // Expected: Node 26 (each row is [case, result] or [case, ["throws", error name]]).
    let p = Project::new("rc-usp-surface");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "u.test.js",
        r##"import { test } from 'vitest'
import * as fs from 'node:fs'
test('URLSearchParams', () => {
  const rows = []
  const t = (name, f) => { let r; try { r = f() } catch (e) { r = ['throws', e.name] } rows.push(JSON.stringify([name, r])) }
  const L = (p) => [...p]
  // constructor
  t('str', () => L(new URLSearchParams('a=1&b=2&a=3')))
  t('str?', () => L(new URLSearchParams('?a=1')))
  t('str??', () => L(new URLSearchParams('??a=1')))
  t('str empty parts', () => L(new URLSearchParams('&&a&=&=x&b==c&')))
  t('str plus pct', () => L(new URLSearchParams('a+b=c%20d+e&%2B=%2b')))
  t('str bad pct', () => L(new URLSearchParams('a=%zz&b=%&c=%4&d=%E2%82&e=%E2%82%AC&f=%C3%28&g=%FF')))
  t('str surrogate', () => L(new URLSearchParams('a=\uD800&\uDC00=b')))
  t('str non-ascii', () => L(new URLSearchParams('€=ü&😀=x')))
  t('undefined', () => L(new URLSearchParams(undefined)))
  t('null', () => L(new URLSearchParams(null)))
  t('number', () => L(new URLSearchParams(12)))
  t('bool', () => L(new URLSearchParams(true)))
  t('record', () => L(new URLSearchParams({ a: 1, b: 'x y', c: null, d: undefined, 2: 'n' })))
  t('record surrogate', () => L(new URLSearchParams({ '\uD800': 'v\uDC00' })))
  t('record dup after usv', () => L(new URLSearchParams({ '\uD800': '1', '\uDC00': '2', '�': '3' })))
  t('record symbol key', () => L(new URLSearchParams({ [Symbol('s')]: 1, a: 2 })))
  t('record non-enum', () => L(new URLSearchParams(Object.defineProperty({ a: 1 }, 'h', { value: 2, enumerable: false }))))
  t('record inherited', () => L(new URLSearchParams(Object.create({ inh: 1 }, { own: { value: 2, enumerable: true } }))))
  t('record symbol value', () => L(new URLSearchParams({ a: Symbol('s') })))
  t('record toString', () => L(new URLSearchParams({ a: { toString() { return 'T' } } })))
  t('seq', () => L(new URLSearchParams([['a', 1], ['b', null], [2, undefined]])))
  t('seq empty', () => L(new URLSearchParams([])))
  t('seq iterable pairs', () => L(new URLSearchParams(new Map([['a', '1'], ['b', '2']]))))
  t('seq set of pairs', () => L(new URLSearchParams(new Set([['x', 'y']]))))
  t('seq generator', () => L(new URLSearchParams((function* () { yield ['g', 'h']; yield new Set(['i', 'j']) })())))
  t('seq pair too short', () => L(new URLSearchParams([['a']])))
  t('seq pair too long', () => L(new URLSearchParams([['a', 'b', 'c']])))
  t('seq pair string', () => L(new URLSearchParams(['ab'])))
  t('seq pair number', () => L(new URLSearchParams([1])))
  t('seq pair null', () => L(new URLSearchParams([null])))
  t('seq pair empty', () => L(new URLSearchParams([[]])))
  t('seq pair symbol', () => L(new URLSearchParams([[Symbol('s'), 'v']])))
  t('seq surrogate', () => L(new URLSearchParams([['\uD800x', 'y\uDFFF']])))
  t('iterator not callable', () => L(new URLSearchParams({ [Symbol.iterator]: 1, a: 2 })))
  t('iterator null', () => L(new URLSearchParams({ [Symbol.iterator]: null, a: 2 })))
  t('copy', () => { const a = new URLSearchParams('a=1&b=2'); const b = new URLSearchParams(a); b.append('c', 3); return [L(a), L(b)] })
  t('copy url sp', () => { const u = new URL('http://h/?x=1'); const b = new URLSearchParams(u.searchParams); b.set('x', 2); return [u.href, L(b)] })
  t('symbol init', () => L(new URLSearchParams(Symbol('s'))))
  t('obj toString init', () => L(new URLSearchParams(Object.assign(() => {}, { a: 1 }))))
  // ToString names and values (issue #117)
  t('issue117', () => { const p = new URLSearchParams(); p.append(1, 'x'); p.set(2, 'y'); return [p.get('1'), p.has(1), p.getAll(1), p.get('2'), [...p.keys()].map((k) => typeof k)] })
  t('tostring all', () => {
    const p = new URLSearchParams()
    p.append(null, undefined); p.append(undefined, null); p.append(true, 1.5); p.set({ toString: () => 'o' }, [1, 2])
    return [L(p), p.get(null), p.get('null'), p.get(undefined), p.getAll(true), p.has(1.5), p.has('true'), p.get({ toString: () => 'o' })]
  })
  t('delete tostring', () => { const p = new URLSearchParams('1=a&2=b&null=c'); p.delete(1); p.delete(null); return L(p) })
  t('append symbol name', () => new URLSearchParams().append(Symbol('s'), 'v'))
  t('append symbol value', () => new URLSearchParams().append('n', Symbol('s')))
  t('get symbol', () => new URLSearchParams().get(Symbol('s')))
  t('surrogate methods', () => { const p = new URLSearchParams(); p.append('\uD800', '\uDC00'); p.set('a\uDBFF', 'b'); return [L(p), p.get('�'), p.has('\uD800'), p.get('a�'), p.toString()] })
  // missing args
  t('append 1 arg', () => new URLSearchParams().append('a'))
  t('append 0 arg', () => new URLSearchParams().append())
  t('set 1 arg', () => new URLSearchParams().set('a'))
  t('get 0 arg', () => new URLSearchParams().get())
  t('getAll 0 arg', () => new URLSearchParams().getAll())
  t('has 0 arg', () => new URLSearchParams().has())
  t('delete 0 arg', () => new URLSearchParams().delete())
  t('append undefined value', () => { const p = new URLSearchParams(); p.append('a', undefined); return L(p) })
  // get/getAll/has/delete
  t('get first', () => { const p = new URLSearchParams('a=1&a=2&b='); return [p.get('a'), p.get('b'), p.get('c'), p.getAll('a'), p.getAll('c'), p.getAll('b')] })
  t('has value', () => { const p = new URLSearchParams('a=1&a=2&b=3'); return [p.has('a', '2'), p.has('a', '3'), p.has('a', 2), p.has('a', undefined), p.has('c', undefined), p.has('a', null), p.has('b', '3')] })
  t('has value null match', () => { const p = new URLSearchParams('a=null'); return [p.has('a', null), p.has('a', 'null')] })
  t('delete value', () => { const p = new URLSearchParams('a=1&b=2&a=2&a=3'); p.delete('a', '2'); const r1 = p.toString(); p.delete('a', 3); const r2 = p.toString(); p.delete('a', undefined); return [r1, r2, p.toString()] })
  t('delete value null', () => { const p = new URLSearchParams('a=null&a=x'); p.delete('a', null); return p.toString() })
  t('delete missing', () => { const p = new URLSearchParams('a=1'); p.delete('z'); return p.toString() })
  // append/set
  t('append dup', () => { const p = new URLSearchParams('a=1'); p.append('a', '1'); p.append('', ''); return [L(p), p.toString()] })
  t('set new / existing', () => { const p = new URLSearchParams('a=1&b=2&a=3'); p.set('a', 'x'); p.set('c', 'y'); p.set('b', ''); return p.toString() })
  // sort
  t('sort stable', () => { const p = new URLSearchParams('z=1&a=1&z=2&a=2&b=0&a=3'); p.sort(); return p.toString() })
  t('sort code units', () => { const p = new URLSearchParams([['�', '1'], ['😀', '2'], ['', '3'], ['a', '4'], ['A', '5'], ['', '6'], ['ab', '7'], ['aa', '8']]); p.sort(); return [...p.keys()] })
  t('sort empty', () => { const p = new URLSearchParams(); p.sort(); return p.toString() })
  // size
  t('size', () => { const p = new URLSearchParams('a=1&a=2&b=3'); const s0 = p.size; p.delete('a'); return [s0, p.size, new URLSearchParams().size, Object.getOwnPropertyDescriptor(URLSearchParams.prototype, 'size') && typeof Object.getOwnPropertyDescriptor(URLSearchParams.prototype, 'size').get] })
  // forEach / iteration
  t('forEach', () => { const p = new URLSearchParams('a=1&b=2'); const out = []; const self = { s: 1 }; p.forEach(function (v, k, o) { out.push([v, k, o === p, this === self]) }, self); return out })
  t('forEach live append', () => { const p = new URLSearchParams('a=1&b=2'); const out = []; p.forEach((v, k) => { out.push(k); if (k === 'a') p.append('c', '3') }); return out })
  t('forEach live delete', () => { const p = new URLSearchParams('a=1&b=2&c=3'); const out = []; p.forEach((v, k) => { out.push(k); if (k === 'a') p.delete('a') }); return out })
  t('forEach no fn', () => new URLSearchParams('a=1').forEach())
  t('forEach not fn', () => new URLSearchParams('a=1').forEach(1))
  t('iter live', () => { const p = new URLSearchParams('a=1&b=2'); const it = p.keys(); const first = it.next(); p.append('c', '3'); return [first, [...it]] })
  t('iter live delete', () => { const p = new URLSearchParams('a=1&b=2&c=3'); const it = p.values(); const first = it.next().value; p.delete('a'); return [first, [...it]] })
  t('iter shapes', () => { const p = new URLSearchParams('a=1&b=2'); return [[...p.entries()], [...p.keys()], [...p.values()], [...p], p[Symbol.iterator] === p.entries, p.entries().next(), new URLSearchParams().entries().next()] })
  t('iter tags', () => { const it = new URLSearchParams('a=1').entries(); return [Object.prototype.toString.call(it), typeof it[Symbol.iterator], it[Symbol.iterator]() === it, Object.prototype.toString.call(new URLSearchParams())] })
  t('iter done stays done', () => { const p = new URLSearchParams('a=1'); const it = p.keys(); it.next(); const d = it.next(); p.append('b', '2'); return [d, it.next()] })
  t('iter result keys', () => { const it = new URLSearchParams('a=1').keys(); return [Reflect.ownKeys(it.next()), Reflect.ownKeys(it.next()), Object.getPrototypeOf(Object.getPrototypeOf(it)) === Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()))] })
  t('entries fresh arrays', () => { const p = new URLSearchParams('a=1'); const e = p.entries().next().value; e[0] = 'z'; return L(p) })
  // toString encoding
  t('enc space plus', () => new URLSearchParams([['a b', 'c d'], ['+', '%']]).toString())
  t('enc ascii', () => { let s = ''; for (let i = 0; i < 128; i++) s += String.fromCharCode(i); return new URLSearchParams([[s, 'v']]).toString() })
  t('enc non-ascii', () => new URLSearchParams([['€', 'ü'], ['😀', '\u0080']]).toString())
  t('enc surrogates', () => new URLSearchParams([['\uD800', '\uDC00'], ['a\uD83Db', '\uDE00\uD83D']]).toString())
  t('enc roundtrip', () => { const s = new URLSearchParams([['a=b&c', '?#/ ~!\'()*']]).toString(); return [s, L(new URLSearchParams(s))] })
  t('enc empty', () => [new URLSearchParams().toString(), new URLSearchParams('a').toString(), new URLSearchParams('=').toString(), new URLSearchParams('=x').toString()])
  t('String()', () => [String(new URLSearchParams('a=1')), `${new URLSearchParams('b=2')}`])
  // write back to the URL
  t('url append', () => { const u = new URL('http://h/p?a=1#f'); u.searchParams.append('b', 'x y'); return [u.href, u.search] })
  t('url set', () => { const u = new URL('http://h/?a=1&a=2'); u.searchParams.set(1, 2); return u.href })
  t('url delete all', () => { const u = new URL('http://h/?a=1#f'); u.searchParams.delete('a'); return [u.href, u.search] })
  t('url delete value', () => { const u = new URL('http://h/?a=1&a=2'); u.searchParams.delete('a', 1); return u.href })
  t('url delete none', () => { const u = new URL('http://h/?a%20b=c'); u.searchParams.delete('zz'); return u.href })
  t('url sort', () => { const u = new URL('http://h/?b=1&a=2'); u.searchParams.sort(); return u.href })
  t('url sort empty', () => { const u = new URL('http://h/?'); u.searchParams.sort(); return u.href })
  t('url reads after search set', () => { const u = new URL('http://h/?a=1'); const sp = u.searchParams; u.search = 'b=2&b=3'; return [sp.getAll('b'), sp.size, sp.has('a')] })
  t('url encoding', () => { const u = new URL('http://h/'); u.searchParams.append('\uD800€', "'~*"); return u.href })
  t('url opaque', () => { const u = new URL('mailto:x?a=1'); u.searchParams.append('b', '2'); return u.href })
  t('url get/has', () => { const u = new URL('http://h/?1=a&null=b'); return [u.searchParams.get(1), u.searchParams.has(null), u.searchParams.getAll(1)] })
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"##,
    );
    let r = turbo_test(&p.dir, &["u.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
    let want = r##"["str",[["a","1"],["b","2"],["a","3"]]]
["str?",[["a","1"]]]
["str??",[["?a","1"]]]
["str empty parts",[["a",""],["",""],["","x"],["b","=c"]]]
["str plus pct",[["a b","c d e"],["+","+"]]]
["str bad pct",[["a","%zz"],["b","%"],["c","%4"],["d","�"],["e","€"],["f","�("],["g","�"]]]
["str surrogate",[["a","�"],["�","b"]]]
["str non-ascii",[["€","ü"],["😀","x"]]]
["undefined",[]]
["null",[["null",""]]]
["number",[["12",""]]]
["bool",[["true",""]]]
["record",[["2","n"],["a","1"],["b","x y"],["c","null"],["d","undefined"]]]
["record surrogate",[["�","v�"]]]
["record dup after usv",[["�","3"]]]
["record symbol key",["throws","TypeError"]]
["record non-enum",[["a","1"]]]
["record inherited",[["own","2"]]]
["record symbol value",["throws","TypeError"]]
["record toString",[["a","T"]]]
["seq",[["a","1"],["b","null"],["2","undefined"]]]
["seq empty",[]]
["seq iterable pairs",[["a","1"],["b","2"]]]
["seq set of pairs",[["x","y"]]]
["seq generator",[["g","h"],["i","j"]]]
["seq pair too short",["throws","TypeError"]]
["seq pair too long",["throws","TypeError"]]
["seq pair string",["throws","TypeError"]]
["seq pair number",["throws","TypeError"]]
["seq pair null",["throws","TypeError"]]
["seq pair empty",["throws","TypeError"]]
["seq pair symbol",["throws","TypeError"]]
["seq surrogate",[["�x","y�"]]]
["iterator not callable",["throws","TypeError"]]
["iterator null",["throws","TypeError"]]
["copy",[[["a","1"],["b","2"]],[["a","1"],["b","2"],["c","3"]]]]
["copy url sp",["http://h/?x=1",[["x","2"]]]]
["symbol init",["throws","TypeError"]]
["obj toString init",[["a","1"]]]
["issue117",["x",true,["x"],"y",["string","string"]]]
["tostring all",[[["null","undefined"],["undefined","null"],["true","1.5"],["o","1,2"]],"undefined","undefined","null",["1.5"],false,true,"1,2"]]
["delete tostring",[["2","b"]]]
["append symbol name",["throws","TypeError"]]
["append symbol value",["throws","TypeError"]]
["get symbol",["throws","TypeError"]]
["surrogate methods",[[["�","�"],["a�","b"]],"�",true,"b","%EF%BF%BD=%EF%BF%BD&a%EF%BF%BD=b"]]
["append 1 arg",["throws","TypeError"]]
["append 0 arg",["throws","TypeError"]]
["set 1 arg",["throws","TypeError"]]
["get 0 arg",["throws","TypeError"]]
["getAll 0 arg",["throws","TypeError"]]
["has 0 arg",["throws","TypeError"]]
["delete 0 arg",["throws","TypeError"]]
["append undefined value",[["a","undefined"]]]
["get first",["1","",null,["1","2"],[],[""]]]
["has value",[true,false,true,true,false,false,true]]
["has value null match",[true,true]]
["delete value",["a=1&b=2&a=3","a=1&b=2","b=2"]]
["delete value null","a=x"]
["delete missing","a=1"]
["append dup",[[["a","1"],["a","1"],["",""]],"a=1&a=1&="]]
["set new / existing","a=x&b=&c=y"]
["sort stable","a=1&a=2&a=3&b=0&z=1&z=2"]
["sort code units",["","A","a","aa","ab","😀","","�"]]
["sort empty",""]
["size",[3,1,0,"function"]]
["forEach",[["1","a",true,true],["2","b",true,true]]]
["forEach live append",["a","b","c"]]
["forEach live delete",["a","c"]]
["forEach no fn",["throws","TypeError"]]
["forEach not fn",["throws","TypeError"]]
["iter live",[{"done":false,"value":"a"},["b","c"]]]
["iter live delete",["1",["3"]]]
["iter shapes",[[["a","1"],["b","2"]],["a","b"],["1","2"],[["a","1"],["b","2"]],true,{"done":false,"value":["a","1"]},{"done":true}]]
["iter tags",["[object URLSearchParams Iterator]","function",true,"[object URLSearchParams]"]]
["iter done stays done",[{"done":true},{"done":false,"value":"b"}]]
["iter result keys",[["done","value"],["done","value"],true]]
["entries fresh arrays",[["a","1"]]]
["enc space plus","a+b=c+d&%2B=%25"]
["enc ascii","%00%01%02%03%04%05%06%07%08%09%0A%0B%0C%0D%0E%0F%10%11%12%13%14%15%16%17%18%19%1A%1B%1C%1D%1E%1F+%21%22%23%24%25%26%27%28%29*%2B%2C-.%2F0123456789%3A%3B%3C%3D%3E%3F%40ABCDEFGHIJKLMNOPQRSTUVWXYZ%5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D%7E%7F=v"]
["enc non-ascii","%E2%82%AC=%C3%BC&%F0%9F%98%80=%C2%80"]
["enc surrogates","%EF%BF%BD=%EF%BF%BD&a%EF%BF%BDb=%EF%BF%BD%EF%BF%BD"]
["enc roundtrip",["a%3Db%26c=%3F%23%2F+%7E%21%27%28%29*",[["a=b&c","?#/ ~!'()*"]]]]
["enc empty",["","a=","=","=x"]]
["String()",["a=1","b=2"]]
["url append",["http://h/p?a=1&b=x+y#f","?a=1&b=x+y"]]
["url set","http://h/?a=1&a=2&1=2"]
["url delete all",["http://h/#f",""]]
["url delete value","http://h/?a=2"]
["url delete none","http://h/?a+b=c"]
["url sort","http://h/?a=2&b=1"]
["url sort empty","http://h/"]
["url reads after search set",[["2","3"],2,false]]
["url encoding","http://h/?%EF%BF%BD%E2%82%AC=%27%7E*"]
["url opaque","mailto:x?a=1&b=2"]
["url get/has",["a",true,["a"]]]"##;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
}

#[test]
fn text_encoder_decoder_surface_is_nodes() {
    // turbo-test#118 and the rest of the TextEncoder/TextDecoder surface, table-driven: encode
    // (lone surrogates -> U+FFFD bytes), encodeInto read/written (surrogate pairs at the buffer
    // edge), decode (U+FFFD per maximal subpart, BOM stripped unless ignoreBOM, fatal -> TypeError,
    // stream keeps partial sequences), input types (ArrayBuffer / any view honouring offset and
    // length / undefined), getters, utf-8 label normalisation and RangeError for other labels,
    // plus Buffer (which decodes through TextDecoder) keeping a BOM.
    // Expected: Node 26 (each row is [case, result] or [case, ["throws", error name, code]]).
    let p = Project::new("rc-text-codec-surface");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "t.test.js",
        r##"import { test } from 'vitest'
import * as fs from 'node:fs'
test('TextEncoder/TextDecoder', () => {
  const rows = []
  const t = (name, f) => { let r; try { r = f() } catch (e) { r = ['throws', e.name, e.code ?? null] } rows.push(JSON.stringify([name, r])) }
  const E = (s) => [...new TextEncoder().encode(s)]
  const D = (bytes, label, opts) => new TextDecoder(label, opts).decode(new Uint8Array(bytes))
  const cu = (s) => [...s].map((c) => c.codePointAt(0).toString(16))
  // encode
  t('enc ascii', () => E('aZ~'))
  t('enc 2/3/4 byte', () => E('ü€😀'))
  t('enc lone high', () => E('\uD800'))
  t('enc lone low', () => E('\uDC00'))
  t('enc high at end', () => E('a\uD83D'))
  t('enc high+high+low', () => E('\uD800😀'))
  t('enc low+high', () => E('\uDC00\uD800x'))
  t('enc nul', () => E('\0'))
  t('enc boundaries', () => E('\u007f\u0080߿ࠀ￿\u{10000}\u{10ffff}'))
  t('enc undefined', () => E(undefined))
  t('enc none', () => [...new TextEncoder().encode()])
  t('enc null', () => E(null))
  t('enc number', () => E(12.5))
  t('enc object', () => E({ toString: () => 'o' }))
  t('enc symbol', () => E(Symbol('s')))
  t('enc result type', () => { const r = new TextEncoder().encode('ab'); return [r.constructor.name, r.byteOffset, r.buffer.byteLength] })
  t('enc encoding', () => new TextEncoder().encoding)
  t('enc encoding own', () => Object.prototype.hasOwnProperty.call(new TextEncoder(), 'encoding'))
  t('enc tag', () => Object.prototype.toString.call(new TextEncoder()))
  // encodeInto
  const I = (s, n, off = 0) => { const u = new Uint8Array(n).fill(0xaa); const r = new TextEncoder().encodeInto(s, off ? u.subarray(off) : u); return [r.read, r.written, [...u]] }
  t('into fits', () => I('abc', 5))
  t('into short ascii', () => I('abcdef', 3))
  t('into multi stops', () => I('a€b', 3))
  t('into exact', () => I('a€', 4))
  t('into pair edge 3', () => I('x😀y', 4))
  t('into pair edge exact', () => I('x😀y', 5))
  t('into pair fits', () => I('😀', 4))
  t('into pair no room', () => I('😀', 3))
  t('into lone surrogate', () => I('\uD800a', 4))
  t('into lone low short', () => I('a\uDC00', 3))
  t('into empty dest', () => I('abc', 0))
  t('into empty src', () => I('', 2))
  t('into subarray', () => I('hi', 4, 1))
  t('into undefined src', () => I(undefined, 10))
  t('into keys', () => Object.keys(new TextEncoder().encodeInto('a', new Uint8Array(2))))
  t('into non-u8', () => new TextEncoder().encodeInto('a', new Uint16Array(2)))
  t('into no dest', () => new TextEncoder().encodeInto('a'))
  // decode: valid
  t('dec ascii', () => D([0x61, 0x62]))
  t('dec multi', () => D([0xc3, 0xbc, 0xe2, 0x82, 0xac, 0xf0, 0x9f, 0x98, 0x80]))
  t('dec boundaries', () => cu(D([0x7f, 0xc2, 0x80, 0xdf, 0xbf, 0xe0, 0xa0, 0x80, 0xef, 0xbf, 0xbf, 0xf0, 0x90, 0x80, 0x80, 0xf4, 0x8f, 0xbf, 0xbf])))
  t('dec nul', () => D([0, 0x41]))
  // decode: invalid (maximal subpart)
  t('dec issue118', () => D([0xff, 0x41, 0xe2, 0x82]))
  t('dec lone cont', () => D([0x80, 0xbf, 0x41]))
  t('dec overlong c0 c1', () => D([0xc0, 0x80, 0xc1, 0xbf]))
  t('dec overlong e0', () => D([0xe0, 0x80, 0x80]))
  t('dec overlong f0', () => D([0xf0, 0x80, 0x80, 0x80]))
  t('dec surrogate ed', () => D([0xed, 0xa0, 0x80, 0xed, 0x9f, 0xbf]))
  t('dec above max f4', () => D([0xf4, 0x90, 0x80, 0x80]))
  t('dec f5-ff', () => D([0xf5, 0xf8, 0xfc, 0xfe, 0xff]))
  t('dec truncated 2', () => D([0xc3]))
  t('dec truncated 3', () => D([0xe2, 0x82, 0x41]))
  t('dec truncated 4', () => D([0xf0, 0x9f, 0x98]))
  t('dec truncated 4 then ascii', () => D([0xf0, 0x9f, 0x41]))
  t('dec lead then lead', () => D([0xc3, 0xc3, 0xbc]))
  t('dec e0 bad second', () => D([0xe0, 0x9f, 0x80]))
  t('dec f4 then cont', () => D([0xf4, 0x8f, 0x41]))
  t('dec cont run', () => D([0xe2, 0x82, 0xac, 0x82, 0xac]))
  // BOM
  t('bom stripped', () => D([0xef, 0xbb, 0xbf, 0x41]))
  t('bom only first', () => D([0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf, 0x41]).length)
  t('bom ignoreBOM', () => D([0xef, 0xbb, 0xbf, 0x41], 'utf-8', { ignoreBOM: true }).length)
  t('bom mid', () => D([0x41, 0xef, 0xbb, 0xbf]).length)
  t('bom partial', () => D([0xef, 0xbb, 0x41]))
  t('bom per decode', () => { const d = new TextDecoder(); const b = new Uint8Array([0xef, 0xbb, 0xbf, 0x41]); return [d.decode(b), d.decode(b)] })
  // fatal
  t('fatal invalid', () => D([0x41, 0xff], 'utf-8', { fatal: true }))
  t('fatal truncated', () => D([0xe2, 0x82], 'utf-8', { fatal: true }))
  t('fatal valid', () => D([0x41, 0xc3, 0xbc], 'utf-8', { fatal: true }))
  t('fatal surrogate', () => D([0xed, 0xa0, 0x80], 'utf-8', { fatal: true }))
  t('fatal truthy', () => new TextDecoder('utf-8', { fatal: 1 }).fatal)
  t('fatal recovers', () => { const d = new TextDecoder('utf-8', { fatal: true }); let a; try { d.decode(new Uint8Array([0xe2]), { stream: true }); d.decode(new Uint8Array([0x41])) } catch (e) { a = e.name } return [a, d.decode(new Uint8Array([0x42]))] })
  // stream
  t('stream split 3', () => { const d = new TextDecoder(); return [d.decode(new Uint8Array([0x61, 0xe2]), { stream: true }), d.decode(new Uint8Array([0x82]), { stream: true }), d.decode(new Uint8Array([0xac, 0x62]))] })
  t('stream split 4', () => { const d = new TextDecoder(); const out = []; for (const b of [0xf0, 0x9f, 0x98, 0x80]) out.push(d.decode(new Uint8Array([b]), { stream: true })); out.push(d.decode()); return out })
  t('stream flush partial', () => { const d = new TextDecoder(); return [d.decode(new Uint8Array([0xe2, 0x82]), { stream: true }), d.decode()] })
  t('stream invalid cont', () => { const d = new TextDecoder(); return [d.decode(new Uint8Array([0xe2]), { stream: true }), d.decode(new Uint8Array([0x41]))] })
  t('stream bom split', () => { const d = new TextDecoder(); return [d.decode(new Uint8Array([0xef]), { stream: true }), d.decode(new Uint8Array([0xbb, 0xbf, 0x41]), { stream: true }), d.decode(new Uint8Array([0xef, 0xbb, 0xbf])).length] })
  t('stream reset after flush', () => { const d = new TextDecoder(); d.decode(new Uint8Array([0xe2]), { stream: true }); d.decode(); return d.decode(new Uint8Array([0x82])) })
  t('stream fatal flush', () => { const d = new TextDecoder('utf-8', { fatal: true }); return [d.decode(new Uint8Array([0xe2]), { stream: true }), d.decode()] })
  t('stream empty', () => { const d = new TextDecoder(); return [d.decode(undefined, { stream: true }), d.decode(new Uint8Array(0))] })
  // input types
  t('in ArrayBuffer', () => new TextDecoder().decode(new Uint8Array([0x68, 0x69]).buffer))
  t('in subarray', () => new TextDecoder().decode(new Uint8Array([0x61, 0x62, 0x63, 0x64]).subarray(1, 3)))
  t('in DataView', () => new TextDecoder().decode(new DataView(new Uint8Array([0x61, 0x62, 0x63, 0x64]).buffer, 1, 2)))
  t('in Uint16Array', () => new TextDecoder().decode(new Uint16Array([0x6261])))
  t('in Int8Array', () => new TextDecoder().decode(new Int8Array([-61, -68])))
  t('in Float64 offset', () => new TextDecoder().decode(new Float64Array(new Uint8Array([0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70]).buffer, 8, 1)))
  t('in Uint8ClampedArray', () => new TextDecoder().decode(new Uint8ClampedArray([0x41])))
  t('in undefined', () => new TextDecoder().decode(undefined))
  t('in none', () => new TextDecoder().decode())
  t('in null', () => new TextDecoder().decode(null))
  t('in string', () => new TextDecoder().decode('ab'))
  t('in number', () => new TextDecoder().decode(5))
  t('in array', () => new TextDecoder().decode([0x41]))
  t('in object', () => new TextDecoder().decode({ buffer: new ArrayBuffer(1) }))
  // getters / labels
  const G = (d) => [d.encoding, d.fatal, d.ignoreBOM]
  t('get default', () => G(new TextDecoder()))
  t('get opts', () => G(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true })))
  t('get undefined label', () => G(new TextDecoder(undefined, { ignoreBOM: 'x' })))
  t('get own props', () => Object.getOwnPropertyNames(new TextDecoder()))
  t('get proto', () => ['encoding', 'fatal', 'ignoreBOM'].map((k) => typeof Object.getOwnPropertyDescriptor(TextDecoder.prototype, k)?.get))
  t('dec tag', () => Object.prototype.toString.call(new TextDecoder()))
  for (const l of ['utf-8', 'UTF-8', 'utf8', 'Utf8', ' utf-8 ', '\tutf-8\n', '\f\rutf8', 'unicode-1-1-utf-8', 'unicode11utf8', 'unicode20utf8', 'x-unicode20utf8', 'UNICODE-1-1-UTF-8'])
    t('label ' + JSON.stringify(l), () => new TextDecoder(l).encoding)
  for (const l of ['foo', '', 'utf-7', 'utf_8', 'utf-8x', 'u\u0000tf-8', ' utf-8', null])
    t('label bad ' + JSON.stringify(l), () => new TextDecoder(l).encoding)
  t('label number', () => new TextDecoder(8).encoding)
  t('label object', () => new TextDecoder({ toString: () => 'UTF8' }).encoding)
  t('label symbol', () => new TextDecoder(Symbol('s')))
  t('opt null', () => G(new TextDecoder('utf-8', null)))
  t('opt number', () => new TextDecoder('utf-8', 5))
  t('dec opt null', () => new TextDecoder().decode(new Uint8Array([0x41]), null))
  t('dec opt number', () => new TextDecoder().decode(new Uint8Array([0x41]), 5))
  t('in SharedArrayBuffer', () => new TextDecoder().decode(new SharedArrayBuffer(1)))
  t('enc bad this', () => TextEncoder.prototype.encode.call({}, 'a'))
  t('dec bad this', () => TextDecoder.prototype.decode.call({}, new Uint8Array(1)))
  t('getter bad this', () => Object.getOwnPropertyDescriptor(TextDecoder.prototype, 'fatal').get.call({}))
  t('buffer keeps bom', () => Buffer.from([0xef, 0xbb, 0xbf, 0x41]).toString().length)
  t('buffer replaces invalid', () => Buffer.from([0xff, 0x41]).toString())
  t('buffer from lone surrogate', () => [...Buffer.from('\uD800')])
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"##,
    );
    let r = turbo_test(&p.dir, &["t.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
    let want = r##"["enc ascii",[97,90,126]]
["enc 2/3/4 byte",[195,188,226,130,172,240,159,152,128]]
["enc lone high",[239,191,189]]
["enc lone low",[239,191,189]]
["enc high at end",[97,239,191,189]]
["enc high+high+low",[239,191,189,240,159,152,128]]
["enc low+high",[239,191,189,239,191,189,120]]
["enc nul",[0]]
["enc boundaries",[127,194,128,223,191,224,160,128,239,191,191,240,144,128,128,244,143,191,191]]
["enc undefined",[]]
["enc none",[]]
["enc null",[110,117,108,108]]
["enc number",[49,50,46,53]]
["enc object",[111]]
["enc symbol",["throws","TypeError",null]]
["enc result type",["Uint8Array",0,2]]
["enc encoding","utf-8"]
["enc encoding own",false]
["enc tag","[object TextEncoder]"]
["into fits",[3,3,[97,98,99,170,170]]]
["into short ascii",[3,3,[97,98,99]]]
["into multi stops",[1,1,[97,170,170]]]
["into exact",[2,4,[97,226,130,172]]]
["into pair edge 3",[1,1,[120,170,170,170]]]
["into pair edge exact",[3,5,[120,240,159,152,128]]]
["into pair fits",[2,4,[240,159,152,128]]]
["into pair no room",[0,0,[170,170,170]]]
["into lone surrogate",[2,4,[239,191,189,97]]]
["into lone low short",[1,1,[97,170,170]]]
["into empty dest",[0,0,[]]]
["into empty src",[0,0,[170,170]]]
["into subarray",[2,2,[170,104,105,170]]]
["into undefined src",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["into keys",["read","written"]]
["into non-u8",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["into no dest",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["dec ascii","ab"]
["dec multi","ü€😀"]
["dec boundaries",["7f","80","7ff","800","ffff","10000","10ffff"]]
["dec nul","\u0000A"]
["dec issue118","�A�"]
["dec lone cont","��A"]
["dec overlong c0 c1","����"]
["dec overlong e0","���"]
["dec overlong f0","����"]
["dec surrogate ed","���퟿"]
["dec above max f4","����"]
["dec f5-ff","�����"]
["dec truncated 2","�"]
["dec truncated 3","�A"]
["dec truncated 4","�"]
["dec truncated 4 then ascii","�A"]
["dec lead then lead","�ü"]
["dec e0 bad second","���"]
["dec f4 then cont","�A"]
["dec cont run","€��"]
["bom stripped","A"]
["bom only first",2]
["bom ignoreBOM",2]
["bom mid",2]
["bom partial","�A"]
["bom per decode",["A","A"]]
["fatal invalid",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA"]]
["fatal truncated",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA"]]
["fatal valid","Aü"]
["fatal surrogate",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA"]]
["fatal truthy",true]
["fatal recovers",["TypeError","B"]]
["stream split 3",["a","","€b"]]
["stream split 4",["","","","😀",""]]
["stream flush partial",["","�"]]
["stream invalid cont",["","�A"]]
["stream bom split",["","A",1]]
["stream reset after flush","�"]
["stream fatal flush",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA"]]
["stream empty",["",""]]
["in ArrayBuffer","hi"]
["in subarray","bc"]
["in DataView","bc"]
["in Uint16Array","ab"]
["in Int8Array","ü"]
["in Float64 offset","ijklmnop"]
["in Uint8ClampedArray","A"]
["in undefined",""]
["in none",""]
["in null",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["in string",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["in number",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["in array",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["in object",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["get default",["utf-8",false,false]]
["get opts",["utf-8",true,true]]
["get undefined label",["utf-8",false,true]]
["get own props",[]]
["get proto",["function","function","function"]]
["dec tag","[object TextDecoder]"]
["label \"utf-8\"","utf-8"]
["label \"UTF-8\"","utf-8"]
["label \"utf8\"","utf-8"]
["label \"Utf8\"","utf-8"]
["label \" utf-8 \"","utf-8"]
["label \"\\tutf-8\\n\"","utf-8"]
["label \"\\f\\rutf8\"","utf-8"]
["label \"unicode-1-1-utf-8\"","utf-8"]
["label \"unicode11utf8\"","utf-8"]
["label \"unicode20utf8\"","utf-8"]
["label \"x-unicode20utf8\"","utf-8"]
["label \"UNICODE-1-1-UTF-8\"","utf-8"]
["label bad \"foo\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label bad \"\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label bad \"utf-7\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label bad \"utf_8\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label bad \"utf-8x\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label bad \"u\\u0000tf-8\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label bad \" utf-8\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label bad null",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label number",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED"]]
["label object","utf-8"]
["label symbol",["throws","TypeError",null]]
["opt null",["utf-8",false,false]]
["opt number",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["dec opt null","A"]
["dec opt number",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["in SharedArrayBuffer","\u0000"]
["enc bad this",["throws","TypeError",null]]
["dec bad this",["throws","TypeError","ERR_INVALID_THIS"]]
["getter bad this",["throws","TypeError","ERR_INVALID_THIS"]]
["buffer keeps bom",2]
["buffer replaces invalid","�A"]
["buffer from lone surrogate",[239,191,189]]"##;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
}

#[test]
fn blob_text_decodes_the_concatenated_bytes_like_nodes() {
    // turbo-test#120: Blob#text UTF-8-decodes the bytes of all parts at once (a sequence split
    // across parts is one character, one leading BOM is stripped), whatever the part types
    // (string, ArrayBuffer, any view honouring offset and length, nested Blob, File, other values
    // stringified). Expected: Node 26 (each row is [case, result]).
    let p = Project::new("rc-blob-text");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "t.test.js",
        r##"import { test } from 'vitest'
import * as fs from 'node:fs'
test('Blob#text', async () => {
  const rows = []
const t = async (name, f) => { let r; try { r = await f() } catch (e) { r = ['throws', e.name, e.code ?? null] } rows.push(JSON.stringify([name, r])) }
const B = (...parts) => new Blob(parts).text()
const cu = (s) => [...s].map((c) => c.codePointAt(0).toString(16))
await t('split 3-byte', () => B(new Uint8Array([0xe2, 0x82]), new Uint8Array([0xac])))
await t('split 4-byte over 3 parts', () => B(new Uint8Array([0xf0]), new Uint8Array([0x9f, 0x98]), new Uint8Array([0x80])))
await t('split string then bytes', () => B('a', new Uint8Array([0xc3]), new Uint8Array([0xbc]), 'b'))
await t('truncated then string', () => B(new Uint8Array([0xe2, 0x82]), 'x'))
await t('bom stripped', async () => cu(await B(new Uint8Array([0xef, 0xbb, 0xbf, 0x41]))))
await t('bom split', async () => cu(await B(new Uint8Array([0xef]), new Uint8Array([0xbb, 0xbf]), 'A')))
await t('bom string part', async () => cu(await B('﻿A')))
await t('bom second only', async () => cu(await B('A', new Uint8Array([0xef, 0xbb, 0xbf]))))
await t('two boms', async () => cu(await B(new Uint8Array([0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf]))))
await t('invalid', () => B(new Uint8Array([0xff, 0x41])))
await t('ArrayBuffer part', () => B(new Uint8Array([0xe2, 0x82]).buffer, new Uint8Array([0xac]).buffer))
await t('subarray part', () => B(new Uint8Array([0x61, 0x62, 0x63, 0x64]).subarray(1, 3)))
await t('DataView part', () => B(new DataView(new Uint8Array([0x61, 0x62, 0x63, 0x64]).buffer, 1, 2)))
await t('Uint16Array part', () => B(new Uint16Array([0x6261])))
await t('nested blob bytes', () => B(new Blob([new Uint8Array([0xe2, 0x82])]), new Uint8Array([0xac])))
await t('nested blob string', () => B('<', new Blob(['é', new Blob(['ü'])]), '>'))
await t('lone surrogate string', async () => cu(await B('\uD800x')))
await t('number part', () => B(12, null, undefined))
await t('object part', () => B({ toString: () => 'o' }))
await t('empty', () => B())
await t('file', () => new File([new Uint8Array([0xe2, 0x82]), new Uint8Array([0xac])], 'f.txt').text())
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"##,
    );
    let r = turbo_test(&p.dir, &["t.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
    let want = r##"["split 3-byte","€"]
["split 4-byte over 3 parts","😀"]
["split string then bytes","aüb"]
["truncated then string","�x"]
["bom stripped",["41"]]
["bom split",["41"]]
["bom string part",["41"]]
["bom second only",["41","feff"]]
["two boms",["feff"]]
["invalid","�A"]
["ArrayBuffer part","€"]
["subarray part","bc"]
["DataView part","bc"]
["Uint16Array part","ab"]
["nested blob bytes","€"]
["nested blob string","<éü>"]
["lone surrogate string",["fffd","78"]]
["number part","12nullundefined"]
["object part","o"]
["empty",""]
["file","€"]"##;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
}

#[test]
fn text_decoder_legacy_encodings_are_nodes() {
    // turbo-test#119: TextDecoder takes every WHATWG label Node 26 does (the replacement
    // encoding's labels throw RangeError naming "replacement") and decodes the legacy encodings:
    // single-byte tables (windows-1252 for latin1 / ascii), UTF-16LE/BE (lone surrogates and an
    // odd byte -> U+FFFD, the encoding's own BOM stripped unless ignoreBOM) and the CJK multi-byte
    // encodings, with fatal (TypeError naming the encoding), stream (partial sequences and the
    // ISO-2022-JP mode carried over) and every BufferSource input. Rows only where Node's ICU
    // decoders agree with the Encoding Standard (vitest.compat.md lists where they do not).
    // Expected: Node 26 (each row is [case, result] or [case, ["throws", name, code, message]]).
    let p = Project::new("rc-text-decoder-legacy");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "t.test.js",
        r##"import { test } from 'vitest'
import * as fs from 'node:fs'
test('TextDecoder legacy encodings', () => {
  const rows = []
const t = (name, f) => { let r; try { r = f() } catch (e) { r = ['throws', e.name, e.code ?? null, e.message] } rows.push(JSON.stringify([name, r])) }
const cu = (s) => [...s].map((c) => c.codePointAt(0).toString(16))
const D = (label, bytes, opts) => cu(new TextDecoder(label, opts).decode(new Uint8Array(bytes)))
// labels -> encoding (issue #119 list, aliases, case/whitespace, every WHATWG encoding)
for (const l of ['latin1', 'utf-16le', 'windows-1252', 'shift_jis', 'utf-16be', 'iso-8859-2', 'ascii', 'ISO-8859-1', ' us-ascii\n', 'utf-16', 'UCS-2', 'unicodefffe', 'csshiftjis', 'sjis', 'ms_kanji', 'cp1250', 'iso-8859-8-i', 'iso_8859-15', 'l9', 'koi8-u', 'koi', 'mac', 'x-mac-cyrillic', 'dos-874', 'tis-620', 'ibm866', 'gbk', 'gb2312', 'x-gbk', 'gb18030', 'big5', 'big5-hkscs', 'euc-jp', 'iso-2022-jp', 'euc-kr', 'ks_c_5601-1987', 'windows-949', 'x-user-defined', 'iso-8859-16', 'windows-1258', 'x-cp1251'])
  t('label ' + JSON.stringify(l), () => new TextDecoder(l).encoding)
// rejected labels: the replacement encoding and near-misses
for (const l of ['replacement', 'iso-2022-kr', 'hz-gb-2312', 'iso-2022-cn', 'csiso2022kr', 'utf-32', 'latin-1', 'utf16', 'windows1252', 'shift-jis'])
  t('label bad ' + JSON.stringify(l), () => new TextDecoder(l).encoding)
// getters on a legacy decoder
t('get legacy', () => { const d = new TextDecoder('Shift_JIS', { fatal: true, ignoreBOM: true }); return [d.encoding, d.fatal, d.ignoreBOM] })
// single-byte
t('w1252 high', () => D('windows-1252', [0x41, 0x80, 0x81, 0x8d, 0x8f, 0x90, 0x9d, 0x9f, 0xa0, 0xe9, 0xff]))
t('latin1 is w1252', () => D('latin1', [0x80, 0x9f, 0xe9]))
t('ascii is w1252', () => D('ascii', [0x80, 0xff]))
t('8859-2', () => D('iso-8859-2', [0xa1, 0xb1, 0xe6, 0xff]))
t('8859-3 unmapped', () => D('iso-8859-3', [0xa5, 0x41, 0xae]))
t('8859-3 fatal', () => D('iso-8859-3', [0x41, 0xa5], { fatal: true }))
t('8859-7 euro', () => D('iso-8859-7', [0xa4, 0xc1]))
t('8859-8-i', () => D('iso-8859-8-i', [0xe0, 0xfa]))
t('koi8-r', () => D('koi8-r', [0xc1, 0xff, 0xa3]))
t('koi8-u', () => D('koi8-u', [0xa4, 0xad]))
t('ibm866', () => D('ibm866', [0x80, 0xb0, 0xff]))
t('macintosh', () => D('macintosh', [0x80, 0xa5, 0xdb]))
t('x-mac-cyrillic', () => D('x-mac-cyrillic', [0x80, 0xff]))
t('windows-874', () => D('windows-874', [0x80, 0xa1, 0xdb]))
t('windows-1251', () => D('windows-1251', [0x80, 0x98, 0xc0]))
t('windows-1258', () => D('windows-1258', [0xcc, 0xd2]))
t('x-user-defined', () => D('x-user-defined', [0x41, 0x7f, 0x80, 0xff]))
t('single-byte bom kept', () => D('windows-1252', [0xef, 0xbb, 0xbf]))
// UTF-16
t('u16le', () => D('utf-16le', [0x41, 0x00, 0xac, 0x20, 0x3d, 0xd8, 0x00, 0xde]))
t('u16be', () => D('utf-16be', [0x00, 0x41, 0x20, 0xac, 0xd8, 0x3d, 0xde, 0x00]))
t('u16le odd', () => D('utf-16le', [0x41, 0x00, 0x42]))
t('u16le lone high', () => D('utf-16le', [0x00, 0xd8, 0x41, 0x00]))
t('u16le lone high end', () => D('utf-16le', [0x41, 0x00, 0x00, 0xd8]))
t('u16le lone low', () => D('utf-16le', [0x00, 0xdc, 0x41, 0x00]))
t('u16be lone high', () => D('utf-16be', [0xd8, 0x00, 0x00, 0x41]))
t('u16le bom', () => D('utf-16le', [0xff, 0xfe, 0x41, 0x00]))
t('u16le bom only first', () => D('utf-16le', [0xff, 0xfe, 0xff, 0xfe, 0x41, 0x00]))
t('u16le bom ignore', () => D('utf-16le', [0xff, 0xfe, 0x41, 0x00], { ignoreBOM: true }))
t('u16le other bom', () => D('utf-16le', [0xfe, 0xff, 0x41, 0x00]))
t('u16be bom', () => D('utf-16be', [0xfe, 0xff, 0x00, 0x41]))
t('u16be bom ignore', () => D('utf-16be', [0xfe, 0xff, 0x00, 0x41], { ignoreBOM: true }))
t('u16le utf8 bom kept', () => D('utf-16le', [0xef, 0xbb]))
t('u16le fatal odd', () => D('utf-16le', [0x41, 0x00, 0x42], { fatal: true }))
t('u16le fatal lone', () => D('utf-16le', [0x00, 0xd8, 0x41, 0x00], { fatal: true }))
t('u16le fatal valid', () => D('utf-16le', [0x3d, 0xd8, 0x00, 0xde], { fatal: true }))
t('u16 label is le', () => D('utf-16', [0x41, 0x00]))
// CJK (cases where ICU and WHATWG agree)
t('shift_jis', () => D('shift_jis', [0x82, 0xa0, 0x41, 0x5c, 0x7e, 0xb1, 0x88, 0x9f, 0x81, 0x40]))
t('shift_jis truncated', () => D('shift_jis', [0x41, 0x82]))
t('shift_jis bad trail', () => D('shift_jis', [0x82, 0x20]))
t('shift_jis fatal', () => D('shift_jis', [0x82], { fatal: true }))
t('euc-jp', () => D('euc-jp', [0xa4, 0xa2, 0x8e, 0xb1, 0x8f, 0xb0, 0xa1, 0x41]))
t('euc-jp truncated', () => D('euc-jp', [0xa4]))
t('iso-2022-jp', () => D('iso-2022-jp', [0x1b, 0x24, 0x42, 0x24, 0x22, 0x1b, 0x28, 0x42, 0x41]))
t('gbk', () => D('gbk', [0xc4, 0xe3, 0xba, 0xc3, 0x80, 0x41]))
t('gb18030 4-byte', () => D('gb18030', [0x81, 0x30, 0x81, 0x30, 0x95, 0x32, 0x82, 0x36]))
t('gb18030 truncated', () => D('gb18030', [0x81, 0x30]))
t('big5', () => D('big5', [0xa4, 0x40, 0xa5, 0x41, 0x41]))
t('big5 truncated', () => D('big5', [0xa4]))
t('euc-kr', () => D('euc-kr', [0xb0, 0xa1, 0xc7, 0xd1, 0x41]))
t('euc-kr truncated', () => D('euc-kr', [0xb0]))
t('euc-kr fatal', () => D('euc-kr', [0xb0], { fatal: true }))
// stream
const S = (label, chunks, opts) => { const d = new TextDecoder(label, opts); const out = chunks.map((c, i) => d.decode(new Uint8Array(c), { stream: i < chunks.length - 1 })); return out.map(cu) }
t('stream u16le split unit', () => S('utf-16le', [[0x41], [0x00, 0x42], [0x00]]))
t('stream u16le split pair', () => S('utf-16le', [[0x3d, 0xd8], [0x00, 0xde], []]))
t('stream u16le flush odd', () => S('utf-16le', [[0x41, 0x00, 0x42], []]))
t('stream u16le bom split', () => S('utf-16le', [[0xff], [0xfe, 0x41, 0x00], []]))
t('stream u16le bom once', () => S('utf-16le', [[0xff, 0xfe, 0x41, 0x00], [0xff, 0xfe], []]))
t('stream sjis split', () => S('shift_jis', [[0x41, 0x82], [0xa0], []]))
t('stream sjis flush', () => S('shift_jis', [[0x82], []]))
t('stream gb18030 split', () => S('gb18030', [[0x81], [0x30, 0x81], [0x30]]))
t('stream iso-2022-jp state', () => S('iso-2022-jp', [[0x1b, 0x24, 0x42], [0x24, 0x22], [0x1b, 0x28, 0x42, 0x41]]))
t('stream reset after flush', () => { const d = new TextDecoder('utf-16le'); d.decode(new Uint8Array([0x41]), { stream: true }); return [cu(d.decode()), cu(d.decode(new Uint8Array([0xff, 0xfe, 0x42, 0x00])))] })
t('stream fatal flush', () => { const d = new TextDecoder('utf-16le', { fatal: true }); return [d.decode(new Uint8Array([0x41]), { stream: true }), d.decode()] })
t('fatal recovers', () => { const d = new TextDecoder('shift_jis', { fatal: true }); let a; try { d.decode(new Uint8Array([0x82]), { stream: true }); d.decode(new Uint8Array([0x20])) } catch (e) { a = e.name } return [a, cu(d.decode(new Uint8Array([0x82, 0xa0])))] })
// input types reach the legacy path too
t('legacy ArrayBuffer', () => cu(new TextDecoder('utf-16le').decode(new Uint8Array([0x41, 0x00]).buffer)))
t('legacy subarray', () => cu(new TextDecoder('windows-1252').decode(new Uint8Array([0x41, 0x80, 0x42]).subarray(1, 2))))
t('legacy DataView', () => cu(new TextDecoder('utf-16be').decode(new DataView(new Uint8Array([0x00, 0x00, 0x41, 0x00]).buffer, 1, 2))))
t('legacy undefined', () => new TextDecoder('shift_jis').decode())
t('legacy bad input', () => new TextDecoder('shift_jis').decode('ab'))
t('legacy long', () => { const b = new Uint8Array(20000).fill(0xe9); const s = new TextDecoder('latin1').decode(b); return [s.length, s.charCodeAt(0), s.charCodeAt(19999)] })
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"##,
    );
    let r = turbo_test(&p.dir, &["t.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
    let want = r##"["label \"latin1\"","windows-1252"]
["label \"utf-16le\"","utf-16le"]
["label \"windows-1252\"","windows-1252"]
["label \"shift_jis\"","shift_jis"]
["label \"utf-16be\"","utf-16be"]
["label \"iso-8859-2\"","iso-8859-2"]
["label \"ascii\"","windows-1252"]
["label \"ISO-8859-1\"","windows-1252"]
["label \" us-ascii\\n\"","windows-1252"]
["label \"utf-16\"","utf-16le"]
["label \"UCS-2\"","utf-16le"]
["label \"unicodefffe\"","utf-16be"]
["label \"csshiftjis\"","shift_jis"]
["label \"sjis\"","shift_jis"]
["label \"ms_kanji\"","shift_jis"]
["label \"cp1250\"","windows-1250"]
["label \"iso-8859-8-i\"","iso-8859-8-i"]
["label \"iso_8859-15\"","iso-8859-15"]
["label \"l9\"","iso-8859-15"]
["label \"koi8-u\"","koi8-u"]
["label \"koi\"","koi8-r"]
["label \"mac\"","macintosh"]
["label \"x-mac-cyrillic\"","x-mac-cyrillic"]
["label \"dos-874\"","windows-874"]
["label \"tis-620\"","windows-874"]
["label \"ibm866\"","ibm866"]
["label \"gbk\"","gbk"]
["label \"gb2312\"","gbk"]
["label \"x-gbk\"","gbk"]
["label \"gb18030\"","gb18030"]
["label \"big5\"","big5"]
["label \"big5-hkscs\"","big5"]
["label \"euc-jp\"","euc-jp"]
["label \"iso-2022-jp\"","iso-2022-jp"]
["label \"euc-kr\"","euc-kr"]
["label \"ks_c_5601-1987\"","euc-kr"]
["label \"windows-949\"","euc-kr"]
["label \"x-user-defined\"","x-user-defined"]
["label \"iso-8859-16\"","iso-8859-16"]
["label \"windows-1258\"","windows-1258"]
["label \"x-cp1251\"","windows-1251"]
["label bad \"replacement\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"replacement\" encoding is not supported"]]
["label bad \"iso-2022-kr\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"replacement\" encoding is not supported"]]
["label bad \"hz-gb-2312\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"replacement\" encoding is not supported"]]
["label bad \"iso-2022-cn\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"replacement\" encoding is not supported"]]
["label bad \"csiso2022kr\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"replacement\" encoding is not supported"]]
["label bad \"utf-32\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"utf-32\" encoding is not supported"]]
["label bad \"latin-1\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"latin-1\" encoding is not supported"]]
["label bad \"utf16\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"utf16\" encoding is not supported"]]
["label bad \"windows1252\"",["throws","RangeError","ERR_ENCODING_NOT_SUPPORTED","The \"windows1252\" encoding is not supported"]]
["label bad \"shift-jis\"","shift_jis"]
["get legacy",["shift_jis",true,true]]
["w1252 high",["41","20ac","81","8d","8f","90","9d","178","a0","e9","ff"]]
["latin1 is w1252",["20ac","178","e9"]]
["ascii is w1252",["20ac","ff"]]
["8859-2",["104","105","107","2d9"]]
["8859-3 unmapped",["fffd","41","fffd"]]
["8859-3 fatal",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA","The encoded data was not valid for encoding iso-8859-3"]]
["8859-7 euro",["20ac","391"]]
["8859-8-i",["5d0","5ea"]]
["koi8-r",["430","42a","451"]]
["koi8-u",["454","491"]]
["ibm866",["410","2591","a0"]]
["macintosh",["c4","2022","20ac"]]
["x-mac-cyrillic",["410","20ac"]]
["windows-874",["20ac","e01","fffd"]]
["windows-1251",["402","98","410"]]
["windows-1258",["300","309"]]
["x-user-defined",["41","7f","f780","f7ff"]]
["single-byte bom kept",["ef","bb","bf"]]
["u16le",["41","20ac","1f600"]]
["u16be",["41","20ac","1f600"]]
["u16le odd",["41","fffd"]]
["u16le lone high",["fffd","41"]]
["u16le lone high end",["41","fffd"]]
["u16le lone low",["fffd","41"]]
["u16be lone high",["fffd","41"]]
["u16le bom",["41"]]
["u16le bom only first",["feff","41"]]
["u16le bom ignore",["feff","41"]]
["u16le other bom",["fffe","41"]]
["u16be bom",["41"]]
["u16be bom ignore",["feff","41"]]
["u16le utf8 bom kept",["bbef"]]
["u16le fatal odd",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA","The encoded data was not valid for encoding utf-16le"]]
["u16le fatal lone",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA","The encoded data was not valid for encoding utf-16le"]]
["u16le fatal valid",["1f600"]]
["u16 label is le",["41"]]
["shift_jis",["3042","41","5c","7e","ff71","4e9c","3000"]]
["shift_jis truncated",["41","fffd"]]
["shift_jis bad trail",["fffd","20"]]
["shift_jis fatal",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA","The encoded data was not valid for encoding shift_jis"]]
["euc-jp",["3042","ff71","4e02","41"]]
["euc-jp truncated",["fffd"]]
["iso-2022-jp",["3042","41"]]
["gbk",["4f60","597d","20ac","41"]]
["gb18030 4-byte",["80","20000"]]
["gb18030 truncated",["fffd"]]
["big5",["4e00","4e15","41"]]
["big5 truncated",["fffd"]]
["euc-kr",["ac00","d55c","41"]]
["euc-kr truncated",["fffd"]]
["euc-kr fatal",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA","The encoded data was not valid for encoding euc-kr"]]
["stream u16le split unit",[[],["41"],["42"]]]
["stream u16le split pair",[[],["1f600"],[]]]
["stream u16le flush odd",[["41"],["fffd"]]]
["stream u16le bom split",[[],["41"],[]]]
["stream u16le bom once",[["41"],["feff"],[]]]
["stream sjis split",[["41"],["3042"],[]]]
["stream sjis flush",[[],["fffd"]]]
["stream gb18030 split",[[],[],["80"]]]
["stream iso-2022-jp state",[[],["3042"],["41"]]]
["stream reset after flush",[["fffd"],["42"]]]
["stream fatal flush",["throws","TypeError","ERR_ENCODING_INVALID_ENCODED_DATA","The encoded data was not valid for encoding utf-16le"]]
["fatal recovers",["TypeError",["3042"]]]
["legacy ArrayBuffer",["41"]]
["legacy subarray",["20ac"]]
["legacy DataView",["41"]]
["legacy undefined",""]
["legacy bad input",["throws","TypeError","ERR_INVALID_ARG_TYPE","The \"input\" argument must be an instance of SharedArrayBuffer, ArrayBuffer or ArrayBufferView."]]
["legacy long",[20000,233,233]]"##;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
}

#[test]
fn blob_and_file_surface_is_nodes() {
    // turbo-test#128: Blob / File as Node 26: size in UTF-8 bytes, type lower-cased (or '' with a
    // character outside U+0020..U+007E), parts copied at construction, endings, arrayBuffer() /
    // bytes() / text() / stream() (one chunk per non-empty part) / textStream(), slice() with
    // clamped and negative bounds and its own type, accessors on the prototype, toStringTag, the
    // argument errors. Expected: Node 26.11 (each row is [case, result] or
    // [case, ["throws", name, code]]).
    let p = Project::new("rc-blob-surface");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "t.test.js",
        r##"import { test } from 'vitest'
import * as fs from 'node:fs'
test('Blob / File surface', async () => {
  const rows = []
const t = async (name, f) => { let r; try { r = await f() } catch (e) { r = ['throws', e.name, e.code ?? null] } rows.push(JSON.stringify([name, r])) }
const u8 = (x) => [...new Uint8Array(x)]
const b = new Blob(['€', new Uint8Array([1, 2])], { type: 'TEXT/Plain' })
await t('size mixed', () => b.size)
await t('size astral', () => new Blob(['😀', 'é']).size)
await t('size lone surrogate', () => new Blob(['\uD800']).size)
await t('size nested', () => new Blob([b, new ArrayBuffer(4), new Uint16Array(3)]).size)
await t('size empty', () => new Blob().size)
await t('type lowercased', () => b.type)
await t('type default', () => new Blob([]).type)
await t('type non-ascii', () => new Blob([], { type: 'text/plainé' }).type)
await t('type control', () => new Blob([], { type: 'a\nb' }).type)
await t('type space kept', () => new Blob([], { type: ' A B ' }).type)
await t('type number', () => new Blob([], { type: 12 }).type)
await t('arrayBuffer', async () => u8(await b.arrayBuffer()))
await t('arrayBuffer instance', async () => (await b.arrayBuffer()) instanceof ArrayBuffer)
await t('bytes typeof', () => typeof b.bytes)
await t('bytes', async () => { const x = await b.bytes(); return [x.constructor.name, [...x]] })
await t('bytes fresh copy', async () => { const x = await b.bytes(); x[0] = 0; return [...(await b.bytes())] })
await t('source copied', async () => { const s = new Uint8Array([5, 6]); const c = new Blob([s]); s[0] = 9; return [...(await c.bytes())] })
await t('slice(1)', () => new Blob(['abc']).slice(1).text())
await t('slice(1,2)', () => new Blob(['abcd']).slice(1, 2).text())
await t('slice(-2)', () => new Blob(['abcd']).slice(-2).text())
await t('slice(-10, -1)', () => new Blob(['abcd']).slice(-10, -1).text())
await t('slice(3,1)', async () => { const s = new Blob(['abcd']).slice(3, 1); return [s.size, await s.text()] })
await t('slice(10)', () => new Blob(['abcd']).slice(10).size)
await t('slice(1.7, 3.2)', () => new Blob(['abcd']).slice(1.7, 3.2).text())
await t('slice() keeps nothing of type', () => b.slice().type)
await t('slice type', () => b.slice(0, 1, 'X/Y').type)
await t('slice type non-ascii', () => b.slice(0, 1, 'é').type)
await t('slice bytes', async () => [...(await b.slice(1, 4).bytes())])
await t('slice returns Blob', () => Object.prototype.toString.call(new File(['ab'], 'f').slice(0, 1)))
await t('stream typeof', () => typeof b.stream())
await t('stream instance', () => b.stream() instanceof ReadableStream)
await t('stream read', async () => { const r = b.stream().getReader(); const out = []; for (;;) { const { done, value } = await r.read(); if (done) break; out.push(value.constructor.name, ...value) } return out })
await t('stream empty', async () => { const r = new Blob().stream().getReader(); return (await r.read()).done })
await t('toStringTag Blob', () => Object.prototype.toString.call(b))
await t('toStringTag File', () => Object.prototype.toString.call(new File([], 'f')))
await t('File instanceof Blob', () => new File([], 'f') instanceof Blob)
await t('File name', () => new File([], 12).name)
await t('File size type', () => { const f = new File(['€x'], 'f', { type: 'A/B', lastModified: 7 }); return [f.size, f.type, f.lastModified] })
await t('File lastModified default', () => typeof new File([], 'f').lastModified)
await t('File bytes', async () => [...(await new File(['hi'], 'f').bytes())])
await t('File slice', () => new File(['hello'], 'f').slice(1, 3).text())
await t('own keys', () => Object.keys(b))
const chunks = async (s) => { const r = s.getReader(); const out = []; for (;;) { const { done, value } = await r.read(); if (done) break; out.push(typeof value === 'string' ? value : [...value]) } return out }
await t('stream chunks per part', () => chunks(new Blob(['', 'a', new Blob(['b', 'c']), new Uint8Array(0), 'd']).stream()))
await t('stream chunks of slice', () => chunks(new Blob(['abc', 'def']).slice(1, 5).stream()))
await t('textStream', () => chunks(new Blob(['ab', 'c']).textStream()))
await t('textStream split char', () => chunks(new Blob([new Uint8Array([0xe2, 0x82]), new Uint8Array([0xac])]).textStream()))
await t('endings native', () => new Blob(['a\rb\r\nc\n'], { endings: 'native' }).text())
await t('endings transparent', () => new Blob(['a\rb\r\nc\n'], { endings: 'transparent' }).size)
await t('endings bogus', () => new Blob([], { endings: 'bogus' }))
await t('parts iterable', () => new Blob(new Set(['a', 'b'])).text())
await t('parts undefined', () => new Blob(undefined).size)
await t('parts null', () => new Blob(null))
await t('parts string', () => new Blob('abc'))
await t('parts plain object', () => new Blob({}))
await t('options string', () => new Blob([], 'x'))
await t('options null', () => new Blob([], null).type)
await t('Blob() without new', () => Blob())
await t('prototype size', () => Blob.prototype.size)
await t('slice string start', () => new Blob(['ab']).slice('1').text())
await t('slice undefined start', () => new Blob(['ab']).slice(undefined, 1).text())
await t('File missing args', () => new File([]))
await t('File lastModified coerced', () => [new File([], 'n', { lastModified: '5' }).lastModified, new File([], 'n', { lastModified: 1.9 }).lastModified])
await t('lengths', () => [Blob.name, Blob.length, File.name, File.length])
await t('Blob.prototype names', () => Object.getOwnPropertyNames(Blob.prototype))
await t('File.prototype names', () => Object.getOwnPropertyNames(File.prototype))
await t('File name is accessor', () => typeof Object.getOwnPropertyDescriptor(File.prototype, 'name').get)
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"##,
    );
    let r = turbo_test(&p.dir, &["t.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
    let want = r##"["size mixed",5]
["size astral",6]
["size lone surrogate",3]
["size nested",15]
["size empty",0]
["type lowercased","text/plain"]
["type default",""]
["type non-ascii",""]
["type control",""]
["type space kept"," a b "]
["type number","12"]
["arrayBuffer",[226,130,172,1,2]]
["arrayBuffer instance",true]
["bytes typeof","function"]
["bytes",["Uint8Array",[226,130,172,1,2]]]
["bytes fresh copy",[226,130,172,1,2]]
["source copied",[5,6]]
["slice(1)","bc"]
["slice(1,2)","b"]
["slice(-2)","cd"]
["slice(-10, -1)","abc"]
["slice(3,1)",[0,""]]
["slice(10)",0]
["slice(1.7, 3.2)","c"]
["slice() keeps nothing of type",""]
["slice type","x/y"]
["slice type non-ascii",""]
["slice bytes",[130,172,1]]
["slice returns Blob","[object Blob]"]
["stream typeof","object"]
["stream instance",true]
["stream read",["Uint8Array",226,130,172,"Uint8Array",1,2]]
["stream empty",true]
["toStringTag Blob","[object Blob]"]
["toStringTag File","[object File]"]
["File instanceof Blob",true]
["File name","12"]
["File size type",[4,"a/b",7]]
["File lastModified default","number"]
["File bytes",[104,105]]
["File slice","el"]
["own keys",[]]
["stream chunks per part",[[97],[98],[99],[100]]]
["stream chunks of slice",[[98,99],[100,101]]]
["textStream",["ab","c"]]
["textStream split char",["€"]]
["endings native","a\nb\nc\n"]
["endings transparent",7]
["endings bogus",["throws","TypeError","ERR_INVALID_ARG_VALUE"]]
["parts iterable","ab"]
["parts undefined",0]
["parts null",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["parts string",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["parts plain object",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["options string",["throws","TypeError","ERR_INVALID_ARG_TYPE"]]
["options null",""]
["Blob() without new",["throws","TypeError",null]]
["prototype size",["throws","TypeError","ERR_INVALID_THIS"]]
["slice string start","b"]
["slice undefined start","a"]
["File missing args",["throws","TypeError","ERR_MISSING_ARGS"]]
["File lastModified coerced",[5,1.9]]
["lengths",["Blob",0,"File",2]]
["Blob.prototype names",["constructor","type","size","slice","arrayBuffer","text","bytes","stream","textStream"]]
["File.prototype names",["constructor","name","lastModified"]]
["File name is accessor","function"]"##;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
}

#[test]
fn file_reader_reads_the_blob_bytes() {
    // turbo-test#132: FileReader's readAsArrayBuffer / readAsDataURL / readAsBinaryString read the
    // Blob's bytes (UTF-8 text and non-UTF-8 binary alike), not char codes of its decoded text.
    // Node 26 has no FileReader; expected: jsdom (vitest's jsdom environment; happy-dom gives the
    // same rows). Each row is [case, result].
    let p = Project::new("rc-file-reader-bytes");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "t.test.js",
        r##"import { test } from 'vitest'
import * as fs from 'node:fs'
test('FileReader bytes', async () => {
  const rows = []
const t = async (name, f) => { let r; try { r = await f() } catch (e) { r = ['throws', e.name, e.code ?? null] } rows.push(JSON.stringify([name, r])) }
const read = (kind, blob) => new Promise((res) => { const r = new FileReader(); r.onload = () => res(r.result); r.onerror = () => res(['error', String(r.error)]); r[kind](blob) })
const ab = async (blob) => { const x = await read('readAsArrayBuffer', blob); return [x instanceof ArrayBuffer, [...new Uint8Array(x)]] }
const bin = [0x00, 0x7f, 0x80, 0xc3, 0x28, 0xff, 0xfe]
await t('arrayBuffer utf8', () => ab(new Blob(['€é'])))
await t('arrayBuffer binary', () => ab(new Blob([new Uint8Array(bin)])))
await t('arrayBuffer empty', () => ab(new Blob([])))
await t('arrayBuffer file slice', () => ab(new File(['a€b'], 'f').slice(1, 4)))
await t('dataURL utf8 typed', () => read('readAsDataURL', new Blob(['€é'], { type: 'text/plain' })))
await t('dataURL binary untyped', () => read('readAsDataURL', new Blob([new Uint8Array(bin)])))
await t('dataURL empty', () => read('readAsDataURL', new Blob([], { type: 'a/b' })))
await t('binaryString utf8', () => read('readAsBinaryString', new Blob(['€é'])).then((s) => [...s].map((c) => c.charCodeAt(0))))
await t('binaryString binary', () => read('readAsBinaryString', new Blob([new Uint8Array(bin)])).then((s) => [...s].map((c) => c.charCodeAt(0))))
await t('text utf8', () => read('readAsText', new Blob(['€é'])))
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"##,
    );
    let r = turbo_test(&p.dir, &["t.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
    let want = r##"["arrayBuffer utf8",[true,[226,130,172,195,169]]]
["arrayBuffer binary",[true,[0,127,128,195,40,255,254]]]
["arrayBuffer empty",[true,[]]]
["arrayBuffer file slice",[true,[226,130,172]]]
["dataURL utf8 typed","data:text/plain;base64,4oKsw6k="]
["dataURL binary untyped","data:application/octet-stream;base64,AH+Awyj//g=="]
["dataURL empty","data:a/b;base64,"]
["binaryString utf8",[226,130,172,195,169]]
["binaryString binary",[0,127,128,195,40,255,254]]
["text utf8","€é"]"##;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
}

#[test]
fn file_reader_events_encoding_and_abort_are_jsdoms() {
    // turbo-test#139: FileReader as jsdom's (vitest's jsdom environment; Node 26 has none):
    // readyState LOADING during a read, loadstart / progress / load / loadend with their
    // readyState and progress fields, every listener (addEventListener deduplicated,
    // removeEventListener by function, on<type> handlers in their slot), readAsText's encoding
    // (BOM sniffing first, an unknown label falls back to UTF-8), abort() in each state and the
    // InvalidStateError for a read while LOADING. Each row is [case, result].
    let p = Project::new("rc-file-reader-events");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "t.test.js",
        r##"import { test } from 'vitest'
import * as fs from 'node:fs'
test('FileReader events', async () => {
  const rows = []
const t = async (name, f) => { let r; try { r = await f() } catch (e) { r = ['throws', e.name, e.code ?? null] } rows.push(JSON.stringify([name, r])) }
const sleep = (ms) => new Promise((res) => setTimeout(res, ms))
const run = async (setup) => { const r = new FileReader(); const ev = []; const log = (tag) => (e) => ev.push([tag, e.type, r.readyState, e.target === r, e.loaded ?? null, e.total ?? null, e.lengthComputable ?? null]); await setup(r, log, ev); await sleep(30); return [r.readyState, r.result, r.error && r.error.name, ev] }
const all = (r, log) => { for (const n of ['loadstart', 'progress', 'load', 'abort', 'error', 'loadend']) r.addEventListener(n, log('L')) }
await t('constants', () => [FileReader.EMPTY, FileReader.LOADING, FileReader.DONE, FileReader.prototype.LOADING])
await t('initial', () => { const r = new FileReader(); return [r.readyState, r.result, r.error] })
await t('events readAsText', () => run((r, log, ev) => { all(r, log); r.readAsText(new Blob(['héllo'])); ev.push(['sync', r.readyState, r.result]) }))
await t('events arrayBuffer', () => run((r, log, ev) => { all(r, log); r.readAsArrayBuffer(new Blob([new Uint8Array([1, 2, 3])])); ev.push(['sync', r.readyState]) }).then((x) => [x[0], [...new Uint8Array(x[1])], x[2], x[3]]))
await t('events empty blob', () => run((r, log) => { all(r, log); r.readAsText(new Blob([])) }))
await t('two listeners and on-handler fire', () => run((r, log) => { r.addEventListener('load', log('A')); r.addEventListener('load', log('B')); r.onload = log('on'); r.readAsText(new Blob(['x'])) }))
await t('same listener once', () => run((r, log) => { const f = log('A'); r.addEventListener('load', f); r.addEventListener('load', f); r.readAsText(new Blob(['x'])) }))
await t('removeEventListener one', () => run((r, log) => { const a = log('A'), b = log('B'); r.addEventListener('load', a); r.addEventListener('load', b); r.removeEventListener('load', a); r.readAsText(new Blob(['x'])) }))
await t('removeEventListener other fn keeps on-handler', () => run((r, log) => { r.onload = log('on'); r.removeEventListener('load', () => {}); r.readAsText(new Blob(['x'])) }))
await t('on-handler null', () => run((r, log) => { r.onload = log('on'); r.onload = null; r.addEventListener('load', log('A')); r.readAsText(new Blob(['x'])) }))
await t('dispatchEvent', () => { const r = new FileReader(); const got = []; r.addEventListener('foo', (e) => got.push(e.type)); return [typeof r.dispatchEvent, r instanceof EventTarget, got] })
await t('readAsText windows-1252', () => run((r) => r.readAsText(new Blob([new Uint8Array([0xe9, 0x80])]), 'windows-1252')))
await t('readAsText shift_jis', () => run((r) => r.readAsText(new Blob([new Uint8Array([0x82, 0xa0])]), 'Shift_JIS')))
await t('readAsText utf-16le', () => run((r) => r.readAsText(new Blob([new Uint8Array([0x41, 0x00])]), 'utf-16le')))
await t('readAsText bad label', () => run((r) => r.readAsText(new Blob(['é']), 'nope')))
await t('readAsText bom wins', () => run((r) => r.readAsText(new Blob([new Uint8Array([0xff, 0xfe, 0x41, 0x00])]), 'windows-1252')))
await t('readAsText utf8 bom', () => run((r) => r.readAsText(new Blob([new Uint8Array([0xef, 0xbb, 0xbf, 0x41])]))))
await t('readAsText charset from type', () => run((r) => r.readAsText(new Blob([new Uint8Array([0xe9])], { type: 'text/plain;charset=windows-1252' }))))
await t('readAsText invalid utf8', () => run((r) => r.readAsText(new Blob([new Uint8Array([0xff, 0x41])]))))
await t('read while loading', () => run((r, log, ev) => { r.readAsText(new Blob(['x'])); try { r.readAsText(new Blob(['y'])) } catch (e) { ev.push(['throws', e.name, e.code]) } }))
await t('read again after done', () => run(async (r, log, ev) => { r.readAsText(new Blob(['x'])); await sleep(10); all(r, log); r.readAsText(new Blob(['yz'])); ev.push(['sync', r.readyState, r.result]) }))
await t('abort while loading', () => run((r, log, ev) => { all(r, log); r.readAsText(new Blob(['x'])); r.abort(); ev.push(['after abort', r.readyState, r.result]) }))
await t('abort when empty', () => run((r, log) => { all(r, log); r.abort() }))
await t('abort when done', () => run(async (r, log, ev) => { r.readAsText(new Blob(['x'])); await sleep(10); all(r, log); r.abort(); ev.push(['after', r.readyState, r.result]) }))
await t('abort then read again', () => run((r, log) => { r.readAsText(new Blob(['x'])); r.abort(); all(r, log); r.readAsText(new Blob(['y'])) }))
await t('read non-blob', () => run((r, log, ev) => { try { r.readAsText('abc') } catch (e) { ev.push(['throws', e.name]) } }))
await t('listener this and order onload last', () => run((r, log, ev) => { r.onloadend = function () { ev.push(['onloadend this', this === r]) }; r.addEventListener('loadend', function () { ev.push(['L this', this === r]) }); r.readAsText(new Blob(['x'])) }))
  fs.writeFileSync('out.txt', rows.join('\n'))
})
"##,
    );
    let r = turbo_test(&p.dir, &["t.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
    let want = r##"["constants",[0,1,2,1]]
["initial",[0,null,null]]
["events readAsText",[2,"héllo",null,[["sync",1,null],["L","loadstart",1,true,0,0,false],["L","progress",1,true,6,6,true],["L","load",2,true,0,0,false],["L","loadend",2,true,0,0,false]]]]
["events arrayBuffer",[2,[1,2,3],null,[["sync",1],["L","loadstart",1,true,0,0,false],["L","progress",1,true,3,3,true],["L","load",2,true,0,0,false],["L","loadend",2,true,0,0,false]]]]
["events empty blob",[2,"",null,[["L","loadstart",1,true,0,0,false],["L","load",2,true,0,0,false],["L","loadend",2,true,0,0,false]]]]
["two listeners and on-handler fire",[2,"x",null,[["A","load",2,true,0,0,false],["B","load",2,true,0,0,false],["on","load",2,true,0,0,false]]]]
["same listener once",[2,"x",null,[["A","load",2,true,0,0,false]]]]
["removeEventListener one",[2,"x",null,[["B","load",2,true,0,0,false]]]]
["removeEventListener other fn keeps on-handler",[2,"x",null,[["on","load",2,true,0,0,false]]]]
["on-handler null",[2,"x",null,[["A","load",2,true,0,0,false]]]]
["dispatchEvent",["function",true,[]]]
["readAsText windows-1252",[2,"é€",null,[]]]
["readAsText shift_jis",[2,"あ",null,[]]]
["readAsText utf-16le",[2,"A",null,[]]]
["readAsText bad label",[2,"é",null,[]]]
["readAsText bom wins",[2,"A",null,[]]]
["readAsText utf8 bom",[2,"A",null,[]]]
["readAsText charset from type",[2,"�",null,[]]]
["readAsText invalid utf8",[2,"�A",null,[]]]
["read while loading",[2,"x",null,[["throws","InvalidStateError",11]]]]
["read again after done",[2,"yz",null,[["sync",1,null],["L","loadstart",1,true,0,0,false],["L","progress",1,true,2,2,true],["L","load",2,true,0,0,false],["L","loadend",2,true,0,0,false]]]]
["abort while loading",[2,null,null,[["L","abort",2,true,0,0,false],["L","loadend",2,true,0,0,false],["after abort",2,null]]]]
["abort when empty",[0,null,null,[]]]
["abort when done",[2,null,null,[["after",2,null]]]]
["abort then read again",[2,"y",null,[["L","loadstart",1,true,0,0,false],["L","progress",1,true,1,1,true],["L","load",2,true,0,0,false],["L","loadend",2,true,0,0,false]]]]
["read non-blob",[0,null,null,[["throws","TypeError"]]]]
["listener this and order onload last",[2,"x",null,[["onloadend this",true],["L this",true]]]]"##;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
}

#[test]
#[ignore = "turbo-test#138: fails until the fix"]
fn expect_message_prefixes_the_failure_like_vitest() {
    // turbo-test#138: `expect(value, message)` prefixes `message: ` to the failure of every chai
    // and jest-style matcher (also through .not, .resolves / .rejects and expect.extend
    // matchers), but not to a TypeError thrown before the assertion or to the .resolves
    // rejection; '' and undefined add nothing, a number is stringified. The default reporter
    // shows the prefixed message. Expected: vitest 4.1.12 (each row is [case, [name, message]]).
    let p = Project::new("rc-expect-message");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "t.test.js",
        r##"import { test, expect, vi } from 'vitest'
import * as fs from 'node:fs'
test('msg', async () => {
const rows = []
const t = async (name, f) => { let r; try { await f(); r = 'passed' } catch (e) { r = [e.name, e.message] } rows.push(JSON.stringify([name, r])) }
const spy = vi.fn()
const ext = { toBeFoo(received) { return { pass: received === 'foo', message: () => `expected ${received} to be foo` } } }
expect.extend(ext)
await t('toBe', () => expect(1, 'GOT<x>').toBe(2))
await t('not.toBe', () => expect(1, 'M').not.toBe(1))
await t('toEqual', () => expect({ a: 1 }, 'M').toEqual({ a: 2 }))
await t('toStrictEqual', () => expect([1], 'M').toStrictEqual([2]))
await t('toBeTruthy', () => expect(0, 'M').toBeTruthy())
await t('toBeNull', () => expect(1, 'M').toBeNull())
await t('toBeDefined', () => expect(undefined, 'M').toBeDefined())
await t('toBeGreaterThan', () => expect(1, 'M').toBeGreaterThan(2))
await t('toBeCloseTo', () => expect(1, 'M').toBeCloseTo(2))
await t('toContain', () => expect([1], 'M').toContain(2))
await t('toContainEqual', () => expect([{ a: 1 }], 'M').toContainEqual({ a: 2 }))
await t('toHaveLength', () => expect([1], 'M').toHaveLength(2))
await t('toHaveProperty', () => expect({}, 'M').toHaveProperty('a'))
await t('toMatch', () => expect('abc', 'M').toMatch(/x/))
await t('toMatchObject', () => expect({ a: 1 }, 'M').toMatchObject({ a: 2 }))
await t('toBeInstanceOf', () => expect(1, 'M').toBeInstanceOf(Map))
await t('toBeTypeOf', () => expect(1, 'M').toBeTypeOf('string'))
await t('toThrow no throw', () => expect(() => {}, 'M').toThrow())
await t('toThrow wrong', () => expect(() => { throw new Error('a') }, 'M').toThrow('b'))
await t('toHaveBeenCalled', () => expect(spy, 'M').toHaveBeenCalled())
await t('toHaveBeenCalledWith', () => expect(spy, 'M').toHaveBeenCalledWith(1))
await t('toHaveBeenCalledTimes', () => expect(spy, 'M').toHaveBeenCalledTimes(2))
await t('toHaveReturned', () => expect(spy, 'M').toHaveReturned())
await t('toSatisfy', () => expect(1, 'M').toSatisfy((x) => x > 2))
await t('custom matcher', () => expect('bar', 'M').toBeFoo())
await t('not spy', () => expect(1, 'M').toHaveBeenCalled())
await t('resolves', () => expect(Promise.resolve(1), 'M').resolves.toBe(2))
await t('rejects', () => expect(Promise.reject(new Error('e')), 'M').rejects.toThrow('x'))
await t('resolves rejected', () => expect(Promise.reject(new Error('e')), 'M').resolves.toBe(1))
await t('empty message', () => expect(1, '').toBe(2))
await t('number message', () => expect(1, 5).toBe(2))
await t('undefined message', () => expect(1, undefined).toBe(2))
await t('passes', () => expect(1, 'M').toBe(1))
await t('soft', () => { expect.soft(1, 'S').toBe(1); return 'ok' })
fs.writeFileSync('out.txt', rows.join('\n'))
})
test('reported', () => { expect(1, 'GOT<x>').toBe(2) })
"##,
    );
    let r = turbo_test(&p.dir, &["t.test.js"], &[("NO_COLOR", "1")]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.all()
            .contains("AssertionError: GOT<x>: expected 1 to be 2 // Object.is equality"),
        "{}",
        r.all()
    );
    let got = std::fs::read_to_string(p.path("out.txt")).unwrap();
    let want = r##"["toBe",["AssertionError","GOT<x>: expected 1 to be 2 // Object.is equality"]]
["not.toBe",["AssertionError","M: expected 1 not to be 1 // Object.is equality"]]
["toEqual",["AssertionError","M: expected { a: 1 } to deeply equal { a: 2 }"]]
["toStrictEqual",["AssertionError","M: expected [ 1 ] to strictly equal [ 2 ]"]]
["toBeTruthy",["AssertionError","M: expected +0 to be truthy"]]
["toBeNull",["AssertionError","M: expected 1 to be null"]]
["toBeDefined",["AssertionError","M: expected undefined to be defined"]]
["toBeGreaterThan",["AssertionError","M: expected 1 to be greater than 2"]]
["toBeCloseTo",["AssertionError","M: expected 1 to be close to 2, received difference is 1, but expected 0.005"]]
["toContain",["AssertionError","M: expected [ 1 ] to include 2"]]
["toContainEqual",["AssertionError","M: expected [ { a: 1 } ] to deep equally contain { a: 2 }"]]
["toHaveLength",["AssertionError","M: expected [ 1 ] to have a length of 2 but got 1"]]
["toHaveProperty",["AssertionError","M: expected {} to have property \"a\""]]
["toMatch",["AssertionError","M: expected 'abc' to match /x/"]]
["toMatchObject",["AssertionError","M: expected { a: 1 } to match object { a: 2 }"]]
["toBeInstanceOf",["AssertionError","M: expected 1 to be an instance of Map"]]
["toBeTypeOf",["AssertionError","M: expected 1 to be type of 'string'"]]
["toThrow no throw",["AssertionError","M: expected [Function] to throw an error"]]
["toThrow wrong",["AssertionError","M: expected [Function] to throw error including 'b' but got 'a'"]]
["toHaveBeenCalled",["AssertionError","M: expected \"vi.fn()\" to be called at least once"]]
["toHaveBeenCalledWith",["AssertionError","M: expected \"vi.fn()\" to be called with arguments: [ 1 ]\n\nNumber of calls: 0\n"]]
["toHaveBeenCalledTimes",["AssertionError","M: expected \"vi.fn()\" to be called 2 times, but got 0 times"]]
["toHaveReturned",["AssertionError","M: expected \"vi.fn()\" to be successfully called at least once"]]
["toSatisfy",["Error","M: expect(received).toSatisfy()\n\nExpected value to satisfy:\n[Function anonymous]\n\nReceived:\n1"]]
["custom matcher",["Error","M: expected bar to be foo"]]
["not spy",["TypeError","1 is not a spy or a call to a spy!"]]
["resolves",["AssertionError","M: expected 1 to be 2 // Object.is equality"]]
["rejects",["AssertionError","M: expected [Function] to throw error including 'x' but got 'e'"]]
["resolves rejected",["AssertionError","promise rejected \"Error: e\" instead of resolving"]]
["empty message",["AssertionError","expected 1 to be 2 // Object.is equality"]]
["number message",["AssertionError","5: expected 1 to be 2 // Object.is equality"]]
["undefined message",["AssertionError","expected 1 to be 2 // Object.is equality"]]
["passes","passed"]
["soft","passed"]"##;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count());
}
