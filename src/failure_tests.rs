use super::*;

fn frame(method: &str, file: &str, line: u32, col: u32) -> Frame {
    Frame {
        method: method.to_string(),
        file: PathBuf::from(file),
        line,
        col,
    }
}

#[test]
fn stacks_keep_the_frames_in_files() {
    let stack = "AssertionError: x\n\
                 \x20   at __chaiAssert (<anonymous>:3355:9)\n\
                 \x20   at /p/a.test.ts:5:35\n\
                 \x20   at Object.helper (/p/src/h.ts:2:9)\n\
                 \x20   at async run (file:///p/x y.ts:10:1)\n\
                 \x20   at new Promise (<anonymous>)\n\
                 \x20   at native\n\
                 \x20   at /p/bad.ts:x:1\n\
                 not a frame /p/z.ts:1:1";
    assert_eq!(
        parse_stack(stack),
        vec![
            frame("", "/p/a.test.ts", 5, 35),
            frame("helper", "/p/src/h.ts", 2, 9),
            frame("run", "/p/x y.ts", 10, 1),
        ]
    );
    assert_eq!(parse_stack(""), vec![]);
}

#[test]
fn stacks_start_after_the_last_define_helper_frame() {
    // turbo-test#182: vitest's parseStacktrace drops the frames up to the last
    // `__VITEST_HELPER__` (vi.defineHelper's wrapper); a file path naming it is not a method
    let stack = "Error: x\n\
                 \x20   at /p/h.test.ts:3:13\n\
                 \x20   at __VITEST_HELPER__ (<anonymous>:10:1)\n\
                 \x20   at /p/h.test.ts:5:1\n\
                 \x20   at async __VITEST_HELPER__ (<anonymous>:12:1)\n\
                 \x20   at /p/h.test.ts:10:3\n\
                 \x20   at run (/p/__VITEST_HELPER__/r.ts:1:2)";
    assert_eq!(
        parse_stack(stack),
        vec![
            frame("", "/p/h.test.ts", 10, 3),
            frame("run", "/p/__VITEST_HELPER__/r.ts", 1, 2),
        ]
    );
}

#[test]
fn errors_print_as_vitest_print_error() {
    let root = Path::new("/p");
    let e = ErrorInfo {
        name: "AssertionError".into(),
        message: "expected 1 to be 2".into(),
        diff: "- Expected\n+ Received\n\n- 2\n+ 1".into(),
        frames: vec![
            frame("", "/p/a.test.ts", 5, 35),
            frame("dep", "/p/node_modules/d/i.js", 1, 1),
            frame("helper", "/elsewhere/h.ts", 2, 9),
        ],
        cause: Some(Box::new(ErrorInfo {
            name: "TypeError".into(),
            message: "inner".into(),
            frames: vec![frame("", "/p/a.test.ts", 6, 1)],
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(
        print_error(&e, root),
        "AssertionError: expected 1 to be 2\n\n- Expected\n+ Received\n\n- 2\n+ 1\n\n \
         ❯ a.test.ts:5:35\n ❯ dep node_modules/d/i.js:1:1\n ❯ helper /elsewhere/h.ts:2:9\n\nCaused by: TypeError: inner\n ❯ a.test.ts:6:1"
    );
    // no diff, no frames
    let bare = ErrorInfo {
        name: "Error".into(),
        message: "load boom".into(),
        ..Default::default()
    };
    assert_eq!(print_error(&bare, root), "Error: load boom");
}

#[test]
fn anonymous_async_frames_keep_their_location() {
    // turbo-test#169: V8 writes an anonymous function suspended at an await as
    // `at async <location>` (no parentheses); its location was read as `async /p/…` and the
    // frame dropped, so an error made after an await had no location in the test file
    let stack = "Error: Matcher did not succeed in time.\n\
                 \x20   at run (<anonymous>:4303:86)\n\
                 \x20   at async /p/pfail.test.js:3:9\n\
                 \x20   at async file:///p/x y.test.js:7:12\n\
                 \x20   at async <anonymous>:1:1\n\
                 \x20   at async Promise.all (index 0)\n\
                 \x20   at async fn (/p/h.ts:2:5)";
    assert_eq!(
        parse_stack(stack),
        vec![
            frame("", "/p/pfail.test.js", 3, 9),
            frame("", "/p/x y.test.js", 7, 12),
            frame("fn", "/p/h.ts", 2, 5),
        ]
    );
}

#[test]
fn eval_frames_point_at_the_eval_origin() {
    // turbo-test#175: a frame in code from `eval` / `new Function` is
    // `at fn (eval at <caller> (<origin>), <anonymous>:L:C)`; vitest's parseSingleV8Stack drops
    // the `(eval at …` and the `, <anonymous>:L:C` parts, so the frame is the eval's origin
    // (a nested eval's location keeps a `)` and is dropped, as vitest drops it)
    let stack = "Error: late\n\
                 \x20   at eval (eval at <anonymous> (/p/afail.test.js:2:14), <anonymous>:3:41)\n\
                 \x20   at f (eval at <anonymous> (file:///p/e.test.js:3:3), <anonymous>:2:9)\n\
                 \x20   at async eval (eval at run (/p/r.js:7:5), <anonymous>:1:1)\n\
                 \x20   at g (eval at h (eval at <anonymous> (/p/n.js:4:2)), <anonymous>:1:1)\n\
                 \x20   at eval code (eval code:1:1)\n\
                 \x20   at eval (<anonymous>:1:1)\n\
                 \x20   at /p/afail.test.js:4:3";
    assert_eq!(
        parse_stack(stack),
        vec![
            frame("eval", "/p/afail.test.js", 2, 14),
            frame("f", "/p/e.test.js", 3, 3),
            frame("eval", "/p/r.js", 7, 5),
            frame("", "/p/afail.test.js", 4, 3),
        ]
    );
}

#[test]
fn stacks_drop_only_vitests_ignored_frames() {
    // turbo-test#173: vitest's parseStacktrace leaves out the frames its `stackIgnorePatterns`
    // match (its own packages, chai, tinyspy, node internals) and keeps a dependency's
    let at = |file: &str| format!("    at f ({file}:1:1)");
    let ignored = [
        "/p/node_modules/chai/index.js",
        "/p/node_modules/tinyspy/dist/index.js",
        "/p/node_modules/@vitest/runner/dist/index.js",
        "/p/node_modules/vitest/dist/chunks/x.js",
        "/p/vitest/src/x.js",
        "/p/packages/runner/dist/index.js",
        "/p/node_modules/vite/dist/node/module-runner.js",
        "/p/node_modules/rolldown-vite/dist/node/module-runner.js",
        "/p/node_modules/.vite/deps/chunk-AB.js",
        "/p/node_modules/.vite/deps/@vitest_x.js",
        "/p/node_modules/.vite/deps/loupe.js",
        "/p/node_modules/.vite/deps/chai.js",
        "/p/node_modules/.vite/deps/vitest_x.js",
        "/p/node_modules/@vitest/browser-playwright/dist/locators.js",
        "/p/node_modules/@vitest/browser-webdriverio/dist/locators.js",
        "/p/node_modules/@vitest/browser-preview/dist/locators.js",
        "/p/x/node:internal/x.js",
        "/p/node:fs/x.js",
        "/p/__vitest_test__/x.js",
        "/p/__vitest_browser__/x.js",
        "/p/@id/__x00__vitest/browser.js",
    ];
    let kept = [
        "/p/node_modules/k6dep/index.js",
        "/p/node_modules/chai-x/index.js",
        "/p/packages/dist/index.js",
        "/p/packages/a-b/dist/index.js",
        "/p/node_modules/@vitest/x/lib/index.js",
        "/p/node:/x.js",
        "/p/a.test.js",
    ];
    let stack: Vec<String> = ignored.iter().chain(&kept).map(|f| at(f)).collect();
    assert_eq!(
        parse_stack(&stack.join("\n")),
        kept.iter().map(|f| frame("f", f, 1, 1)).collect::<Vec<_>>()
    );
}
