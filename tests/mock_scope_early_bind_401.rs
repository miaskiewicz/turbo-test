//! A test file's top-level bindings are visible to its retained `jest.mock` factory closures as
//! soon as they are initialised, not only once the whole file has run (#401).
//!
//! A decorator factory returned by a `jest.mock` factory runs while the decorated dependency
//! loads, i.e. in the middle of the test file. Real Jest 30 (ts-jest 29.4) passes every file
//! below: the `const` / `let` declared before the import/require are initialised, a function
//! declaration is hoisted, and a `const` declared after the require is still in its temporal
//! dead zone (`Cannot access 'mockLate' before initialization`).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const TX: &str = "export const Transactional = (_opts?: unknown) => (_t: object, _k: string, d: PropertyDescriptor) => d;\n";

const SERVICE: &str = r#"import { Transactional } from './tx';
export class Service {
  @Transactional({ name: 'save' })
  save() { return 'saved'; }
}
"#;

const IMPORTED: &str = r#"const mockTransactionalOptions: Array<{
  key: string;
  options: unknown;
}> = [];
jest.mock('./tx', () => ({
  Transactional: (options: unknown) => (_t: object, key: string, d: PropertyDescriptor) => {
    mockTransactionalOptions.push({ key, options });
    return d;
  },
}));
import { Service } from './service';
test('the decorator factory records its options while the service loads', () => {
  expect(new Service().save()).toBe('saved');
  expect(mockTransactionalOptions).toEqual([{ key: 'save', options: { name: 'save' } }]);
});
"#;

const REQUIRED: &str = r#"const mockTransactionalMethods = new Set<string>();
jest.mock('./tx', () => ({
  Transactional: () => (_t: object, key: string, d: PropertyDescriptor) => {
    mockTransactionalMethods.add(key);
    return d;
  },
}));
const { Service } = require('./service');
test('the decorator factory records its method while the service loads', () => {
  expect(new Service().save()).toBe('saved');
  expect([...mockTransactionalMethods]).toEqual(['save']);
});
"#;

const HOISTED: &str = r#"'use strict';
jest.mock('./tx', () => ({
  Transactional: () => (_t: object, key: string, d: PropertyDescriptor) => {
    mockRecord(key);
    mockLoads.count += 1;
    return d;
  },
  current: () => mockLabel,
}));
const mockLoads: { count: number } = { count: 0 };
let mockLabel: string = 'initial';
const { Service } = require('./service');
const tx = require('./tx');
function mockRecord(key: string) {
  (globalThis as { mockSeen?: string }).mockSeen = key;
}
test('a hoisted function and a later reassigned let are read live through the factory', () => {
  expect(new Service().save()).toBe('saved');
  expect((globalThis as { mockSeen?: string }).mockSeen).toBe('save');
  expect(mockLoads.count).toBe(1);
  expect(tx.current()).toBe('initial');
  mockLabel = 'changed';
  expect(tx.current()).toBe('changed');
});
"#;

const EARLY: &str = r#"jest.mock('./tx', () => ({
  Transactional: () => (_t: object, key: string, d: PropertyDescriptor) => {
    mockLate.add(key);
    return d;
  },
}));
let error: unknown;
try { require('./service'); } catch (e) { error = e; }
const mockLate = new Set<string>();
test('a binding read before its declaration is still in its temporal dead zone', () => {
  expect(error).toBeInstanceOf(ReferenceError);
  expect((error as Error).message).toBe("Cannot access 'mockLate' before initialization");
  expect(mockLate.size).toBe(0);
});
"#;

fn fixture() -> Project {
    let p = Project::new("mock-scope-early-bind-401");
    p.with_esbuild().with_typescript();
    p.file("package.json", r#"{"name":"mock-scope-early-bind-401"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("tsconfig.json", r#"{"compilerOptions":{"module":"CommonJS","target":"ES2021","experimentalDecorators":true,"emitDecoratorMetadata":true,"isolatedModules":true,"esModuleInterop":true}}"#);
    p.file("tx.ts", TX);
    p.file("service.ts", SERVICE);
    p
}

fn passes(p: &Project, file: &str) {
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json", file], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{file}: {}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{file}: {}", run.all());
    assert_eq!(report["success"], true, "{file}: {}", run.all());
}

#[test]
fn a_decorator_factory_reads_a_const_initialised_before_the_static_import() {
    let p = fixture();
    p.file("imported.test.ts", IMPORTED);
    passes(&p, "imported.test.ts");
}

#[test]
fn a_decorator_factory_reads_a_const_initialised_before_the_require() {
    let p = fixture();
    p.file("required.test.ts", REQUIRED);
    passes(&p, "required.test.ts");
}

#[test]
fn hoisted_functions_and_live_lets_after_a_directive() {
    let p = fixture();
    p.file("hoisted.test.ts", HOISTED);
    passes(&p, "hoisted.test.ts");
}

#[test]
fn a_binding_declared_after_the_require_stays_in_its_temporal_dead_zone() {
    let p = fixture();
    p.file("early.test.ts", EARLY);
    passes(&p, "early.test.ts");
}
