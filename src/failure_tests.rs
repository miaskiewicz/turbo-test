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
         ❯ a.test.ts:5:35\n ❯ helper /elsewhere/h.ts:2:9\n\nCaused by: TypeError: inner\n ❯ a.test.ts:6:1"
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
#[ignore = "turbo-test#169: fails until the fix"]
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
