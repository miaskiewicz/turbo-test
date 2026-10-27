//! The `vi` surface against vitest 4.1.12 and the jest-compat `jest` object against jest 30.5.2
//! (turbo-test#179, #180, #181), as `runner_c_expect_surface.rs` does for `expect`: the members
//! of `vi`, of what `vi.fn()` / `vi.spyOn()` return (and their `mock`) and of `jest`, then a
//! behaviour row for every member that was missing or behaved differently. The data in
//! `tests/data/vi_surface` was recorded with vitest 4.1.12 (`*.vitest.json`) and jest 30.5.2
//! (`*.jest.json`, the same `jest_*.test.js` files with `const fs = require('fs')` for the
//! `node:fs` import) on Node 26.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};
use serde_json::Value;
use std::collections::BTreeSet;

/// `jest` members jest 30.5.2 has and turbo-test does not, each tracked by its issue: the
/// automock switches (turbo-test#184).
const JEST_KNOWN_MISSING: [&str; 5] = [
    "autoMockOff:function",
    "autoMockOn:function",
    "disableAutomock:function",
    "enableAutomock:function",
    "onGenerateMock:function",
];

fn set(v: &Value) -> BTreeSet<String> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

fn read(p: &Project, name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p.path(name)).unwrap()).unwrap()
}

/// Each recorded row against turbo-test's, by name (a row turbo-test lacks shows as null).
fn rows_diff(want: &Value, got: &Value) -> Vec<String> {
    let got: Vec<&Value> = got.as_array().unwrap().iter().collect();
    let mut out = Vec::new();
    for w in want.as_array().unwrap() {
        let g = got.iter().find(|g| g[0] == w[0]);
        if g != Some(&w) {
            out.push(format!("want {w}\n got {}", g.map_or("null".into(), |g| g.to_string())));
        }
    }
    if got.len() != want.as_array().unwrap().len() {
        out.push(format!("{} rows, vitest has {}", got.len(), want.as_array().unwrap().len()));
    }
    out
}

#[test]
fn vi_and_mock_members_are_vitests() {
    // turbo-test#179: vi lacked getRealSystemTime (declared in the types), setTimerTickMode,
    // defineHelper, mockObject and resetConfig; a mock lacked getMockImplementation, mockThrow,
    // mockThrowOnce, withImplementation and mock.contexts, its mock.lastCall was a data property
    // and its own `__impl` / `__name` internals showed.
    let p = Project::new("rc-vi-members");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "members.test.js",
        include_str!("data/vi_surface/members.test.js"),
    );
    let r = turbo_test(&p.dir, &["members.test.js"], &[("OUT", "tt.json")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = read(&p, "tt.json");
    let want: Value =
        serde_json::from_str(include_str!("data/vi_surface/members.vitest.json")).unwrap();
    let mut diff = Vec::new();
    for k in ["vi", "fn", "fnMock", "spy", "getSpy"] {
        let (w, g) = (set(&want[k]), set(&got[k]));
        for m in w.difference(&g) {
            diff.push(format!("missing {k} {m}"));
        }
        for m in g.difference(&w) {
            diff.push(format!("extra {k} {m}"));
        }
    }
    assert_eq!(diff, Vec::<String>::new());
    assert_eq!(got["mocked"], want["mocked"]);
    assert_eq!(got["mockedSame"], want["mockedSame"]);
}

#[test]
fn vi_and_mock_members_behave_as_vitests() {
    // One row per member that was missing or differed (turbo-test#179): the new vi members
    // (setTimerTickMode's modes and errors, defineHelper's `this` / promise / name, mockObject
    // automock and in-place spy), setConfig / resetConfig through fakeTimers, the vi functions
    // returning `vi` (stubGlobal / stubEnv / unstubAll* / *AllMocks), stubGlobal's descriptor and
    // its restore, stubEnv's string / boolean / delete, doMock / doUnmock / hoisted argument
    // errors, getMockedSystemTime of a Date-only mock, vi.mocked returning its argument
    // untouched; and the mock function: name `Mock` / the method's, length, getMockImplementation,
    // mockThrow(Once), withImplementation (sync, async, once queue), mock.contexts, the lastCall
    // getter, mockReset back to vi.fn(impl)'s implementation, mockRestore, mockReturnValue
    // (undefined), spyOn's 'get' / 'set' accessors and its errors.
    let p = Project::new("rc-vi-surface");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "surface.test.js",
        include_str!("data/vi_surface/surface.test.js"),
    );
    let r = turbo_test(&p.dir, &["surface.test.js"], &[("OUT", "tt.json")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let want: Value =
        serde_json::from_str(include_str!("data/vi_surface/surface.vitest.json")).unwrap();
    assert_eq!(
        rows_diff(&want, &read(&p, "tt.json")),
        Vec::<String>::new()
    );
}

#[test]
fn config_fake_timers_are_use_fake_timers_defaults() {
    // turbo-test#180: `test.fakeTimers` (now, toFake, loopLimit) is what vi.useFakeTimers() uses,
    // an argument is merged over it, vi.setConfig({ fakeTimers }) replaces it (sinon's loopLimit
    // 1000 then) and vi.resetConfig() brings it back. turbo-test ignored the config: Date.now()
    // was the real time and the loop limit 10000.
    let p = Project::new("rc-vi-config-fake-timers");
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file(
        "vitest.config.js",
        include_str!("data/vi_surface/config/vitest.config.js"),
    );
    p.file(
        "config.test.js",
        include_str!("data/vi_surface/config/config.test.js"),
    );
    // without a config: vitest's default loopLimit (10000)
    p.file(
        "sub/vitest.config.js",
        "export default { test: { environment: 'node' } }\n",
    );
    p.file(
        "sub/default.test.js",
        r#"import { test, vi, expect } from 'vitest'
test('vitest default', () => {
  vi.useFakeTimers()
  setInterval(() => {}, 1)
  expect(() => vi.runAllTimers()).toThrow('Aborting after running 10000 timers, assuming an infinite loop!')
  vi.useRealTimers()
})
"#,
    );
    let r = turbo_test(&p.dir, &["config.test.js"], &[("OUT", "tt.json")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let want: Value =
        serde_json::from_str(include_str!("data/vi_surface/config.vitest.json")).unwrap();
    assert_eq!(
        rows_diff(&want, &read(&p, "tt.json")),
        Vec::<String>::new()
    );
    let sub = p.path("sub");
    let r = turbo_test(&sub, &["default.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

fn jest_project(name: &str) -> Project {
    let p = Project::new(name);
    p.file("package.json", r#"{ "type": "module" }"#);
    p.file("jest.config.js", "export default {}\n");
    p
}

#[test]
fn jest_members_are_jests() {
    // turbo-test#179 / #181: the jest object lacked advanceTimersToNextFrame,
    // advanceTimersToNextTimerAsync, runAllTicks, runOnlyPendingTimersAsync, setTimerTickMode,
    // runAllImmediates, getRealSystemTime's siblings and createMockFromModule, deepUnmock,
    // getSeed, isEnvironmentTornDown, setMock, unstable_mockModule, unstable_unmockModule.
    let p = jest_project("rc-jest-members");
    p.file(
        "members.test.js",
        include_str!("data/vi_surface/jest_members.test.js"),
    );
    let r = turbo_test(&p.dir, &["members.test.js"], &[("OUT", "tt.json")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let got = read(&p, "tt.json");
    let want: Value =
        serde_json::from_str(include_str!("data/vi_surface/members.jest.json")).unwrap();
    // turbo-test's jest object sits on vi: Object.prototype's names are its realm's, not jest's
    let (w, g) = (set(&want["jest"]), set(&got["jest"]));
    let missing: Vec<&String> = w.difference(&g).collect();
    let extra: Vec<&String> = g
        .difference(&w)
        .filter(|m| !m.starts_with("__") && !m.starts_with("constructor:"))
        .filter(|m| {
            ![
                "hasOwnProperty:",
                "isPrototypeOf:",
                "propertyIsEnumerable:",
                "toLocaleString:",
                "toString:",
                "valueOf:",
            ]
            .iter()
            .any(|o| m.starts_with(o))
        })
        .collect();
    assert_eq!(missing, JEST_KNOWN_MISSING.iter().collect::<Vec<_>>());
    assert_eq!(extra, Vec::<&String>::new());
}

#[test]
fn jest_fake_timers_are_jests() {
    // turbo-test#181: jest.useFakeTimers reads jest's options — `now`, `doNotFake`, `timerLimit`
    // (default 100000), `advanceTimers` (true / ms) and `legacyFakeTimers` (Date real,
    // runAllImmediates) — and ignores vitest's toFake / loopLimit; without fake timers every
    // advance function (getTimerCount, setSystemTime, setTimerTickMode too) console.warns jest's
    // message and does nothing; the timer functions return undefined; setTimerTickMode takes
    // `{ mode, delta }`. Each row is [name, value | throws, result, the warnings' first part].
    let p = jest_project("rc-jest-timers");
    p.file(
        "timers.test.js",
        include_str!("data/vi_surface/jest_timers.test.js"),
    );
    let r = turbo_test(&p.dir, &["timers.test.js"], &[("OUT", "tt.json")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let want: Value =
        serde_json::from_str(include_str!("data/vi_surface/timers.jest.json")).unwrap();
    assert_eq!(
        rows_diff(&want, &read(&p, "tt.json")),
        Vec::<String>::new()
    );
}

#[test]
fn jest_module_members_are_jests() {
    // turbo-test#179: createMockFromModule (the real module automocked), setMock (the module is
    // the given exports), deepUnmock / unstable_mockModule / unstable_unmockModule /
    // clearAllMocks & co. returning jest, getSeed (one 32-bit integer per run),
    // isEnvironmentTornDown.
    let p = jest_project("rc-jest-modules");
    p.file("helper.cjs", include_str!("data/vi_surface/helper.cjs"));
    p.file(
        "modules.test.js",
        include_str!("data/vi_surface/jest_modules.test.js"),
    );
    let r = turbo_test(&p.dir, &["modules.test.js"], &[("OUT", "tt.json")]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let want: Value =
        serde_json::from_str(include_str!("data/vi_surface/modules.jest.json")).unwrap();
    assert_eq!(
        rows_diff(&want, &read(&p, "tt.json")),
        Vec::<String>::new()
    );
}
