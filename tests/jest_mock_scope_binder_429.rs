//! A test file's mock-prefixed bindings reach its retained `jest.mock` factories while the
//! file's own imports load (turbo-test#429, after #401). The spec opens with an import; a
//! `mock…` const follows, a factory closes over it, and a later import's dependency applies a
//! decorator from the mocked module at load time, so the factory's closure reads the const then.
//! ts-jest keeps the const before that import, so Jest has it initialised; the decorating import
//! placed before the const is a genuine temporal dead zone, in Jest as here. Every expectation was
//! checked against Jest 30 (ts-jest), the failing assertion's reported line included.
#![cfg(unix)]

mod common;
use common::{file_counts, file_status, json_report, turbo_test, Project};

const MOCK: &str = "const mockTransactionalMethods = new Set<string>();
jest.mock('./deco', () => ({
  Transactional: () => (_target: unknown, key: string) => {
    mockTransactionalMethods.add(key);
  },
}));
";

fn project() -> Project {
    let p = Project::new("jest-binder-429");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jestbinder" }"#);
    p.file(
        "tsconfig.json",
        r#"{ "compilerOptions": { "module": "commonjs", "target": "es2020", "experimentalDecorators": true, "esModuleInterop": true } }"#,
    );
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("src/helper.ts", "export const helper = (): number => 1;\n");
    p.file(
        "src/deco.ts",
        "export function Transactional(): MethodDecorator {\n  return () => undefined;\n}\n",
    );
    p.file(
        "src/service.ts",
        "import { Transactional } from './deco';\n\nexport class Service {\n  @Transactional()\n  run(): string {\n    return 'ran';\n  }\n}\n",
    );
    p
}

/// The file's entry in the JSON report.
fn entry<'a>(j: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    j["testResults"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"].as_str().unwrap().ends_with(name))
        .unwrap()
}

#[test]
fn a_mock_const_reaches_the_factory_while_a_later_import_loads() {
    let p = project();
    // the #429 shape: opens with an import; the decorating import comes after the mock const
    p.file(
        "src/opens_with_import.spec.ts",
        &format!(
            "import {{ helper }} from './helper';\n\
             // a mock-prefixed const the retained factory closes over\n\
             {MOCK}\n\
             import {{ Service }} from './service';\n\
             \n\
             test('the decorator ran with the mock while the service loaded', () => {{\n\
             \x20 expect(helper()).toBe(1);\n\
             \x20 expect(new Service().run()).toBe('ran');\n\
             \x20 expect([...mockTransactionalMethods]).toEqual(['run']);\n\
             }});\n\
             \n\
             test('a failing assertion reports its own line', () => {{\n\
             \x20 expect(helper()).toBe(2);\n\
             }});\n"
        ),
    );
    // a directive prologue stays first
    p.file(
        "src/directive.spec.ts",
        &format!(
            "'use strict';\n\
             import {{ helper }} from './helper';\n\
             {MOCK}\
             import {{ Service }} from './service';\n\
             \n\
             test('directive first', () => {{\n\
             \x20 expect(helper()).toBe(1);\n\
             \x20 expect(new Service().run()).toBe('ran');\n\
             \x20 expect([...mockTransactionalMethods]).toEqual(['run']);\n\
             }});\n"
        ),
    );
    // a genuine TDZ: the decorating import comes before the const, and the file's top level uses
    // it ahead of the mocks, so it loads before the const runs (in Jest as here)
    p.file(
        "src/genuine_tdz.spec.ts",
        &format!(
            "import {{ Service }} from './service';\n\
             const mockTransactionalMethods = new Set<string>();\n\
             const LoadedService = Service;\n\
             {}\n\
             test('never runs', () => {{\n\
             \x20 expect(new LoadedService().run()).toBe('ran');\n\
             }});\n",
            MOCK.split_once('\n').unwrap().1
        ),
    );
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let j = json_report(&r);

    assert_eq!(
        file_counts(entry(&j, "opens_with_import.spec.ts")),
        (1, 1),
        "{}",
        r.all()
    );
    assert_eq!(
        file_counts(entry(&j, "directive.spec.ts")),
        (1, 0),
        "{}",
        r.all()
    );

    let tdz = entry(&j, "genuine_tdz.spec.ts");
    assert_eq!(file_status(tdz), "error", "{}", r.all());
    assert!(
        tdz["message"]
            .as_str()
            .unwrap()
            .contains("Cannot access 'mockTransactionalMethods' before initialization"),
        "{}",
        r.all()
    );

    // the failing assertion is reported where it is, line 19 column 20, as Jest reports it: the
    // moved binder shifts no line or column of the code
    let r = turbo_test(
        &p.dir,
        &["src/opens_with_import.spec.ts"],
        &[("NO_COLOR", "1"), ("TURBO_LEGACY_REPORTER", "0")],
    );
    assert!(
        r.all().contains("opens_with_import.spec.ts:19:20"),
        "{}",
        r.all()
    );
}
