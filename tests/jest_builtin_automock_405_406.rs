//! Mocks of node builtins (turbo-test#405) and automocked class statics (turbo-test#406).
//!
//! #405: `jest.mock('fs')` (automock or factory) must replace the builtin for the test file and
//! the app modules it loads, under both spellings (`fs` / `node:fs`, one shared mock), while
//! `jest.requireActual('fs')`, other test files and node_modules packages keep the real one.
//! #406: an automocked class's static methods, inherited ones included (`static async create`
//! on an abstract base), are mock functions on the mock class, as in Jest's moduleMocker.
//! Every expectation here was checked against real Jest 30 (ts-jest for the .ts files) or, for
//! the external-package case, real vitest.
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

fn jest_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestbuiltinmock" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p
}

#[test]
fn explicit_fs_automock_replaces_namespace_and_named_imports() {
    let p = jest_project("jest-fs-automock-405");
    p.file(
        "reader.ts",
        "import { readFileSync } from 'node:fs';\n\
         export function readText(p: string): string {\n\
           return readFileSync(p, 'utf8') as string;\n\
         }\n",
    );
    p.file(
        "a_mock.test.ts",
        r#"import * as fs from 'fs';
import { existsSync, readFileSync } from 'fs';
import { readText } from './reader';

jest.mock('fs');

const mockedFs = fs as jest.Mocked<typeof fs>;

test('the namespace and named imports are the automock', () => {
  mockedFs.readFileSync.mockReturnValue('mocked');
  expect(readText('a.txt')).toBe('mocked');
  expect(readFileSync).toHaveBeenCalledWith('a.txt', 'utf8');
  expect(jest.isMockFunction(readFileSync)).toBe(true);
  expect(jest.mocked(fs).readFileSync).toBe(readFileSync);
});

test('mock state is shared with the node: alias', () => {
  (existsSync as jest.Mock).mockReturnValue(true);
  expect(require('node:fs').existsSync('nope')).toBe(true);
  expect(require('node:fs')).toBe(require('fs'));
  expect(jest.requireActual('fs').existsSync('/definitely/not/here')).toBe(false);
});
"#,
    );
    p.file(
        "b_real.test.js",
        "// No mock in this file: another file's jest.mock('fs') must not leak into it.\n\
         test('fs is the real builtin', () => {\n\
           const fs = require('fs');\n\
           expect(jest.isMockFunction(fs.readFileSync)).toBe(false);\n\
           expect(fs.existsSync(__filename)).toBe(true);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (3, 0), "{}", r.all());
}

#[test]
fn builtin_factory_mock_serves_both_spellings_until_unmocked() {
    let p = jest_project("jest-fs-factory-405");
    p.file(
        "factory.test.js",
        r#"jest.mock('node:fs', () => ({ readFileSync: jest.fn(() => 'from-factory') }));

test('a builtin factory mock serves both spellings', () => {
  expect(require('fs').readFileSync('x')).toBe('from-factory');
  expect(require('node:fs')).toBe(require('fs'));
  expect(require('fs').existsSync).toBe(undefined);
});

test('dontMock + resetModules restores the real builtin', () => {
  jest.dontMock('node:fs');
  jest.resetModules();
  expect(jest.isMockFunction(require('fs').readFileSync)).toBe(false);
  expect(typeof require('fs').existsSync).toBe('function');
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (2, 0), "{}", r.all());
}

#[test]
fn throwing_builtin_factory_fails_the_require() {
    let p = jest_project("jest-fs-factory-throws-405");
    p.file(
        "throws.test.js",
        "jest.mock('fs', () => {\n\
           throw new Error('fs factory boom');\n\
         });\n\
         test('a throwing builtin factory fails the require', () => {\n\
           expect(() => require('fs')).toThrow('fs factory boom');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn vitest_builtin_mock_skips_external_packages() {
    // vitest: app modules see vi.mock('node:fs'); a node_modules package loads through Node
    // and keeps the real builtin (the same rule as other mocked modules, turbo-test#135).
    let p = Project::new("vi-fs-external-405");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "vifsext", "type": "module" }"#);
    p.file(
        "node_modules/fsuser/package.json",
        r#"{ "name": "fsuser", "version": "1.0.0", "main": "index.js" }"#,
    );
    p.file(
        "node_modules/fsuser/index.js",
        "module.exports = () => require('fs').readFileSync;\n",
    );
    p.file(
        "app.js",
        "import { readFileSync } from 'fs';\nexport function readIt() { return readFileSync('anything'); }\n",
    );
    p.file(
        "ext.test.js",
        r#"import { vi, test, expect } from 'vitest';
import * as fs from 'node:fs';
import fsuser from 'fsuser';
import { readIt } from './app.js';

vi.mock('node:fs', () => ({ readFileSync: vi.fn(() => 'mocked'), existsSync: vi.fn(() => true), default: { readFileSync: vi.fn(() => 'mocked-default') } }));

test('app modules see the builtin mock, external packages the real one', () => {
  expect(fs.readFileSync('x')).toBe('mocked');
  expect(readIt()).toBe('mocked');
  expect(vi.isMockFunction(fsuser())).toBe(false);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}

#[test]
fn automocked_class_keeps_own_and_inherited_statics_as_mocks() {
    let p = jest_project("jest-class-statics-406");
    p.file(
        "engine.ts",
        r#"abstract class Root {
  static describeCountry(): string {
    return 'root';
  }
}

export abstract class BaseEngine extends Root {
  protected static readonly COUNTRY: string;
  public static async create<T extends BaseEngine>(this: new () => T): Promise<T> {
    return new this();
  }
  baseHelper(): number {
    return 1;
  }
}

export class TaxEngine extends BaseEngine {
  static readonly COUNTRY = 'MX';
  static previousPeriod(d: Date): string {
    return 'real-' + d.getTime();
  }
  compute(): number {
    return 2;
  }
}
"#,
    );
    p.file(
        "statics.test.ts",
        r#"import { TaxEngine } from './engine';

jest.mock('./engine');

const MockEngine = TaxEngine as jest.Mocked<typeof TaxEngine>;

beforeEach(() => {
  MockEngine.previousPeriod.mockReturnValue('P1');
  MockEngine.create.mockResolvedValue({ rate: 7 } as never);
});

test('own and inherited statics of an automocked class are mocks', async () => {
  expect(TaxEngine.previousPeriod(new Date(0))).toBe('P1');
  await expect(TaxEngine.create()).resolves.toEqual({ rate: 7 });
  expect(Object.prototype.hasOwnProperty.call(TaxEngine, 'create')).toBe(true);
  expect(MockEngine.create).toHaveBeenCalledTimes(1);
  expect(jest.isMockFunction(MockEngine.describeCountry)).toBe(true);
  expect(MockEngine.COUNTRY).toBe('MX');
  expect(jest.isMockFunction(new TaxEngine().compute)).toBe(true);
  expect(jest.isMockFunction(new TaxEngine().baseHelper)).toBe(true);
});
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (1, 0), "{}", r.all());
}
