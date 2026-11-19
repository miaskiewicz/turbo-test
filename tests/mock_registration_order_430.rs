//! Every hoisted mock of a file is registered before any of its factories runs (turbo-test#430).
//! jest runs `jest.mock` factories lazily, on the module's first require; vitest registers every
//! hoisted `vi.mock` before it imports anything. So a factory that loads the real module
//! (`jest.requireActual`, `vi.importActual`, `importOriginal`) gets the file's other mocks in that
//! module's graph whatever order the calls are written in. Here `reader` imports `key`; the test
//! mocks `reader` over its actual exports and `key` outright, in both orders. The jest cases were
//! checked against Jest 30 (ts-jest), the vitest ones against vitest 4.
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

/// `key` -> `reader` (imports key) -> `loader` (imports reader), under `src/`.
fn sources(p: &Project) {
    p.file(
        "src/util/key.ts",
        "export function getKey(c: string): string | undefined { return c === 'MX' ? 'mexico' : undefined; }\n",
    );
    p.file(
        "src/reader/reader.ts",
        "import { getKey } from '../util/key';\n\
         export function field(data: Record<string, unknown>): unknown { const k = getKey('MX'); return k ? data[k] : undefined; }\n\
         export function other(): string { return 'real-other'; }\n",
    );
    p.file(
        "src/svc/loader.ts",
        "import { field } from '../reader/reader';\n\
         export function load(data: Record<string, unknown>) { return field(data) ?? ''; }\n",
    );
}

/// The tests every spec runs: the loader reads `key` through the actual reader, so it sees the
/// test's mock of `key`, and the reader mock's own override stays.
const CHECKS: &str = "\
beforeEach(() => { mockedGetKey.mockReset(); mockedGetKey.mockReturnValue('mexico'); });
it('the actual reader reads the mocked key', () => { expect(load({ mexico: 'v' })).toBe('v'); expect(other()).toBe('mocked-other'); });
it('the mocked key decides', () => { mockedGetKey.mockReturnValue(undefined); expect(load({ mexico: 'v' })).toBe(''); });
";

fn jest_spec(reader_mock: &str, key_mock: &str, first_reader: bool) -> String {
    let (a, b) = if first_reader {
        (reader_mock, key_mock)
    } else {
        (key_mock, reader_mock)
    };
    format!(
        "import {{ getKey }} from '@/util/key';\n\
         import {{ load }} from './loader';\n\
         import {{ other }} from '@/reader/reader';\n\
         {a}\n{b}\n\
         const mockedGetKey = jest.mocked(getKey);\n{CHECKS}"
    )
}

#[test]
fn jest_mocks_are_all_registered_before_a_factory_loads_the_actual_module() {
    let p = Project::new("jest-mock-order-430");
    p.with_esbuild();
    sources(&p);
    p.file("package.json", r#"{ "name": "jestorder" }"#);
    p.file(
        "tsconfig.json",
        r#"{ "compilerOptions": { "module": "commonjs", "esModuleInterop": true, "baseUrl": ".", "paths": { "@/*": ["src/*"] } } }"#,
    );
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', moduleNameMapper: { '^@/(.*)$': '<rootDir>/src/$1' } };\n",
    );
    // aliased specifiers, both orders
    let reader = "jest.mock('@/reader/reader', () => ({ ...jest.requireActual('@/reader/reader'), other: jest.fn(() => 'mocked-other') }));";
    let key = "jest.mock('@/util/key', () => ({ getKey: jest.fn() }));";
    p.file(
        "src/svc/alias_reader_first.spec.ts",
        &jest_spec(reader, key, true),
    );
    p.file(
        "src/svc/alias_key_first.spec.ts",
        &jest_spec(reader, key, false),
    );
    // relative specifiers
    let reader = "jest.mock('../reader/reader', () => ({ ...jest.requireActual('../reader/reader'), other: jest.fn(() => 'mocked-other') }));";
    let key = "jest.mock('../util/key', () => ({ getKey: jest.fn() }));";
    p.file(
        "src/svc/rel_reader_first.spec.ts",
        &jest_spec(reader, key, true),
    );
    // CommonJS requires
    p.file(
        "src/svc/cjs_reader_first.spec.js",
        &format!(
            "jest.mock('../reader/reader', () => ({{ ...jest.requireActual('../reader/reader'), other: jest.fn(() => 'mocked-other') }}));\n\
             jest.mock('../util/key', () => ({{ getKey: jest.fn() }}));\n\
             const {{ getKey }} = require('../util/key');\n\
             const {{ load }} = require('./loader');\n\
             const {{ other }} = require('../reader/reader');\n\
             const mockedGetKey = getKey;\n{CHECKS}"
        ),
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (8, 0), "{}", r.all());
}

fn vitest_spec(reader_mock: &str, first_reader: bool) -> String {
    let key = "vi.mock('../util/key', () => ({ getKey: vi.fn() }));";
    let (a, b) = if first_reader {
        (reader_mock, key)
    } else {
        (key, reader_mock)
    };
    format!(
        "import {{ vi, it, expect, beforeEach }} from 'vitest';\n\
         import {{ getKey }} from '../util/key';\n\
         import {{ load }} from './loader';\n\
         import {{ other }} from '../reader/reader';\n\
         {a}\n{b}\n\
         const mockedGetKey = vi.mocked(getKey);\n{CHECKS}"
    )
}

#[test]
fn vitest_mocks_are_all_registered_before_a_factory_imports_the_original() {
    let p = Project::new("vi-mock-order-430");
    p.with_esbuild();
    sources(&p);
    p.file("package.json", r#"{ "name": "viorder", "type": "module" }"#);
    let original = "vi.mock('../reader/reader', async (importOriginal) => ({ ...(await importOriginal<object>()), other: vi.fn(() => 'mocked-other') }));";
    let actual = "vi.mock('../reader/reader', async () => ({ ...(await vi.importActual<object>('../reader/reader')), other: vi.fn(() => 'mocked-other') }));";
    p.file(
        "src/svc/original_first.test.ts",
        &vitest_spec(original, true),
    );
    p.file(
        "src/svc/original_last.test.ts",
        &vitest_spec(original, false),
    );
    p.file("src/svc/actual_first.test.ts", &vitest_spec(actual, true));
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    assert_eq!(counts(&r), (6, 0), "{}", r.all());
}
