//! CommonJS decorator metadata cycles use partial exports, matching ts-jest (#351).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
use std::process::Command;

const FIXTURES: [(&str, &str); 6] = [
    (
        "metadata.ts",
        include_str!("data/decorator-cycle/metadata.ts"),
    ),
    ("role.ts", include_str!("data/decorator-cycle/role.ts")),
    (
        "person-role.ts",
        include_str!("data/decorator-cycle/person-role.ts"),
    ),
    (
        "partner-profile.ts",
        include_str!("data/decorator-cycle/partner-profile.ts"),
    ),
    (
        "customer.ts",
        include_str!("data/decorator-cycle/customer.ts"),
    ),
    (
        "cycle.test.ts",
        include_str!("data/decorator-cycle/cycle.test.ts"),
    ),
];

fn fixture() -> Project {
    let p = Project::new("decorator-cjs-cycle-351");
    p.with_esbuild().with_typescript();
    p.file("package.json", r#"{"name":"decorator-cjs-cycle-351"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("tsconfig.json", r#"{"compilerOptions":{"module":"CommonJS","target":"ES2021","experimentalDecorators":true,"emitDecoratorMetadata":true,"isolatedModules":true}}"#);
    for (name, source) in FIXTURES {
        p.file(name, source);
    }
    p
}

fn node_control(p: &Project) {
    let script = r#"
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const ts = require(process.argv[1]), root = process.argv[2], out = path.join(root, 'node-control');
fs.mkdirSync(out);
for (const name of fs.readdirSync(root).filter((name) => name.endsWith('.ts'))) {
  const result = ts.transpileModule(fs.readFileSync(path.join(root, name), 'utf8'), { fileName: name, compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021, experimentalDecorators: true,
    emitDecoratorMetadata: true, isolatedModules: true, esModuleInterop: true, useDefineForClassFields: false,
  }});
  fs.writeFileSync(path.join(out, name.replace(/\.ts$/, '.js')), result.outputText);
}
let count = 0;
global.test = (_name, fn) => { fn(); count++; };
require(path.join(out, 'cycle.test.js'));
assert.equal(count, 3);
"#;
    let output = Command::new("node")
        .arg("-e")
        .arg(script)
        .arg(p.path("node_modules/typescript"))
        .arg(&p.dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Node CommonJS control: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn decorator_cycles_match_typescript_commonjs_partial_exports() {
    let p = fixture();
    node_control(&p);
    let run = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "json", "cycle.test.ts"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

fn set_jest_mode(p: &Project, jest: bool) {
    let (keep, remove, config) = if jest {
        (
            "jest.config.js",
            "vitest.config.ts",
            "module.exports = { testEnvironment: 'node' };\n",
        )
    } else {
        (
            "vitest.config.ts",
            "jest.config.js",
            "export default { test: { environment: 'node' } };\n",
        )
    };
    if p.path(remove).exists() {
        std::fs::remove_file(p.path(remove)).unwrap();
    }
    p.file(keep, config);
}

fn check_mode(p: &Project, jest: bool) {
    set_jest_mode(p, jest);
    let run = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "json", "cycle.test.ts"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["success"], jest, "{}", run.all());
    if jest {
        assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    } else {
        assert!(
            report["testResults"]
                .to_string()
                .contains("before initialization"),
            "{}",
            run.all()
        );
    }
}

#[test]
fn warm_esm_metadata_cache_cannot_change_commonjs_cycle_semantics() {
    let p = fixture();
    check_mode(&p, false);
    check_mode(&p, true);
}

#[test]
fn warm_commonjs_metadata_cache_cannot_change_esm_cycle_semantics() {
    let p = fixture();
    check_mode(&p, true);
    check_mode(&p, false);
}

#[test]
fn decorated_dependency_hoists_its_own_named_and_default_mocks() {
    let p = fixture();
    p.file("dep.ts", "export const value = 'real';\n");
    p.file("default-dep.ts", "export default 'real-default';\n");
    p.file(
        "decorated.ts",
        r#"
import { value } from './dep';
import defaultValue from './default-dep';
function Tag<T extends Function>(constructor: T): T { return constructor; }
@Tag
export class Holder { named = value; defaultValue = defaultValue; }
jest.mock('./dep', () => ({ value: 'mocked' }));
jest.mock('./default-dep', () => ({ __esModule: true, default: 'mock-default' }));
export const captured = [value, defaultValue];
"#,
    );
    p.file(
        "mock.test.ts",
        r#"
import assert from 'node:assert/strict';
import { Holder, captured } from './decorated';
test('the decorated dependency registers its mocks before imported values are captured', () => {
  assert.deepEqual(captured, ['mocked', 'mock-default']);
  const holder = new Holder();
  assert.equal(holder.named, 'mocked');
  assert.equal(holder.defaultValue, 'mock-default');
});
"#,
    );
    let run = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "json", "mock.test.ts"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
