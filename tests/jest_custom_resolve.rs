//! turbo-test#345: jest `moduleDirectories` and `customExportConditions` must be honoured.
//! `moduleDirectories` tells the resolver where to look for bare modules (besides `node_modules`).
//! `customExportConditions` adds extra export conditions when resolving package.json `exports`.
#![cfg(unix)]

mod common;
use common::{json_report, turbo_test, Project};

/// `moduleDirectories: ["node_modules", "src"]` lets `require('utils/math')` find
/// `<root>/src/utils/math.js` as a bare module (not a relative `./src/utils/math`).
#[test]
fn module_directories_resolves_from_custom_dir() {
    let p = Project::new("jest-module-dirs");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jest-module-dirs" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', moduleDirectories: ['node_modules', 'src'] };\n",
    );
    // A utility in src/utils/math.js reachable as bare `utils/math`.
    p.file(
        "src/utils/math.js",
        "module.exports = { add: (a, b) => a + b };\n",
    );
    p.file(
        "a.test.js",
        r#"
const { test, expect } = require('@jest/globals');
test('bare import from custom moduleDirectory', () => {
    const math = require('utils/math');
    expect(math.add(1, 2)).toBe(3);
});
"#,
    );

    let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
    let j = json_report(&r);
    assert_eq!(
        (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
        (Some(1), Some(0)),
        "moduleDirectories should allow resolving bare imports from 'src':\n{}",
        r.all()
    );
}

/// `customExportConditions: ["myapp"]` adds the `myapp` condition when resolving package.json
/// `exports`, so a package that exports `"myapp": "./myapp.js"` picks that entry.
#[test]
fn custom_export_conditions_selects_correct_entry() {
    let p = Project::new("jest-custom-conditions");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jest-custom-conditions" }"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node', customExportConditions: ['myapp'] };\n",
    );
    // A package whose exports map has a `myapp` condition.
    p.file(
        "node_modules/cond-pkg/package.json",
        r#"{"name":"cond-pkg","exports":{".":{"myapp":"./myapp.js","default":"./default.js"}}}"#,
    );
    p.file(
        "node_modules/cond-pkg/myapp.js",
        "module.exports = 'FROM_MYAPP';\n",
    );
    p.file(
        "node_modules/cond-pkg/default.js",
        "module.exports = 'FROM_DEFAULT';\n",
    );
    p.file(
        "a.test.js",
        r#"
const { test, expect } = require('@jest/globals');
test('custom export condition selects myapp entry', () => {
    const val = require('cond-pkg');
    expect(val).toBe('FROM_MYAPP');
});
"#,
    );

    let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
    let j = json_report(&r);
    assert_eq!(
        (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
        (Some(1), Some(0)),
        "customExportConditions should add 'myapp' to the resolver:\n{}",
        r.all()
    );
}

/// Both options together: custom dirs + custom conditions in one config.
#[test]
fn module_directories_and_custom_conditions_together() {
    let p = Project::new("jest-both-resolve-opts");
    p.with_esbuild();
    p.file("package.json", r#"{ "name": "jest-both-resolve-opts" }"#);
    p.file(
        "jest.config.js",
        "module.exports = {\n\
         \x20 testEnvironment: 'node',\n\
         \x20 moduleDirectories: ['node_modules', 'lib'],\n\
         \x20 customExportConditions: ['custom-env'],\n\
         };\n",
    );
    // A local module in lib/.
    p.file(
        "lib/helpers/greet.js",
        "module.exports = name => `Hi ${name}`;\n",
    );
    // A package with a custom-env condition.
    p.file(
        "node_modules/env-pkg/package.json",
        r#"{"name":"env-pkg","exports":{".":{"custom-env":"./custom.js","default":"./fallback.js"}}}"#,
    );
    p.file(
        "node_modules/env-pkg/custom.js",
        "module.exports = 'CUSTOM';\n",
    );
    p.file(
        "node_modules/env-pkg/fallback.js",
        "module.exports = 'FALLBACK';\n",
    );
    p.file(
        "a.test.js",
        r#"
const { test, expect } = require('@jest/globals');
test('both custom resolve options work together', () => {
    const greet = require('helpers/greet');
    expect(greet('world')).toBe('Hi world');
    const val = require('env-pkg');
    expect(val).toBe('CUSTOM');
});
"#,
    );

    let r = turbo_test(&p.dir, &["--reporter", "json", "a.test.js"], &[]);
    let j = json_report(&r);
    assert_eq!(
        (j["numPassedTests"].as_u64(), j["numFailedTests"].as_u64()),
        (Some(1), Some(0)),
        "both moduleDirectories and customExportConditions should work:\n{}",
        r.all()
    );
}
