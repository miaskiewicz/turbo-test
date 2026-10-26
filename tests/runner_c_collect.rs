//! Test collection and run order as vitest 4.1.12 does them: tasks run (and are reported) in
//! declaration order. Expected values are vitest 4.1.12's on the same files.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project, Run};

/// The TAP lines of a `--reporter tap` run (one `ok N - name` / `not ok N - name` per test).
fn tap_points(r: &Run) -> Vec<String> {
    r.stdout
        .lines()
        .filter(|l| l.starts_with("ok ") || l.starts_with("not ok "))
        .map(str::to_string)
        .collect()
}

#[test]
fn tasks_run_in_declaration_order() {
    // turbo-test#64: a block's own tests ran before its nested describes, so `last` ran before
    // `d > inner`. vitest 4.1.12 runs (and reports) first, d > inner, last.
    let p = Project::new("rc-collect-order");
    p.file(
        "o.test.js",
        "const log = []\n\
         test('first', () => { log.push('first') })\n\
         describe('d', () => {\n\
           test('inner', () => { log.push('inner') })\n\
           describe('deeper', () => { test('deep', () => { log.push('deep') }) })\n\
           test('inner2', () => { log.push('inner2') })\n\
         })\n\
         test('last', () => { log.push('last'); expect(log.join()).toBe('first,inner,deep,inner2,last') })\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "tap", "o.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(
        tap_points(&r),
        [
            "ok 1 - first",
            "ok 2 - d > inner",
            "ok 3 - d > deeper > deep",
            "ok 4 - d > inner2",
            "ok 5 - last",
        ],
        "{}",
        r.all()
    );
}

#[test]
fn only_runs_what_vitest_runs() {
    // turbo-test#65: `describe.only` skipped its own tests (only tests marked `.only` themselves
    // ran). Expected = vitest 4.1.12 (`interpretTaskModes`): a task runs when it or an enclosing
    // suite is `.only`, but inside a suite holding `.only` tasks only those run.
    let p = Project::new("rc-collect-only");
    p.file(
        "o.test.js",
        "describe.only('d', () => {\n\
           test('a', () => {})\n\
         })\n\
         test('outside', () => {})\n\
         describe('e', () => {\n\
           test.only('eo', () => {})\n\
           test('en', () => {})\n\
         })\n\
         describe.only('f', () => {\n\
           test.only('fo', () => {})\n\
           test('fn', () => {})\n\
           describe('fd', () => { test('fdt', () => {}) })\n\
         })\n\
         describe('g', () => {\n\
           describe.only('gi', () => { test('gt', () => {}) })\n\
           test('gx', () => {})\n\
         })\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "tap", "o.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(
        tap_points(&r),
        [
            "ok 1 - d > a",
            "ok 2 - outside # SKIP",
            "ok 3 - e > eo",
            "ok 4 - e > en # SKIP",
            "ok 5 - f > fo",
            "ok 6 - f > fn # SKIP",
            "ok 7 - f > fd > fdt # SKIP",
            "ok 8 - g > gi > gt",
            "ok 9 - g > gx # SKIP",
        ],
        "{}",
        r.all()
    );
}

/// The tests a `--reporter tap` run ran (its points without a `# SKIP` directive), names only.
fn ran(r: &Run) -> Vec<String> {
    tap_points(r)
        .into_iter()
        .filter(|l| !l.ends_with("# SKIP"))
        .map(|l| l.split_once(" - ").unwrap().1.to_string())
        .collect()
}

#[test]
fn name_pattern_matches_the_space_joined_full_name() {
    // turbo-test#66: -t was matched against `describe > test`. vitest 4.1.12 matches the full
    // name, the names joined with a space (`getTaskFullName`).
    let p = Project::new("rc-collect-tname");
    p.file(
        "a.test.js",
        "describe('outer', () => {\n\
           it('inner pass', () => {})\n\
           describe('deep', () => { it('leaf', () => {}) })\n\
         })\n\
         test('top', () => {})\n",
    );
    let cases: [(&str, &[&str]); 5] = [
        ("outer inner", &["outer > inner pass"]),
        ("outer > inner", &[]),
        ("^outer deep leaf$", &["outer > deep > leaf"]),
        ("deep", &["outer > deep > leaf"]),
        ("^top$", &["top"]),
    ];
    for (pat, want) in cases {
        let r = turbo_test(&p.dir, &["--reporter", "tap", "-t", pat, "a.test.js"], &[]);
        assert_eq!(r.code, Some(0), "{pat}\n{}", r.all());
        assert_eq!(ran(&r), want, "{pat}\n{}", r.all());
    }
}

/// `test.each` / `describe.each` titles: printf placeholders, `$key` paths, a tagged table and
/// every value kind vitest's `inspect` / `objDisplay` render differently.
const EACH_TITLES: &str = r#"class Pt { constructor() { this.x = 1 } }
function named() {}
test.each([1, 2])('num %i', () => {})
test.each([[1, 'a'], [2, 'b']])('pair %d %s', () => {})
test.each([{ a: 1, b: { c: 'x' } }])('obj $a $b.c %#', () => {})
test.each(['x'])('str %s %% %# %$', () => {})
test.each([[1.5, { k: 1 }, [1, 2]]])('fmt %f %j %o', () => {})
test.each([[{ k: 1 }, 'q']])('O %O %s', () => {})
test.each([[1, 2, 3]])('extra %s', () => {})
test.each([null, undefined, true])('lit %s', () => {})
test.each`
  a    | b
  ${1} | ${'two'}
  ${3} | ${{ x: 1 }}
`('tpl $a $b', () => {})
describe.each([1, 2])('desc %i', () => { test('inner', () => {}) })
describe.each([{ n: 'z' }])('desc $n', () => { test('inner', () => {}) })
it.each([5])('it %i', () => {})
test.each([[-0, NaN, Infinity, 10n]])('nums %s %d %i %f', () => {})
test.each([{ v: -0 }, { v: NaN }, { v: 0 }, { v: 10n }, { v: Symbol('s') }, { v: named }, { v: () => {} }])('attr $v', () => {})
test.each([{ v: "it's\n" }, { v: new Pt() }, { v: new Map([['k', 1]]) }, { v: new Set([1]) }, { v: /a+/gi }, { v: new Date(0) }])('attr2 $v', () => {})
test.each([{ v: { aaaa: 1, bbbb: 2, cccc: 3, dddd: 4, eeee: 5 } }, { v: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14] }, { v: 'x'.repeat(50) }])('long $v', () => {})
test.each([[{ toString() { return 'TS' } }, new Error('e1'), new Uint8Array([1, 2])]])('misc %s %o %O', () => {})
test.each([['p', 'q']])('arr $0 $1 $2', () => {})
test.each(['p'])('prim $0 $a', () => {})
test.each([{ a: [1, { b: 2 }] }])('path $a.1.b $a[0] $missing', () => {})
test.each([[1, 2]])(named, () => {})
test.each([{ 'a-b': 1, [Symbol('k')]: 2 }])('keys %o', () => {})
test.each([[1]])('json %j %c%s', () => {})
"#;

#[test]
fn each_titles_are_formatted_like_vitest() {
    // turbo-test#67: `.each` titles were the template plus ` [i]` (`num %i [0]`) and
    // `describe.each` kept the template. Expected = vitest 4.1.12's names for the same file.
    let p = Project::new("rc-collect-each");
    p.file("e.test.js", EACH_TITLES);
    let r = turbo_test(&p.dir, &["--reporter", "tap", "e.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    let names: Vec<String> = tap_points(&r)
        .iter()
        .map(|l| l.split_once(" - ").unwrap().1.to_string())
        .collect();
    let want = [
        "num 1",
        "num 2",
        "pair 1 a",
        "pair 2 b",
        "obj 1 'x' 0",
        "str x % undefined 0 1",
        "fmt 1.5 {\"k\":1} [ 1, 2 ]",
        "O { k: 1 } q",
        "extra 1",
        "lit null",
        "lit undefined",
        "lit true",
        "tpl 1 'two'",
        "tpl 3 { x: 1 }",
        "desc 1 > inner",
        "desc 2 > inner",
        "desc 'z' > inner",
        "it 5",
        "nums -0 NaN NaN -10",
        "attr -0",
        "attr NaN",
        "attr +0",
        "attr 10n",
        "attr Symbol(s)",
        "attr [Function named]",
        "attr [Function v]",
        "attr2 'it\\'s\\n'",
        "attr2 Pt{ x: 1 }",
        "attr2 Map{ 'k' => 1 }",
        "attr2 Set{ 1 }",
        "attr2 /a+/gi",
        "attr2 1970-01-01T00:00:00.000Z",
        "long { aaaa: 1, bbbb: 2, cccc: 3, …(2) }",
        "long [ 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, …(4) ]",
        "long 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx…'",
        "misc TS Error: e1 Uint8Array[ 1, 2 ]",
        "arr 'p' 'q' undefined",
        "prim 'p' $a",
        "path 2 [ 1, { b: 2 } ][0] undefined",
        "named",
        "keys { 'a-b': 1, [Symbol(k)]: 2 }",
        "json 1 undefined",
    ];
    assert_eq!(names, want, "{}", r.all());
}

#[test]
fn for_collects_one_task_per_row() {
    // turbo-test#68: `test.for` / `it.for` / `describe.for` were missing (TypeError at load).
    // vitest 4.1.12 passes each row as ONE argument plus the test context; the expected TAP is
    // vitest's for the same file.
    let p = Project::new("rc-collect-for");
    p.file(
        "f.test.js",
        r#"test.for([[1, 2], [3, 4]])('for %i %i', ([a, b], ctx) => {
  expect(a + 1).toBe(b)
  expect(typeof ctx.expect).toBe('function')
})
it.for([5])('it for %i', (n) => { expect(n).toBe(5) })
test.for`
  a | b
  ${1} | ${2}
`('tpl $a $b', ({ a, b }) => { expect(a + b).toBe(3) })
describe.for([{ n: 'x' }])('d $n', (row) => {
  test('t', () => { expect(row.n).toBe('x') })
})
test.for([[1, 2]])('opts %i', { timeout: 100 }, ([a]) => { expect(a).toBe(1) })
"#,
    );
    let r = turbo_test(&p.dir, &["--reporter", "tap", "f.test.js"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(
        tap_points(&r),
        [
            "ok 1 - for 1 2",
            "ok 2 - for 3 4",
            "ok 3 - it for 5",
            "ok 4 - tpl 1 2",
            "ok 5 - d 'x' > t",
            "ok 6 - opts 1",
        ],
        "{}",
        r.all()
    );
}

#[test]
fn a_failing_before_all_skips_its_suite_and_an_empty_describe_fails() {
    // turbo-test#69: a throwing beforeAll still ran its suite's tests (counted as one more failed
    // test), and an empty describe passed. vitest 4.1.12: the suite's tests (nested ones too) are
    // skipped, its afterAll still runs, and the file fails; an empty describe fails its file with
    // "No test found in suite <name>".
    let p = Project::new("rc-collect-hooks");
    p.file(
        "b.test.js",
        "const log = []\n\
         describe('ba', () => {\n\
           beforeAll(() => { log.push('ba'); throw new Error('ba boom') })\n\
           beforeEach(() => log.push('be'))\n\
           afterEach(() => log.push('ae'))\n\
           afterAll(() => log.push('aa'))\n\
           test('t1', () => log.push('t1'))\n\
           describe('nested', () => { beforeAll(() => log.push('nba')); test('t2', () => log.push('t2')) })\n\
         })\n\
         test('after', () => { expect(log.join()).toBe('ba,aa') })\n",
    );
    p.file(
        "e.test.js",
        "describe('empty', () => {})\ntest('ok', () => {})\n",
    );
    let r = turbo_test(&p.dir, &["--reporter", "tap", "b.test.js"], &[]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(
        tap_points(&r),
        [
            "ok 1 - ba > t1 # SKIP",
            "ok 2 - ba > nested > t2 # SKIP",
            "ok 3 - after",
        ],
        "{}",
        r.all()
    );
    let r = turbo_test(&p.dir, &["b.test.js", "e.test.js"], &[("TURBO_JOBS", "1")]);
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stdout.starts_with(
            "FAIL  b.test.js  (1 passed, 0 failed)\n        ✗ ba beforeAll: ba boom\n\
             FAIL  e.test.js  (1 passed, 0 failed)\n        ✗ No test found in suite empty\n"
        ),
        "{}",
        r.all()
    );
    assert!(
        r.stdout
            .contains("\n Test Files  2 failed (2)\n      Tests  2 passed | 2 skipped (4)\n"),
        "{}",
        r.all()
    );
}
