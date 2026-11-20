//! A TypeScript class whose fields have no initializer and carry legacy decorators +
//! emitDecoratorMetadata (as NestJS DTOs do, recovery.spec): turbo-test's oxc decorator path
//! currently drops the fields. ts-jest 30 keeps them as own undefined properties when the
//! project's tsconfig produces `useDefineForClassFields: true` semantics (turbo-test#438).
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

fn project() -> Project {
    let p = Project::new("jest-class-fields-438");
    p.with_esbuild();
    p.file(
        "package.json",
        r#"{"name":"jest-class-fields-438","version":"0.0.0"}"#,
    );
    p.file(
        "jest.config.cjs",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file(
        "tsconfig.json",
        include_str!("data/jest-class-fields-438/tsconfig.json"),
    );
    p.file(
        "fields.test.ts",
        include_str!("data/jest-class-fields-438/fields.test.ts"),
    );
    p
}

#[test]
fn decorated_class_fields_without_initializer_are_own_undefined_properties() {
    let p = project();
    let r = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&r);
    assert_eq!(report["numPassedTests"], 1, "{}", r.all());
    assert_eq!(report["numFailedTests"], 0, "{}", r.all());
}
