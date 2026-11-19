//! `jest.mock(spec, factory, { virtual: true })`: the factory's exports are registered against
//! the raw spec (plus the caller's dir), so `require(spec)` returns them without the spec ever
//! being resolved against the filesystem (turbo-test#308). Jest-only; vi.mock keeps its usual
//! resolve-then-register semantics.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn counts(r: &common::Run) -> (u64, u64) {
    let j = json_report(r);
    (
        j["numPassedTests"].as_u64().unwrap(),
        j["numFailedTests"].as_u64().unwrap(),
    )
}

#[test]
fn jest_mock_virtual_registers_against_raw_spec() {
    // The spec `virtual-only-pkg` isn't a real file / package, but the virtual mock makes
    // `require('virtual-only-pkg')` return the factory's exports.
    let p = Project::new("jest-virtual-308-basic");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestvirt" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "virt.test.js",
        "jest.mock('virtual-only-pkg', () => ({ default: 42, named: 'hi' }), { virtual: true });\n\
         const m = require('virtual-only-pkg');\n\
         test('default + named reach us', () => {\n\
             expect(m.default).toBe(42);\n\
             expect(m.named).toBe('hi');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn jest_do_mock_virtual_registers_eagerly() {
    // jest.doMock's runtime form takes the same options object; the spec never needs to exist.
    let p = Project::new("jest-virtual-308-domock");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestvirtdm" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "virt.test.js",
        "test('doMock virtual', () => {\n\
             jest.doMock('nope', () => ({ v: 7 }), { virtual: true });\n\
             const m = require('nope');\n\
             expect(m.v).toBe(7);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn unresolvable_spec_without_virtual_still_errors() {
    // Regression guard: dropping the `{ virtual: true }` must NOT silently swallow the missing
    // module — jest.mock of an unresolvable spec without the flag must still error.
    let p = Project::new("jest-virtual-308-regression");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestvirtneg" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "virt.test.js",
        "jest.mock('definitely-not-real-pkg', () => ({ x: 1 }));\n\
         test('errors', () => {\n\
             let threw = false;\n\
             try { require('definitely-not-real-pkg'); } catch (e) { threw = true; }\n\
             expect(threw).toBe(true);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn virtual_mock_keys_on_the_path_resolved_from_the_test_dir() {
    // Jest keys a relative / absolute virtual mock by `path.resolve(testDir, spec)` and a bare one
    // by the raw spec, so any module reaching that path (absolute, relative from elsewhere, a
    // dynamic import) gets the mock; the extension is literal, never resolved (turbo-test#405,
    // the seed runner's `await import(path.join(__dirname, 'seeds', file))`). Every expectation
    // here was checked against Jest 30 (ts-jest).
    let p = Project::new("jest-virtual-405-abs");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestvirtabs" }"#);
    p.file(
        "tsconfig.json",
        r#"{ "compilerOptions": { "module": "commonjs", "esModuleInterop": false } }"#,
    );
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "runner.ts",
        "import * as path from 'path';\n\
         export const reqAbs = (f: string) => require(path.join(__dirname, 'seeds', f));\n\
         export const impAbs = async (f: string) => (await import(path.join(__dirname, 'seeds', f))) as any;\n\
         export const reqRel = (f: string) => require('./seeds/' + f);\n",
    );
    p.file(
        "sub/other.ts",
        "export const reqUp = (f: string) => require('../seeds/' + f);\n\
         export const reqBare = () => require('virtual-pkg');\n",
    );
    p.file(
        "v.test.ts",
        r#"import * as path from 'path';
import { reqAbs, impAbs, reqRel } from './runner';
import { reqUp, reqBare } from './sub/other';

jest.mock('./seeds/a.ts', () => ({ default: { name: 'A' } }), { virtual: true });
jest.mock('./seeds/b', () => ({ default: { name: 'B' } }), { virtual: true });
jest.mock('virtual-pkg', () => ({ v: 'P' }), { virtual: true });

const name = (f: () => any) => {
  try {
    return f().default.name;
  } catch (e) {
    return 'ERR';
  }
};

test('relative, absolute and bare virtual mocks', () => {
  expect(name(() => reqAbs('a.ts'))).toBe('A');
  expect(name(() => reqAbs('b'))).toBe('B');
  expect(name(() => reqRel('a.ts'))).toBe('A');
  expect(name(() => reqUp('a.ts'))).toBe('A');
  expect(name(() => reqUp('b'))).toBe('B');
  expect(name(() => require(path.join(__dirname, 'seeds', 'a.ts')))).toBe('A');
  expect(name(() => require('./seeds/../seeds/a.ts'))).toBe('A');
  expect(reqBare().v).toBe('P');
});

test('the extension is literal', () => {
  expect(name(() => reqAbs('a'))).toBe('ERR');
  expect(name(() => reqAbs('b.ts'))).toBe('ERR');
});

test('a dynamic import of the absolute path gets the mock', async () => {
  expect((await impAbs('a.ts')).default.name).toBe('A');
  expect((await impAbs('b')).default.name).toBe('B');
  await expect(impAbs('b.ts')).rejects.toThrow();
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (3, 0), "{}", r.all());
}
