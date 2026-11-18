//! Pairwise profile selection and initialization contracts; not an exhaustive Cartesian product.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

const CAPABILITIES: &str = r#"
const assert=require('node:assert/strict');
function assertProfile(dom) {
 assert.equal(globalThis.__ttRuntimeBackendInstalled,!dom);
 assert.equal(typeof document,dom?'object':'undefined');
 for(const key of ['DOMPoint','DOMMatrix','FileReader','MutationObserver']) assert.equal(typeof globalThis[key],dom?'function':'undefined',key);
 for(const key of ['__ttAesGcm','__ttHttp','__ttTcp','__ttSpawnSync','__ttAsyncProcess','__ttAsyncFrameGet','__ttAsyncFrameSet','__ttVmRun','__ttRsa']) assert.equal(typeof globalThis[key],dom?'undefined':'function',key);
 assert.equal(process.env.SSR,dom?'':'1');
}
"#;
struct Case {
    name: &'static str,
    configs: &'static [(&'static str, &'static str)],
    cli: &'static [&'static str],
    pragma: &'static str,
    dom: bool,
}
const V_NODE: &str = "export default {test:{environment:'node'}};";
const V_DOM: &str = "export default {test:{environment:'jsdom'}};";
const V_HAPPY: &str = "export default {test:{environment:'happy-dom'}};";
const J_NODE: &str = "module.exports={testEnvironment:'node'};";
const J_DOM: &str = "module.exports={testEnvironment:'jsdom'};";

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "vitest-node",
            configs: &[("vitest.config.js", V_NODE)],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "vitest-jsdom",
            configs: &[("vitest.config.js", V_DOM)],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "vitest-happy-dom",
            configs: &[("vitest.config.js", V_HAPPY)],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "jest-node",
            configs: &[("jest.config.js", J_NODE)],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "jest-jsdom",
            configs: &[("jest.config.js", J_DOM)],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "jest-node-alias",
            configs: &[(
                "jest.config.cjs",
                "module.exports={testEnvironment:'jest-environment-node'};",
            )],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "jest-jsdom-alias",
            configs: &[(
                "jest.config.cjs",
                "module.exports={testEnvironment:'jest-environment-jsdom'};",
            )],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "jest-happy-alias",
            configs: &[(
                "jest.config.js",
                "module.exports={testEnvironment:'jest-environment-happy-dom'};",
            )],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "jest-default",
            configs: &[("jest.config.js", "module.exports={};")],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "vitest-default-heuristic",
            configs: &[("vitest.config.js", "export default {test:{}};")],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "package-jest-node",
            configs: &[(
                "package.json",
                r#"{"name":"matrix","jest":{"testEnvironment":"node"}}"#,
            )],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "package-jest-jsdom",
            configs: &[(
                "package.json",
                r#"{"name":"matrix","jest":{"testEnvironment":"jsdom"}}"#,
            )],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "package-jest-default",
            configs: &[("package.json", r#"{"name":"matrix","jest":{}}"#)],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "cli-node-over-dom",
            configs: &[("vitest.config.js", V_DOM)],
            cli: &["--environment", "node"],
            pragma: "",
            dom: false,
        },
        Case {
            name: "cli-dom-over-node",
            configs: &[("jest.config.js", J_NODE)],
            cli: &["--environment", "jsdom"],
            pragma: "",
            dom: true,
        },
        Case {
            name: "pragma-node-over-cli-and-config",
            configs: &[("vitest.config.js", V_HAPPY)],
            cli: &["--environment", "jsdom"],
            pragma: "node",
            dom: false,
        },
        Case {
            name: "pragma-happy-over-cli-and-config",
            configs: &[("jest.config.js", J_DOM)],
            cli: &["--environment", "node"],
            pragma: "happy-dom",
            dom: true,
        },
        Case {
            name: "nested-jest-node",
            configs: &[
                ("vitest.config.js", V_DOM),
                ("nested/jest.config.js", J_NODE),
            ],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "nested-jest-dom",
            configs: &[
                ("vitest.config.js", V_NODE),
                ("nested/jest.config.js", J_DOM),
            ],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "nested-vitest-node",
            configs: &[
                ("jest.config.js", J_DOM),
                ("nested/vitest.config.js", V_NODE),
            ],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "nested-vitest-happy",
            configs: &[
                ("jest.config.js", J_NODE),
                ("nested/vitest.config.js", V_HAPPY),
            ],
            cli: &[],
            pragma: "",
            dom: true,
        },
        Case {
            name: "nested-package-jest-default",
            configs: &[
                ("vitest.config.js", V_DOM),
                ("nested/package.json", r#"{"name":"child","jest":{}}"#),
            ],
            cli: &[],
            pragma: "",
            dom: false,
        },
        Case {
            name: "forced-config-over-nearest",
            configs: &[
                ("selected.config.js", V_DOM),
                ("nested/jest.config.js", J_NODE),
            ],
            cli: &["--config", "selected.config.js"],
            pragma: "",
            dom: true,
        },
        Case {
            name: "pragma-over-forced-config",
            configs: &[
                ("selected.config.js", V_DOM),
                ("nested/jest.config.js", J_DOM),
            ],
            cli: &["--config", "selected.config.js"],
            pragma: "node",
            dom: false,
        },
    ]
}
fn assert_passed(p: &Project, args: &[&str], count: u64) {
    let run = turbo_test(&p.dir, args, &[]);
    let report = json_report(&run);
    assert_eq!(report["numTotalTests"], count, "{}", run.all());
    assert_eq!(report["numPassedTests"], count, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
#[test]
fn profile_selection_matrix_covers_config_cli_pragma_and_nearest_framework_owners() {
    for case in cases() {
        let p = Project::new(case.name);
        p.with_esbuild();
        p.file("package.json", r#"{"name":"runtime-environment-matrix"}"#);
        for (path, source) in case.configs {
            p.file(path, source);
        }
        let source = format!(
            "/* @vitest-environment {} */\n{}\ntest('selected profile',()=>assertProfile({}));",
            case.pragma, CAPABILITIES, case.dom
        );
        // Empty pragma must not itself select an environment.
        let source = source.replace("/* @vitest-environment  */\n", "");
        p.file("nested/check.test.cjs", &source);
        let mut args = vec!["--jobs", "1", "--reporter", "json", "nested/check.test.cjs"];
        args.extend_from_slice(case.cli);
        assert_passed(&p, &args, 1);
    }
}

fn settings_project(jest: bool, reuse: bool) -> Project {
    let p = Project::new("profile-settings-pairs");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"profile-settings-pairs"}"#);
    let (path, config) = if jest {
        ("jest.config.js","module.exports={testEnvironment:'node',clearMocks:true,fakeTimers:{enableGlobally:true,now:1234567890},setupFiles:['./early.cjs'],setupFilesAfterEnv:['./late.cjs']};")
    } else {
        ("vitest.config.js","export default {test:{environment:'happy-dom',clearMocks:true,fakeTimers:{now:1234567890},setupFiles:['./early.cjs','./late.cjs']}};")
    };
    p.file(path, config);
    let dom = !jest;
    p.file("early.cjs",&format!("{CAPABILITIES}\nassertProfile({dom}); globalThis.__matrixEarlyExpect=typeof expect; globalThis.__matrixSetupProfile={dom};"));
    p.file(
        "late.cjs",
        &format!(
            "{CAPABILITIES}\nassertProfile({dom});globalThis.__matrixLateExpect=typeof expect;"
        ),
    );
    let source = format!(
        r#"
{CAPABILITIES}
const clock={clock};
const mock=clock.fn();
test('setup and configured fake timers run in the selected profile',()=>{{
 assertProfile({dom});
 assert.equal(globalThis.__matrixSetupProfile,{dom});
 assert.equal(globalThis.__matrixEarlyExpect,'{early}');
 assert.equal(globalThis.__matrixLateExpect,'function');
 clock.useFakeTimers();
 assert.equal(Date.now(),1234567890);
 setTimeout(mock,5);
 clock.advanceTimersByTime(5);
 assert.equal(mock.mock.calls.length,1);
 clock.useRealTimers();
}});
test('configured clearMocks and explicit reset preserve the selected profile',()=>{{
 assertProfile({dom});
 assert.equal(mock.mock.calls.length,0);
 mock.mockReturnValue('changed');
 assert.equal(mock(),'changed');
 clock.resetAllMocks();
 assert.equal(mock(),undefined);
}});
"#,
        clock = if jest { "jest" } else { "vi" },
        early = if jest { "undefined" } else { "function" }
    );
    p.file("a.test.cjs", &source);
    p.file("b.test.cjs", &source);
    let isolation = if reuse { "--no-isolate" } else { "--isolate" };
    assert_passed(&p, &["--jobs", "1", "--reporter", "json", isolation], 4);
    p
}
#[test]
fn profile_settings_pairs_cover_setup_timers_mock_cleanup_and_fresh_or_reused_workers() {
    for (jest, reuse) in [(true, false), (true, true), (false, false), (false, true)] {
        let _project = settings_project(jest, reuse);
    }
}
