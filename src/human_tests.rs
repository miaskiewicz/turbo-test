use super::*;
use crate::runner::{SnapshotResult, SuiteError};

fn file(idx: usize, tests: Vec<TestCase>) -> FileResult {
    let failed_file = tests.iter().any(|t| t.status == "failed");
    FileResult {
        idx,
        line: String::new(),
        passed: 0,
        failed: 0,
        load_error: false,
        failed_file,
        error_msg: String::new(),
        tests,
        suites: Vec::new(),
        snapshot: None,
        setup_us: 0.0,
        dur_ms: 12.4,
        suite_errors: Vec::new(),
        load_error_info: None,
        logs: Vec::new(),
        waited_ms: 0.0,
    }
}

fn test(path: &str, status: &str, ms: f64) -> TestCase {
    let mut parts: Vec<String> = path.split(" > ").map(str::to_string).collect();
    let title = parts.pop().unwrap();
    TestCase {
        name: path.to_string(),
        title,
        ancestors: parts,
        status: status.to_string(),
        duration_ms: ms,
        message: if status == "failed" {
            "boom".into()
        } else {
            String::new()
        },
        ..Default::default()
    }
}

fn error(message: &str) -> ErrorInfo {
    ErrorInfo {
        name: "Error".into(),
        message: message.into(),
        ..Default::default()
    }
}

fn run<'a>(root: &'a Path, names: &'a [PathBuf]) -> Run<'a> {
    Run {
        root,
        names,
        wall_ms: 1500.0,
        start: "10:00:00".into(),
        test_files: "1 passed (1)".into(),
        tests: "1 passed (1)".into(),
        did_update: false,
    }
}

#[test]
fn format_time_is_vitests() {
    assert_eq!(format_time(0.0), "0ms");
    assert_eq!(format_time(2.5), "3ms");
    assert_eq!(format_time(1000.0), "1000ms");
    assert_eq!(format_time(1000.4), "1.00s");
    assert_eq!(format_time(12_345.0), "12.35s");
}

#[test]
fn dividers_fill_thirty_columns() {
    assert_eq!(
        divider(" Failed Tests 3 ", None),
        "⎯⎯⎯⎯⎯⎯⎯ Failed Tests 3 ⎯⎯⎯⎯⎯⎯⎯"
    );
    // an odd remainder goes to the right
    assert_eq!(
        divider("abc", None),
        format!("{}abc{}", "⎯".repeat(13), "⎯".repeat(14))
    );
    assert_eq!(
        divider("[1/3]", Some(1)),
        format!("{}[1/3]⎯", "⎯".repeat(24))
    );
    // wider than the columns: no fill on the left
    assert_eq!(
        divider(&"x".repeat(40), Some(1)),
        format!("{}⎯", "x".repeat(40))
    );
}

#[test]
fn code_frames_are_generate_code_frames() {
    let src = "one\n\ttwo\nthree\nfour\nfive\nsix\n";
    // two lines either side, the caret under the 1-based column, tabs as spaces
    assert_eq!(
        code_frame_of(src, 3, 2),
        "      1| one\n      2|  two\n      3| three\n       |  ^\n      4| four\n      5| five"
    );
    // the first line: nothing before it; a blank line is its number alone
    assert_eq!(
        code_frame_of("a\n\nb", 1, 1),
        "      1| a\n       | ^\n      2|\n      3| b"
    );
    // past the last line: the last one
    assert_eq!(
        code_frame_of("x\ny", 9, 0),
        "      1| x\n      2| y\n       |^"
    );
    // CRLF lines lose their \r
    assert_eq!(
        code_frame_of("a\r\nb", 2, 1),
        "      1| a\n      2| b\n       | ^"
    );
    // over 71 characters: 70 of them and `…`; trailing blanks trimmed
    let long = format!("{}{}", "x".repeat(70), "yz");
    assert_eq!(
        code_frame_of(&format!("{long}\nend   "), 2, 1),
        format!("      1| {}…\n      2| end\n       | ^", "x".repeat(70))
    );
    assert_eq!(
        code_frame_of(&"x".repeat(71), 1, 1),
        format!("      1| {}\n       | ^", "x".repeat(71))
    );
    // a source-map comment is left out; a line over 200 characters drops the whole frame
    assert_eq!(
        code_frame_of("a\n//# sourceMappingURL=x", 1, 1),
        "      1| a\n       | ^"
    );
    assert_eq!(code_frame_of(&format!("a\n{}", "x".repeat(201)), 1, 1), "");
    assert_ne!(code_frame_of(&format!("a\n{}", "x".repeat(200)), 1, 1), "");
}

#[test]
fn module_lines_count_like_get_module_log() {
    let one = file(0, vec![test("a", "passed", 1.0)]);
    assert_eq!(
        module_line(&one, "a.test.ts"),
        " ✓ a.test.ts (1 test) 12ms\n"
    );
    let mixed = file(
        0,
        vec![
            test("a", "passed", 1.0),
            test("b", "failed", 1.0),
            test("c", "skipped", 0.0),
            test("d", "todo", 0.0),
            test("e", "todo", 0.0),
        ],
    );
    assert_eq!(
        module_line(&mixed, "x.test.ts"),
        " ❯ x.test.ts (5 tests | 1 failed | 1 skipped | 2 todo) 12ms\n"
    );
    // nothing ran: ↓; timer waits count in the file's duration
    let mut skipped = file(0, vec![test("a", "skipped", 0.0), test("b", "todo", 0.0)]);
    skipped.waited_ms = 300.0;
    assert_eq!(
        module_line(&skipped, "s.test.ts"),
        " ↓ s.test.ts (2 tests | 1 skipped | 1 todo) 312ms\n"
    );
    // a file that failed to load has no duration
    let mut load = file(0, vec![]);
    load.load_error = true;
    load.failed_file = true;
    assert_eq!(
        module_line(&load, "l.test.ts"),
        " ❯ l.test.ts (0 test) \n".replace(" \n", "\n")
    );
}

#[test]
fn test_suffixes_and_symbols() {
    let mut t = test("a", "passed", 2.4);
    assert_eq!(suffix(&t), " 2ms");
    t.retry_count = 2;
    assert_eq!(suffix(&t), " 2ms (retry x2)");
    // a forbidden .only never ran: no duration
    let mut f = test("a", "failed", 0.0);
    f.not_run = true;
    assert_eq!(suffix(&f), "");
    assert_eq!(suffix(&test("a", "skipped", 5.0)), "");
    let sym = |s: &str| symbol(&test("a", s, 0.0));
    assert_eq!(
        [
            sym("passed"),
            sym("failed"),
            sym("todo"),
            sym("skipped"),
            sym("queued")
        ],
        ["✓", "×", "□", "↓", "·"]
    );
}

/// turbo-test#96: a test skipped at run time (it has a start) shows its duration and its note;
/// a statically skipped one neither, an empty note no brackets, a note only on a skipped test.
#[test]
fn skipped_at_run_time_suffixes() {
    let mut t = test("a", "skipped", 1.6);
    t.start_ms = 1000.0;
    assert_eq!(suffix(&t), " 2ms");
    t.note = "not today".into();
    assert_eq!(suffix(&t), " 2ms [not today]");
    let mut s = test("a", "skipped", 0.0);
    s.note = "n".into();
    assert_eq!(suffix(&s), " [n]");
    let mut p = test("a", "passed", 0.4);
    p.note = "n".into();
    assert_eq!(suffix(&p), " 0ms");
}

/// vitest's printAnnotations: grouped by location and type in first-seen order (one made at the
/// same place with the same type joins its group), a group without a location is the type alone.
#[test]
fn annotations_group_by_place_and_type() {
    let a = |kind: &str, message: &str, line: u32, col: u32| Annotation {
        kind: kind.into(),
        message: message.into(),
        line,
        col,
    };
    let notes = [
        a("notice", "one", 3, 9),
        a("warning", "two", 3, 9),
        a("notice", "three", 3, 9),
        a("notice", "four", 0, 0),
        a("notice", "five", 4, 1),
    ];
    assert_eq!(
        annotation_lines(&notes, "a.test.ts", 1),
        " ❯ a.test.ts:3:9 notice\n   ↳ one\n   ↳ three\n ❯ a.test.ts:3:9 warning\n   ↳ two\n \
         ❯ notice\n   ↳ four\n ❯ a.test.ts:4:1 notice\n   ↳ five\n"
    );
    assert_eq!(
        annotation_lines(&notes[..1], "b.ts", 3),
        "   ❯ b.ts:3:9 notice\n     ↳ one\n"
    );
    assert_eq!(annotation_lines(&[], "b.ts", 3), "");
}

#[test]
fn snapshot_lines_are_render_snapshot_summary() {
    let names = [PathBuf::from("a.test.ts"), PathBuf::from("src/b.test.ts")];
    let root = Path::new("/r");
    let mut a = file(0, vec![]);
    a.snapshot = Some(SnapshotResult {
        added: 1,
        ..Default::default()
    });
    // one line: no blank line after it
    assert_eq!(
        snapshot_lines(&[a.clone()], &run(root, &names)),
        ["  Snapshots  1 written"]
    );
    a.snapshot = Some(SnapshotResult {
        added: 1,
        unmatched: 2,
        updated: 3,
        unchecked: 1,
        unchecked_keys: vec!["k 1".into()],
        ..Default::default()
    });
    let mut b = file(1, vec![]);
    b.snapshot = Some(SnapshotResult {
        file_deleted: true,
        unchecked_keys: vec!["old 1".into()],
        ..Default::default()
    });
    let lines = |a: &FileResult, did_update| {
        let mut r = run(root, &names);
        r.did_update = did_update;
        snapshot_lines(&[a.clone(), b.clone()], &r)
    };
    assert_eq!(
        lines(&a, false),
        [
            "  Snapshots  1 written",
            "             2 failed",
            "             3 updated ",
            "             1 files obsolete ",
            "             1 obsolete",
            "             ↳ ./a.test.ts",
            "               · k 1",
            "             ↳ src/b.test.ts",
            "               · old 1",
            "",
        ]
    );
    assert_eq!(lines(&a, true)[3], "             1 files removed ");
    assert_eq!(lines(&a, true)[4], "             1 removed");
    // the keys are listed only under an obsolete / removed count
    a.snapshot.as_mut().unwrap().unchecked = 0;
    assert_eq!(lines(&a, false).len(), 5);
}

#[test]
fn print_error_writes_frames_code_frame_and_cause() {
    let dir = std::env::temp_dir().join(format!("tt-human-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.test.ts");
    std::fs::write(&src, "x\nthrow 1\n").unwrap();
    let frame = |file: &Path, method: &str| Frame {
        method: method.into(),
        file: file.to_path_buf(),
        line: 2,
        col: 1,
    };
    let e = ErrorInfo {
        name: "TypeError".into(),
        message: "bad".into(),
        diff: "- Expected\n+ Received".into(),
        frames: vec![
            frame(&dir.join("node_modules/dep/x.js"), "dep"),
            frame(&dir.join("gone.ts"), ""),
            frame(&src, "helper"),
            frame(&src, ""),
        ],
        cause: Some(Box::new(ErrorInfo {
            frames: vec![frame(&src, "")],
            ..error("inner")
        })),
        ..Default::default()
    };
    // node_modules frames are left out; the code frame follows the first frame of a file that
    // exists; the cause has no code frame
    assert_eq!(
        print_error(&e, &dir, true, false, ""),
        "TypeError: bad\n\n- Expected\n+ Received\n\n ❯ gone.ts:2:1\n ❯ helper a.test.ts:2:1\n      1| x\n      2| throw 1\n       | ^\n      3|\n ❯ a.test.ts:2:1\n\nCaused by: Error: inner\n ❯ a.test.ts:2:1\n\n"
    );
    // without frames: no blank line after the message
    assert_eq!(
        print_error(&error("m"), &dir, true, false, "P: "),
        "P: Error: m\n"
    );
    // turbo-test#97: the serialized form replaces `Name: message`; the properties follow the
    // frames under a divider when asked for, a cause's always
    let shown = ErrorInfo {
        inspected: Some("{ a: 1, stacks: [] }".into()),
        properties: Some("{ a: 1 }".into()),
        ..error("")
    };
    let line = "⎯".repeat(30);
    assert_eq!(
        print_error(&shown, &dir, true, true, "P: "),
        format!("{{ a: 1, stacks: [] }}\n{line}\nSerialized Error: {{ a: 1 }}\n")
    );
    assert_eq!(
        print_error(&shown, &dir, true, false, ""),
        "{ a: 1, stacks: [] }\n"
    );
    let with_cause = ErrorInfo {
        frames: vec![frame(&dir.join("gone.ts"), "")],
        properties: Some("{ code: 1 }".into()),
        cause: Some(Box::new(ErrorInfo {
            properties: Some("{ code: 'C' }".into()),
            ..error("inner")
        })),
        ..error("outer")
    };
    assert_eq!(
        print_error(&with_cause, &dir, false, false, ""),
        format!("Error: outer\n ❯ gone.ts:2:1\n\nCaused by: Error: inner\n{line}\nSerialized Error: {{ code: 'C' }}\n")
    );
    assert_eq!(
        print_error(&with_cause, &dir, false, true, ""),
        format!("Error: outer\n ❯ gone.ts:2:1\n\n{line}\nSerialized Error: {{ code: 1 }}\nCaused by: Error: inner\n{line}\nSerialized Error: {{ code: 'C' }}\n")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn errors_summary_counts_tasks_and_merges_identical_errors() {
    let names = [PathBuf::from("a.test.ts"), PathBuf::from("b.test.ts")];
    let root = Path::new("/r");
    let mut t1 = test("d > one", "failed", 1.0);
    t1.errors = vec![error("same"), error("second")];
    let mut t2 = test("two", "failed", 1.0);
    t2.errors = vec![error("same")];
    // a failed test without a recorded error reports its message
    let t3 = test("three", "failed", 1.0);
    let mut a = file(0, vec![t1, t2, t3, test("ok", "passed", 1.0)]);
    a.suite_errors = vec![SuiteError {
        name: "hooked".into(),
        duration_ms: 0.0,
        errors: vec![error("hook")],
    }];
    // a file that failed to load: named `file [ file ]`, its message from the error text
    let mut b = file(1, vec![]);
    b.failed_file = true;
    b.load_error = true;
    b.error_msg = "evaluate threw: Error: load boom".into();
    let out = errors_summary(&[a, b], &run(root, &names), false);
    let bar = |n: &str| format!("{}{n}⎯", "⎯".repeat(29 - n.chars().count()));
    let expected = [
        format!("\n{}\n\n", divider(" Failed Suites 2 ", None)),
        " FAIL  a.test.ts > hooked\nError: hook\n".into(),
        format!("{}\n\n", bar("[1/6]")),
        " FAIL  b.test.ts [ b.test.ts ]\nError: load boom\n".into(),
        format!("{}\n\n", bar("[2/6]")),
        format!("\n{}\n\n", divider(" Failed Tests 3 ", None)),
        " FAIL  a.test.ts > d > one\n FAIL  a.test.ts > two\nError: same\n".into(),
        format!("{}\n\n", bar("[3/6]")),
        " FAIL  a.test.ts > d > one\nError: second\n".into(),
        format!("{}\n\n", bar("[4/6]")),
        " FAIL  a.test.ts > three\nError: boom\n".into(),
        format!("{}\n\n", bar("[5/6]")),
    ]
    .concat();
    assert_eq!(out, expected);
    // nothing failed: no section
    assert_eq!(
        errors_summary(&[file(0, vec![])], &run(root, &names), false),
        ""
    );
}

#[test]
fn render_routes_console_blocks_and_filters_for_the_agent() {
    let names = [PathBuf::from("a.test.ts")];
    let root = Path::new("/r");
    let mut slow = test("d > slow", "passed", 301.0);
    slow.retry_count = 1;
    let mut a = file(
        0,
        vec![
            test("d > fast", "passed", 1.0),
            slow,
            test("bad", "failed", 2.0),
        ],
    );
    // each entry's content ends with its newline, as console.log writes it
    a.logs = vec![
        ("stdout".into(), String::new(), "top\n".into()),
        ("stderr".into(), "bad".into(), "oops\n".into()),
        ("stdout".into(), "d > fast".into(), "quiet\n".into()),
    ];
    let r = run(root, &names);
    let (out, err) = render(Kind::Default, std::slice::from_ref(&a), &r);
    assert!(out.starts_with(
        "\n RUN  v4.1.12 /r\n\nstdout | a.test.ts\ntop\n\nstdout | a.test.ts > d > fast\nquiet\n\n ❯ a.test.ts (3 tests | 1 failed) 12ms\n     ✓ fast 1ms\n     ✓ slow  301ms (retry x1)\n   × bad 2ms\n\n Test Files  1 passed (1)\n      Tests  1 passed (1)\n   Start at  10:00:00\n   Duration  1.50s (transform 0ms, setup 0ms, import 0ms, tests 304ms, environment 0ms)\n\n"
    ), "{out}");
    assert!(
        err.starts_with("stderr | a.test.ts > bad\noops\n\n"),
        "{err}"
    );
    // the agent: only failed tasks' console output (the failed file's own too) and failed tests
    let (out, err) = render(Kind::Agent, std::slice::from_ref(&a), &r);
    assert!(out.starts_with(
        "\n RUN  v4.1.12 /r\n\nstdout | a.test.ts\ntop\n\n ❯ a.test.ts (3 tests | 1 failed) 12ms\n   × bad 2ms\n\n"
    ), "{out}");
    assert!(
        err.starts_with("stderr | a.test.ts > bad\noops\n\n"),
        "{err}"
    );
    // a passing file: the agent prints no module line, the default reporter only the module
    // line and its slow tests
    let ok = file(
        0,
        vec![test("fast", "passed", 1.0), test("slow", "passed", 400.0)],
    );
    let (out, _) = render(Kind::Agent, std::slice::from_ref(&ok), &r);
    assert!(
        out.starts_with("\n RUN  v4.1.12 /r\n\n\n Test Files"),
        "{out}"
    );
    let (out, _) = render(Kind::Default, std::slice::from_ref(&ok), &r);
    assert!(
        out.contains("\n ✓ a.test.ts (2 tests) 12ms\n   ✓ slow  400ms\n\n"),
        "{out}"
    );
    // verbose: a line per test (no slow-test line of its own), each error's message under a failure
    let mut bad = test("bad", "failed", 2.0);
    bad.errors = vec![error("one"), error("two")];
    let v = file(0, vec![test("d > slow", "passed", 400.0), bad]);
    let (out, _) = render(Kind::Verbose, std::slice::from_ref(&v), &r);
    assert!(
        out.contains(
            "\n ✓ a.test.ts > d > slow 400ms\n × a.test.ts > bad 2ms\n   → one\n   → two\n\n"
        ),
        "{out}"
    );
}

#[test]
fn the_agent_prints_failed_tasks_output_as_each_task_finishes() {
    let log = |task: &str| ("stdout".to_string(), task.to_string(), format!("{task}\n"));
    let mut a = file(
        0,
        vec![
            test("o > i > fails", "failed", 1.0),
            test("o > i > passes", "passed", 1.0),
            test("o > calm > ok", "passed", 1.0),
            test("top", "passed", 1.0),
            test("h > never", "skipped", 0.0),
        ],
    );
    a.suite_errors = vec![SuiteError {
        name: "h".into(),
        ..Default::default()
    }];
    a.logs = [
        "",
        "o",
        "o > i",
        "o > i > fails",
        "o > i > passes",
        "o > calm",
        "top",
        "h",
        "gone",
    ]
    .map(log)
    .to_vec();
    let tasks: Vec<&str> = agent_logs(&a).iter().map(|l| l.1.as_str()).collect();
    // a failed test, then its suites inside out (a passing test's, a passing suite's and a
    // passing top-level test's output is left out), a suite whose hook failed, the file last
    assert_eq!(tasks, ["o > i > fails", "o > i", "o", "h", ""]);
    // a passing file prints nothing
    let mut ok = file(0, vec![test("t", "passed", 1.0)]);
    ok.logs = vec![log(""), log("t")];
    assert!(agent_logs(&ok).is_empty());
}
