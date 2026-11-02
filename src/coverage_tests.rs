//! Unit tests for the pure parts of `coverage.rs`: threshold parsing, glob splitting / brace
//! expansion / matching, the hand-rolled VLQ source-map decoder, gen→orig position mapping, the
//! V8-range → line / function / branch / statement attribution, and every reporter's rendering.
//!
//! The recorders write into process-wide maps keyed by file path, so every test that records uses
//! its own virtual path (`/virtual/<test>.js`) — parallel tests never see each other's data, and
//! nothing here calls `report()` (which would read every file).

use super::*;
use std::collections::BTreeMap;

// ---- helpers ----------------------------------------------------------------------------------

/// The one-line CJS wrapper the runner compiles around the transformed source.
const WRAP: &str = "(function (exports, module, require, __filename, __dirname) {\n";

/// `src` wrapped the way the runner does, with an inline identity source map (each generated
/// line maps column-for-column onto the same original line).
fn identity_wrapped(src: &str) -> String {
    let n = src.lines().count().max(1);
    let mut m = String::from("AAAA");
    for _ in 1..n {
        m.push_str(";AACA");
    }
    inline_wrapped(src, &format!("{{\"version\":3,\"mappings\":\"{m}\"}}"))
}

/// `src` wrapped with an arbitrary inline source-map JSON document.
fn inline_wrapped(src: &str, map_json: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(map_json);
    format!("{WRAP}{src}\n//# sourceMappingURL=data:application/json;base64,{b64}\n}})")
}

/// Wrapped byte offset of the first occurrence of `needle` in `src` (+ `extra` bytes).
fn at(src: &str, needle: &str, extra: usize) -> usize {
    WRAP.len()
        + src
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in source"))
        + extra
}

/// Wrapped byte offset of the LAST occurrence of `needle` in `src` (+ `extra` bytes).
fn at_last(src: &str, needle: &str, extra: usize) -> usize {
    WRAP.len() + src.rfind(needle).unwrap() + extra
}

fn vpath(name: &str) -> PathBuf {
    PathBuf::from(format!("/virtual/coverage-unit/{name}"))
}

fn lines_of(p: &Path) -> BTreeMap<u32, u64> {
    let g = COV.lock().unwrap();
    g.as_ref()
        .and_then(|m| m.get(p))
        .map(|m| m.iter().map(|(k, v)| (*k, *v)).collect())
        .unwrap_or_default()
}

fn fns_of(p: &Path) -> BTreeMap<(u32, String), u64> {
    let g = FN_COV.lock().unwrap();
    g.as_ref()
        .and_then(|m| m.get(p))
        .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
        .unwrap_or_default()
}

fn brs_of(p: &Path) -> BTreeMap<(u32, u32), (u32, bool, u64)> {
    let g = BR_COV.lock().unwrap();
    g.as_ref()
        .and_then(|m| m.get(p))
        .map(|m| m.iter().map(|(k, v)| (*k, *v)).collect())
        .unwrap_or_default()
}

fn stmts_of(p: &Path) -> BTreeMap<u32, (u32, u64)> {
    let g = ST_COV.lock().unwrap();
    g.as_ref()
        .and_then(|m| m.get(p))
        .map(|m| m.iter().map(|(k, v)| (*k, *v)).collect())
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn row(
    path: &str,
    lh: u64,
    lf: u64,
    fnh: u64,
    fnf: u64,
    brh: u64,
    brf: u64,
    sh: u64,
    sf: u64,
) -> Row {
    Row {
        path: PathBuf::from(path),
        lh,
        lf,
        fnh,
        fnf,
        brh,
        brf,
        sh,
        sf,
    }
}

// ---- thresholds -------------------------------------------------------------------------------

#[test]
fn parse_thresholds_reads_every_metric_and_aliases() {
    let t = parse_thresholds(
        Thresholds::default(),
        "lines=90,functions=80,branches=70.5,statements=60",
    );
    assert_eq!(
        t,
        Thresholds {
            lines: Some(90.0),
            functions: Some(80.0),
            branches: Some(70.5),
            statements: Some(60.0)
        }
    );
    // `funcs` / `stmts` aliases, whitespace around keys and values, any order.
    let t = parse_thresholds(Thresholds::default(), " stmts = 12 , funcs= 34 ");
    assert_eq!(
        t,
        Thresholds {
            lines: None,
            functions: Some(34.0),
            branches: None,
            statements: Some(12.0)
        }
    );
}

#[test]
fn parse_thresholds_skips_malformed_parts_and_keeps_the_base() {
    let base = Thresholds {
        lines: Some(50.0),
        ..Default::default()
    };
    // no `=`, a non-numeric value, an unknown key, an empty part: all ignored.
    let t = parse_thresholds(base, "bogus,branches=abc,foo=10,,functions=75");
    assert_eq!(
        t,
        Thresholds {
            lines: Some(50.0),
            functions: Some(75.0),
            branches: None,
            statements: None
        }
    );
    // a later spec overrides the earlier value for the same metric
    let t = parse_thresholds(t, "lines=99");
    assert_eq!(t.lines, Some(99.0));
    assert_eq!(t.functions, Some(75.0));
    assert_eq!(
        parse_thresholds(Thresholds::default(), ""),
        Thresholds::default()
    );
}

#[test]
fn threshold_failures_lists_total_then_per_file() {
    let cwd = Path::new("/proj");
    let rows = vec![
        row("/proj/src/a.ts", 10, 10, 2, 2, 4, 4, 8, 8),
        row("/proj/src/b.ts", 5, 10, 1, 2, 1, 4, 4, 8),
        row("/elsewhere/c.ts", 0, 0, 0, 0, 0, 0, 0, 0),
    ];
    let total = row("", 15, 20, 3, 4, 5, 8, 12, 16);
    let t = Thresholds {
        lines: Some(80.0),
        functions: Some(75.0),
        branches: Some(60.0),
        statements: Some(70.0),
    };
    // total: lines 75 < 80, functions 75 == 75 passes, branches 62.5 passes, stmts 75 passes.
    assert_eq!(
        threshold_failures(&t, false, &total, &rows, cwd),
        vec!["total: lines 75.00% < 80%"]
    );
    // per-file adds b.ts (every metric 50%) but not a.ts (100%) nor the empty c.ts (0/0 = 100%).
    assert_eq!(
        threshold_failures(&t, true, &total, &rows, cwd),
        vec![
            "total: lines 75.00% < 80%",
            "src/b.ts: lines 50.00% < 80%",
            "src/b.ts: functions 50.00% < 75%",
            "src/b.ts: branches 25.00% < 60%",
            "src/b.ts: statements 50.00% < 70%",
        ]
    );
    // nothing configured → nothing fails
    assert!(threshold_failures(&Thresholds::default(), true, &total, &rows, cwd).is_empty());
}

#[test]
fn threshold_failures_tolerates_float_rounding_at_the_boundary() {
    // 1/3 * 3 style imprecision: 33.333...% vs a 33.333333333333336 minimum still passes (1e-9).
    let r = row("/p/x.ts", 1, 3, 0, 0, 0, 0, 0, 0);
    let t = Thresholds {
        lines: Some(100.0 / 3.0),
        ..Default::default()
    };
    assert!(threshold_failures(&t, false, &r, &[], Path::new("/p")).is_empty());
    let t = Thresholds {
        lines: Some(33.34),
        ..Default::default()
    };
    assert_eq!(
        threshold_failures(&t, false, &r, &[], Path::new("/p")),
        vec!["total: lines 33.33% < 33.34%"]
    );
}

#[test]
fn gate_message_renders_pass_and_failures() {
    assert_eq!(gate_message(&[]), "  coverage thresholds met ✓\n");
    assert_eq!(
        gate_message(&["total: lines 1.00% < 2%".to_string(), "a.ts: branches 0.00% < 50%".to_string()]),
        "\n  coverage threshold FAILURES (2):\n    ✗ total: lines 1.00% < 2%\n    ✗ a.ts: branches 0.00% < 50%\n"
    );
}

#[test]
fn zero_files_message_names_the_cause() {
    assert_eq!(
        zero_files_message(true, &[]),
        "\n  coverage ERROR: 0 source files instrumented.\n    No instrumentable source was loaded under --coverage.\n    Refusing to report a vacuous 0/0 pass — check coverage.include / paths.\n"
    );
    let inc = vec!["src/**/*.{ts,tsx}".to_string(), "lib/**".to_string()];
    let m = zero_files_message(false, &inc);
    assert!(m.contains("    The coverage.include globs matched no loaded source: [src/**/*.{ts,tsx}, lib/**]\n"), "{m}");
    assert!(!m.contains("No instrumentable source"), "{m}");
    assert!(m.ends_with("check coverage.include / paths.\n"), "{m}");
}

// ---- globs ------------------------------------------------------------------------------------

#[test]
fn split_globs_trims_drops_empties_and_tolerates_stray_close_brace() {
    assert!(split_globs("").is_empty());
    assert!(split_globs(" , ,").is_empty());
    assert_eq!(split_globs(" a/** ,, b/*.ts "), vec!["a/**", "b/*.ts"]);
    // a stray `}` never drives the depth negative, so the next top-level comma still splits
    assert_eq!(split_globs("a},b"), vec!["a}", "b"]);
    // nested braces stay one glob
    assert_eq!(split_globs("x/{a,{b,c}}/*,y"), vec!["x/{a,{b,c}}/*", "y"]);
}

#[test]
fn expand_braces_handles_none_unclosed_and_multiple_groups() {
    assert_eq!(expand_braces("src/**/*.ts"), vec!["src/**/*.ts"]);
    assert_eq!(expand_braces("src/{a"), vec!["src/{a"]); // unclosed: literal
    assert_eq!(
        expand_braces("{a,b}.{c,d}"),
        vec!["a.c", "a.d", "b.c", "b.d"]
    );
    assert_eq!(expand_braces("x{,y}"), vec!["x", "xy"]); // empty alternative
}

#[test]
fn simple_match_question_mark_and_single_star() {
    assert!(glob_match("a?c", "abc"));
    assert!(!glob_match("a?c", "a/c")); // `?` never matches a separator
    assert!(!glob_match("a?c", "ac"));
    assert!(glob_match("a*c", "abbbc")); // `*` backtracks
    assert!(glob_match("a*c", "ac"));
    assert!(!glob_match("a*c", "ab/c")); // `*` never crosses '/'
    assert!(!glob_match("a*", "a/b"));
    assert!(glob_match("src/*", "src/x"));
    assert!(glob_match("src/*", "src/")); // trailing star matches empty
    assert!(!glob_match("abc", "ab")); // pattern longer than path
    assert!(!glob_match("ab", "abc")); // path longer than pattern, no star
    assert!(!glob_match("x*y", "xay/b"));
}

#[test]
fn simple_match_double_star() {
    assert!(glob_match("**", "a/b/c"));
    assert!(glob_match("src/**/x.ts", "src/x.ts")); // `**/` matches zero dirs
    assert!(glob_match("src/**/x.ts", "src/a/b/x.ts"));
    assert!(!glob_match("src/**/x.ts", "src/a/y.ts"));
    assert!(!glob_match("src/**/x.ts", "lib/a/x.ts"));
    assert!(glob_match("**/*.test.ts", "a/b/c.test.ts"));
    assert!(glob_match("a/**", "a/")); // trailing `**`
    assert!(glob_match("src**x", "src/a/x")); // `**` with no separator after it
                                              // the public alias used by the launcher's include discovery agrees
    assert!(glob_matches("src/**/*.{ts,tsx}", "src/a/b.tsx"));
    assert!(!glob_matches("src/**/*.{ts,tsx}", "src/a/b.jsx"));
}

#[test]
fn glob_filter_applies_include_then_exclude() {
    let inc = vec!["src/**/*.{ts,tsx}".to_string()];
    let exc = vec!["**/*.gen.ts".to_string()];
    assert!(glob_filter("anything/at/all.js", &[], &[])); // no include = everything
    assert!(glob_filter("src/a.ts", &inc, &exc));
    assert!(!glob_filter("lib/a.ts", &inc, &exc));
    assert!(!glob_filter("src/a.gen.ts", &inc, &exc));
    assert!(!glob_filter("x.gen.ts", &[], &exc));
}

#[test]
fn ignore_directive_is_honored_within_the_first_ten_lines() {
    let marks = [
        "turbo-test-coverage-ignore-file",
        "coverage-ignore-file",
        "disable-test-coverage-check",
    ];
    for (i, m) in marks.iter().enumerate() {
        let p = vpath(&format!("ignore-{i}.js"));
        scan_ignore_directive(&p, &format!("// first\n/* {m} */\nconst x = 1;\n"));
        assert!(is_ignored(&p), "{m}");
    }
    let tenth = vpath("ignore-tenth.js");
    scan_ignore_directive(
        &tenth,
        &format!("{}// coverage-ignore-file\n", "\n".repeat(9)),
    );
    assert!(is_ignored(&tenth));
    let eleventh = vpath("ignore-eleventh.js");
    scan_ignore_directive(
        &eleventh,
        &format!("{}// coverage-ignore-file\n", "\n".repeat(10)),
    );
    assert!(!is_ignored(&eleventh));
    let plain = vpath("ignore-none.js");
    scan_ignore_directive(&plain, "const coverage = 1;\n");
    assert!(!is_ignored(&plain));
}

// ---- source maps ------------------------------------------------------------------------------

#[test]
fn byte_to_linecol_counts_utf16_columns() {
    let src = "ab\ncd\né😀x";
    assert_eq!(byte_to_linecol(src, 0), (0, 0));
    assert_eq!(byte_to_linecol(src, 1), (0, 1));
    assert_eq!(byte_to_linecol(src, 3), (1, 0));
    assert_eq!(byte_to_linecol(src, 6), (2, 0));
    // é = 2 bytes / 1 UTF-16 unit; 😀 = 4 bytes / 2 UTF-16 units
    assert_eq!(byte_to_linecol(src, 8), (2, 1));
    assert_eq!(byte_to_linecol(src, 12), (2, 3));
    assert_eq!(byte_to_linecol(src, 1000), (2, 4)); // past the end clamps to EOF
}

#[test]
fn vlq_decode_signs_continuations_and_garbage() {
    assert_eq!(vlq_decode("AAAA"), vec![0, 0, 0, 0]);
    assert_eq!(vlq_decode("C"), vec![1]);
    assert_eq!(vlq_decode("D"), vec![-1]);
    assert_eq!(vlq_decode("gB"), vec![16]); // continuation bit
    assert_eq!(vlq_decode("2HwBC"), vec![123, 24, 1]);
    assert_eq!(vlq_decode("A!C"), vec![0, 1]); // non-base64 chars skipped
    assert!(vlq_decode("!!").is_empty());
}

#[test]
fn parse_mappings_full_accumulates_across_lines() {
    // line 0: col0→(0,0), col4→(0,4); line 1: empty; line 2: col2→(1,0); line 3: gen-col-only
    // segment (no source) + garbage; line 4: a segment whose source column goes negative.
    let (genmap, segs) = parse_mappings_full("AAAA,IAAI;;EACJ;A,!!;AAAL");
    assert_eq!(segs.len(), 5);
    assert_eq!(segs[0], vec![(0, 0, 0), (4, 0, 4)]);
    assert!(segs[1].is_empty());
    assert_eq!(segs[2], vec![(2, 1, 0)]);
    assert!(segs[3].is_empty());
    assert!(segs[4].is_empty()); // src_col -5 → dropped from the segment table
    let gm: BTreeMap<u32, u32> = genmap.into_iter().collect();
    // line 4 still maps to its (non-negative) source line for line coverage
    assert_eq!(gm, BTreeMap::from([(0, 0), (2, 1), (4, 1)]));
}

#[test]
fn decode_inline_map_requires_a_well_formed_inline_map() {
    assert!(decode_inline_map("const x = 1;").is_none());
    let marker = "//# sourceMappingURL=data:application/json;base64,";
    assert!(decode_inline_map(&format!("x\n{marker}")).is_none()); // nothing after the marker
    assert!(decode_inline_map(&format!("x\n{marker}%%%notbase64\n")).is_none());
    let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
    assert!(decode_inline_map(&format!("x\n{marker}{}\n", b64("not json"))).is_none());
    assert!(decode_inline_map(&format!("x\n{marker}{}\n", b64("{\"version\":3}"))).is_none());
    assert!(decode_inline_map(&format!("x\n{marker}{}\n", b64("{\"mappings\":7}"))).is_none());
    let (gm, segs) = decode_inline_map(&format!(
        "x\n{marker}{}\n",
        b64("{\"mappings\":\"AAAA;AACA\"}")
    ))
    .unwrap();
    assert_eq!(gm.get(&1), Some(&1));
    assert_eq!(segs.len(), 2);
}

#[test]
fn register_meta_caches_and_tolerates_unparseable_maps() {
    let p = vpath("no-map.js");
    assert!(!has_meta(&p));
    register_meta(&p, "(function () {\nconst x = 1;\n})", "const x = 1;\n");
    assert!(has_meta(&p)); // remembered (so it is not rebuilt), but holds no map
    assert!(meta_for(&p).is_none());
    // every mapper is a no-op without a map
    map_script(&p, &[(0, 10, 1)], &[("f".to_string(), 0, 1)]);
    map_branches(&p, &[(0, 10, 1)]);
    map_statements(&p, &[(0, 10, 1)]);
    assert!(lines_of(&p).is_empty() && fns_of(&p).is_empty());
    assert!(brs_of(&p).is_empty() && stmts_of(&p).is_empty());
    // a source carrying the ignore directive is marked while registering
    let ig = vpath("register-ignored.js");
    register_meta(
        &ig,
        &identity_wrapped("// coverage-ignore-file\nf();"),
        "// coverage-ignore-file\nf();",
    );
    assert!(is_ignored(&ig));
}

#[test]
fn gen_to_orig_maps_through_segments() {
    let src = "  abc;\n\nxyz;";
    // line 0 starts its only segment at column 2; line 1 has none; line 2 is identity.
    let w = inline_wrapped(src, "{\"mappings\":\"EAAE;;AAEF\"}");
    let p = vpath("gen-to-orig.js");
    register_meta(&p, &w, src);
    let meta = meta_for(&p).unwrap();
    assert_eq!(gen_to_orig(&meta, 0), None); // wrapper line
    assert_eq!(gen_to_orig(&meta, WRAP.len()), None); // before the first segment's column
    assert_eq!(gen_to_orig(&meta, WRAP.len() + 3), Some((0, 3)));
    assert_eq!(gen_to_orig(&meta, WRAP.len() + 7), None); // empty source line, no segments
    assert_eq!(gen_to_orig(&meta, WRAP.len() + 9), Some((2, 1)));
    // the trailing marker / `})` lines have no generated-line entry at all
    assert_eq!(gen_to_orig(&meta, w.len() - 1), None);
}

/// An ESM source as esbuild emits it under `--format=cjs`: the interop prologue (export table +
/// `__toCommonJS`, every line mapped to (0, 0) only), then the code. Source line 1 holds a whole
/// function whose export getter (`longExportedName: () => …`) is never read.
const ESM_SRC: &str = "\
export function longExportedName() { return 1; }
export function api() {
  return longExportedName();
}
";
const ESM_GEN: &str = "\
var m_exports = {};
__export(m_exports, {
  api: () => api,
  longExportedName: () => longExportedName
});
module.exports = __toCommonJS(m_exports);
function longExportedName() {
  return 1;
}
function api() {
  return longExportedName();
}";
const ESM_MAP: &str = "AAAA;AAAA;AAAA;AAAA;AAAA;AAAA;AAAO;EAA8B;AAAW;AACzC;EACL;AACF";

fn esm_meta(name: &str) -> PathBuf {
    let p = vpath(name);
    let w = inline_wrapped(ESM_GEN, &format!("{{\"mappings\":\"{ESM_MAP}\"}}"));
    register_meta(&p, &w, ESM_SRC);
    p
}

#[test]
fn interop_header_end_spans_the_esbuild_prologue_only() {
    let (_, segs) = parse_mappings_full(ESM_MAP);
    let gen: Vec<&str> = ESM_GEN.lines().collect();
    assert_eq!(interop_header_end(&gen, &segs), 6);
    // CommonJS output: real mappings come first, so there is no prologue
    let (_, cjs) = parse_mappings_full("AAAA,IAAI;AACA");
    let cjs_gen = ["const a = 1;", "module.exports = __toCommonJS(a);"];
    assert_eq!(interop_header_end(&cjs_gen, &cjs), 0);
    // (0, 0)-only lines without the closing `__toCommonJS` line are not a prologue either
    assert_eq!(interop_header_end(&gen[..5], &segs), 0);
}

#[test]
fn interop_prologue_has_no_source_positions_or_functions() {
    let p = esm_meta("esm-prologue.js");
    let meta = meta_for(&p).unwrap();
    assert_eq!(meta.header_end, 6);
    let getter = at(ESM_GEN, "() => longExportedName", 0);
    assert_eq!(gen_to_orig(&meta, getter), None);
    assert_eq!(
        gen_to_orig(&meta, at(ESM_GEN, "function longExportedName", 0)),
        Some((0, 7))
    );
    let lf = (
        at(ESM_GEN, "function longExportedName", 0),
        at(ESM_GEN, "}\nfunction api", 1),
    );
    let api = (at(ESM_GEN, "function api", 0), WRAP.len() + ESM_GEN.len());
    let ranges = vec![
        (getter, at(ESM_GEN, "\n});", 0), 0), // the getter is never read
        (lf.0, lf.1, 1),                      // …but the function runs, via api()
        (api.0, api.1, 1),
    ];
    // export getters (named after their export, mapped to line 1) are not the source's functions
    let funcs = vec![
        ("api".to_string(), at(ESM_GEN, "() => api", 0), 1),
        ("longExportedName".to_string(), getter, 0),
        ("longExportedName".to_string(), lf.0, 1),
        ("api".to_string(), api.0, 1),
    ];
    map_script(&p, &ranges, &funcs);
    assert_eq!(
        fns_of(&p),
        BTreeMap::from([
            ((1, "longExportedName".to_string()), 1),
            ((2, "api".to_string()), 1)
        ])
    );
    // the unread getter's 0-count range does not land on `return 1` (source line 1, col 37)
    map_statements(&p, &ranges);
    assert_eq!(stmts_of(&p), BTreeMap::from([(0, (1, 1)), (1, (3, 1))]));
}

#[test]
fn interop_prologue_lines_are_not_source_line_1() {
    let p = esm_meta("esm-prologue-lines.js");
    let lf = (
        at(ESM_GEN, "function longExportedName", 0),
        at(ESM_GEN, "}\nfunction api", 1),
    );
    let ranges = vec![
        (0, WRAP.len() + ESM_GEN.len(), 1), // module body: the prologue runs
        (lf.0, lf.1, 0),                    // the line-1 function never does
    ];
    map_script(&p, &ranges, &[]);
    // line 1 is only the never-run function; the prologue (count 1) does not count as line 1
    assert_eq!(
        lines_of(&p),
        BTreeMap::from([(1, 0), (2, 1), (3, 1), (4, 1)])
    );
}

/// turbo-test#34's thr.js and the code part of esbuild's output for it, with esbuild's own
/// mappings: the indented line starts its first segment on the indentation, and the `throw`
/// segment starts on the space before the keyword (gen col 8 → source col 9).
const THR_SRC: &str = "export function g(c) {\n  if (c) throw new Error('x');\n  return 1;\n}\n";
const THR_GEN: &str = "function g(c) {\n  if (c) throw new Error(\"x\");\n  return 1;\n}";
const THR_MAP: &str = "AAAO,SAAS,EAAE,GAAG;AACnB,MAAI,EAAG,OAAM,IAAI,MAAM,GAAG;AAC1B,SAAO;AACT";

#[test]
fn gen_to_orig_skips_a_segments_blank_lead_in() {
    let p = vpath("thr-lead.js");
    let w = inline_wrapped(THR_GEN, &format!("{{\"mappings\":\"{THR_MAP}\"}}"));
    register_meta(&p, &w, THR_SRC);
    let meta = meta_for(&p).unwrap();
    let line1 = at(THR_GEN, "  if", 0);
    assert_eq!(gen_to_orig(&meta, line1), Some((1, 2))); // indentation: the segment's own start
    assert_eq!(gen_to_orig(&meta, line1 + 1), Some((1, 2)));
    assert_eq!(gen_to_orig(&meta, line1 + 2), Some((1, 2))); // `if`
    assert_eq!(gen_to_orig(&meta, line1 + 3), Some((1, 3)));
    assert_eq!(gen_to_orig(&meta, line1 + 9), Some((1, 9))); // `throw` (was col 10)
    assert_eq!(gen_to_orig(&meta, line1 + 12), Some((1, 12)));
    assert_eq!(gen_to_orig(&meta, line1 + 15), Some((1, 15))); // `new`: next segment, no lead
                                                               // a lead-in whose source position is itself blank is a real (identity) run: no skipping
    let q = vpath("blank-source-lead.js");
    let src = "a  =  1;";
    register_meta(
        &q,
        &inline_wrapped(src, "{\"mappings\":\"AAAA,CAAC\"}"),
        src,
    );
    let qm = meta_for(&q).unwrap();
    assert_eq!(gen_to_orig(&qm, WRAP.len() + 3), Some((0, 3)));
}

#[test]
fn braceless_consequent_statement_takes_its_own_range() {
    let p = vpath("thr-statements.js");
    let w = inline_wrapped(THR_GEN, &format!("{{\"mappings\":\"{THR_MAP}\"}}"));
    register_meta(&p, &w, THR_SRC);
    // V8's ranges for g(false): the function, and the never-run `throw` (it starts AT the keyword)
    let ranges = vec![
        (at(THR_GEN, "function", 0), WRAP.len() + THR_GEN.len(), 1),
        (at(THR_GEN, "throw", 0), at(THR_GEN, "\n  return", 0), 0),
    ];
    map_statements(&p, &ranges);
    // if (2), throw (2), return 1 (3)
    assert_eq!(
        stmts_of(&p),
        BTreeMap::from([(0, (2, 1)), (1, (2, 0)), (2, (3, 1))])
    );
    map_branches(&p, &ranges);
    assert_eq!(
        brs_of(&p),
        BTreeMap::from([((0, 0), (2, true, 0)), ((0, 1), (2, true, 1))])
    );
}

/// turbo-test#34's sw.js and the code part of esbuild's output, with esbuild's own mappings:
/// each `case`/`default`/`}` line opens with a segment covering only its indentation, mapped to
/// where the previous source token ended (`    default:` → the `2` of `return 2;`).
const SW_SRC: &str = "\
export function sw(x) {
  switch (x) {
    case 'a': return 1;
    case 'b': return 2;
    default: return 0;
  }
}
";
const SW_GEN: &str = "\
function sw(x) {
  switch (x) {
    case \"a\":
      return 1;
    case \"b\":
      return 2;
    default:
      return 0;
  }
}";
const SW_MAP: &str = "AAAO,SAAS,GAAG,GAAG;AACpB,UAAQ,GAAG;AAAA,IACT,KAAK;AAAK,aAAO;AAAA,IACjB,KAAK;AAAK,aAAO;AAAA,IACjB;AAAS,aAAO;AAAA,EAClB;AACF";

#[test]
fn a_line_opening_with_a_blank_only_segment_is_not_attributed_to_the_previous_line() {
    let p = vpath("switch-lines.js");
    let w = inline_wrapped(SW_GEN, &format!("{{\"mappings\":\"{SW_MAP}\"}}"));
    register_meta(&p, &w, SW_SRC);
    assert_eq!(
        meta_for(&p).unwrap().blank_first,
        vec![false, false, true, false, true, false, true, false, true, false]
    );
    // V8's ranges for sw('a'): the function, and the two cases that never ran
    let ranges = vec![
        (at(SW_GEN, "function", 0), WRAP.len() + SW_GEN.len(), 1),
        (at(SW_GEN, "case \"b\"", 0), at(SW_GEN, "return 2;", 9), 0),
        (at(SW_GEN, "default", 0), at(SW_GEN, "return 0;", 9), 0),
    ];
    map_script(&p, &ranges, &[]);
    // `    default:` and `  }` are credited to lines 4 and 5 (where their indentation segment
    // points) but open outside both case ranges: they take the count of the token they point at
    assert_eq!(
        lines_of(&p),
        BTreeMap::from([(1, 1), (2, 1), (3, 1), (4, 0), (5, 0), (7, 1)])
    );
}

// ---- mapping V8 ranges onto lines / functions / branches / statements -----------------------

#[test]
fn map_script_attributes_innermost_counts_to_lines_and_functions() {
    let src = "function f() {\n  return 1;\n}\nfunction g() {\n  return 2;\n}\nf();";
    let p = vpath("map-script.js");
    register_meta(&p, &identity_wrapped(src), src);
    let f_start = at(src, "function f", 0);
    let g_start = at(src, "function g", 0);
    let g_end = at(src, "}\nf();", 1);
    let ranges = vec![
        (0, WRAP.len() + src.len(), 1), // module body (covers the wrapper line too)
        (f_start, g_start, 3),
        (g_start, g_end, 0),
        (at(src, "  return 2", 0), at(src, "  return 2", 5), -1), // negative counts clamp to 0
    ];
    let funcs = vec![
        ("f".to_string(), f_start, 3),
        (String::new(), g_start, -2),  // anonymous + negative
        ("wrapper".to_string(), 0, 1), // starts on the wrapper line → not attributable
    ];
    map_script(&p, &ranges, &funcs);
    assert_eq!(
        lines_of(&p),
        BTreeMap::from([(1, 3), (2, 3), (3, 3), (4, 0), (5, 0), (6, 0), (7, 1)])
    );
    assert_eq!(
        fns_of(&p),
        BTreeMap::from([
            ((1, "f".to_string()), 3),
            ((4, "(anonymous)".to_string()), 0)
        ])
    );
    // a second run merges by max: lower counts never lower a line, higher ones raise it
    map_script(
        &p,
        &[(g_start, g_end, 5), (f_start, g_start, 1)],
        &[("f".to_string(), f_start, 1)],
    );
    let l = lines_of(&p);
    assert_eq!((l[&1], l[&4], l[&5]), (3, 5, 5));
    assert_eq!(fns_of(&p)[&(1, "f".to_string())], 3);
}

#[test]
fn map_branches_if_else_ternary_logical_and_switch() {
    let src = "\
function f(x, y) {
  if (x) {
    a();
  } else {
    b();
  }
  const t = x ? 1 : 2;
  const l = x && y;
  switch (y) {
    case 1:
      break;
    default:
      c();
  }
}";
    let p = vpath("map-branches.js");
    register_meta(&p, &identity_wrapped(src), src);
    let body = (at(src, "function", 0), WRAP.len() + src.len(), 4);
    let ranges = vec![
        (0, WRAP.len() + src.len(), 1), // module wrapper: opens on line 0 → starts at (0, 0)
        body,
        (at(src, "{\n    a()", 0), at(src, "} else", 1), 3), // consequent
        (at(src, "{\n    b()", 0), at(src, "  }\n  const t", 3), 1), // alternate
        (at(src, "2;", 0), at(src, "2;", 1), 1),             // ternary alternate
        (at(src, "y;", 0), at(src, "y;", 1), 0),             // `&& y` never evaluated
        (at(src, "case 1", 0), at(src, "    default", 0), 0),
        (at(src, "default", 0), at_last(src, "}", 0), 4),
        (at(src, "x) {", 0), at(src, "x) {", 0), 9), // empty range: ignored
    ];
    map_branches(&p, &ranges);
    let b = brs_of(&p);
    assert_eq!(b[&(0, 0)], (2, true, 3)); // if → then
    assert_eq!(b[&(0, 1)], (2, true, 1)); // if → else
    assert_eq!(b[&(1, 0)], (7, true, 4)); // ternary consequent: falls back to the body count
    assert_eq!(b[&(1, 1)], (7, true, 1));
    assert_eq!(b[&(2, 0)], (8, true, 4)); // && left operand
    assert_eq!(b[&(2, 1)], (8, true, 0)); // && right operand never ran
    assert_eq!(b[&(3, 0)], (9, true, 0)); // case 1
    assert_eq!(b[&(3, 1)], (9, true, 4)); // default
    assert_eq!(b.len(), 8);
}

#[test]
fn map_branches_implicit_else_derives_counts_from_sub_ranges() {
    // V8's ranges for `g` called 5 times. V8 keeps a nested range only when its count differs from
    // the enclosing range's: a consequent's when it ran sometimes or never (NOT always), the
    // continuation after the `if` when the consequent returned. The then-count is the count at the
    // consequent's start; else = (if-statement executions) − then.
    let run = |name: &str, src: &str, ranges: Vec<(usize, usize, i64)>| {
        let p = vpath(name);
        register_meta(&p, &identity_wrapped(src), src);
        map_branches(&p, &ranges);
        let b = brs_of(&p);
        (b[&(0, 0)], b[&(0, 1)])
    };
    let ret = "function g(x) {\n  if (x) return 1;\n  return 2;\n}";
    let ret_ranges = |consequent: Option<i64>, continuation: Option<i64>| {
        let mut r = vec![(at(ret, "function", 0), WRAP.len() + ret.len(), 5)];
        if let Some(c) = consequent {
            r.push((at(ret, "return 1", 0), at(ret, "return 1", 9), c));
        }
        if let Some(c) = continuation {
            r.push((at(ret, "\n  return 2", 0), at_last(ret, "}", 0), c));
        }
        r
    };
    // then sometimes taken: its own range's count is the then-count
    assert_eq!(
        run("ie-some.js", ret, ret_ranges(Some(3), Some(2))),
        ((2, true, 3), (2, true, 2))
    );
    // then always taken: no consequent range (the enclosing count is the then-count); the code
    // after the `if` never ran
    assert_eq!(
        run("ie-always.js", ret, ret_ranges(None, Some(0))),
        ((2, true, 5), (2, true, 0))
    );
    // then never taken: the consequent keeps its 0-count range, the continuation has none
    assert_eq!(
        run("ie-never.js", ret, ret_ranges(Some(0), None)),
        ((2, true, 0), (2, true, 5))
    );
    // a consequent that falls through, then a nested function `h` that g returns (its callers
    // call it any number of times)
    let fall = "function g(x) {\n  if (x) {\n    x++;\n  }\n  const h = () => x;\n  return h;\n}";
    let fall_ranges = |consequent: Option<i64>, h: Option<i64>| {
        let mut r = vec![(at(fall, "function", 0), WRAP.len() + fall.len(), 5)];
        if let Some(c) = consequent {
            r.push((at(fall, "{\n    x++", 0), at(fall, "}\n  const", 1), c));
        }
        if let Some(c) = h {
            r.push((at(fall, "() => x", 0), at(fall, "() => x", 7), c));
        }
        r
    };
    assert_eq!(
        run("ie-fall-some.js", fall, fall_ranges(Some(3), Some(7))),
        ((2, true, 3), (2, true, 2))
    );
    assert_eq!(
        run("ie-fall-never.js", fall, fall_ranges(Some(0), Some(7))),
        ((2, true, 0), (2, true, 5))
    );
    // turbo-test#40: always taken, so V8 emits no range besides the function's (and `h`'s). This
    // was read as "then never taken" (then 0 / else 5), and a later nested function's range was
    // taken for the continuation after the `if` (else = `h`'s count)
    for (name, h) in [
        ("ie-fall-always.js", None),
        ("ie-fall-always-h-called.js", Some(3)),
        ("ie-fall-always-h-uncalled.js", Some(0)),
    ] {
        assert_eq!(
            run(name, fall, fall_ranges(None, h)),
            ((2, true, 5), (2, true, 0)),
            "{name}"
        );
    }
    // the decision itself never reached (no range covers it): not reached, nothing taken
    assert_eq!(
        run("ie-unreached.js", ret, vec![]),
        ((2, false, 0), (2, false, 0))
    );
}

#[test]
fn map_branches_and_statements_skip_sources_without_any() {
    let src = "f();";
    let p = vpath("no-branches.js");
    register_meta(&p, &identity_wrapped(src), src);
    map_branches(&p, &[(WRAP.len(), WRAP.len() + 4, 1)]);
    assert!(brs_of(&p).is_empty());
    let empty = vpath("empty-source.js");
    register_meta(&empty, &identity_wrapped(""), "");
    map_statements(&empty, &[(WRAP.len(), WRAP.len() + 1, 1)]);
    assert!(stmts_of(&empty).is_empty());
}

#[test]
fn map_statements_uses_the_innermost_covering_range() {
    let src = "function f(x) {\n  const a = 1;\n  if (x) {\n    return a;\n  }\n  return 2;\n}";
    let p = vpath("map-statements.js");
    register_meta(&p, &identity_wrapped(src), src);
    let ranges = vec![
        (at(src, "function", 0), WRAP.len() + src.len(), 2),
        (
            at(src, "{\n    return", 0),
            at(src, "  }\n  return 2", 3),
            0,
        ),
        (at(src, "  return 2", 0), at_last(src, "}", 0), -3),
    ];
    map_statements(&p, &ranges);
    // const a (2), if (3), return a (4), return 2 (6)
    assert_eq!(
        stmts_of(&p),
        BTreeMap::from([(0, (2, 2)), (1, (3, 2)), (2, (4, 0)), (3, (6, 0))])
    );
    // merging keeps the max count
    map_statements(&p, &[(at(src, "function", 0), WRAP.len() + src.len(), 7)]);
    let s = stmts_of(&p);
    assert_eq!((s[&0].1, s[&2].1, s[&3].1), (7, 7, 7));
}

#[test]
fn top_level_code_takes_the_module_range_that_opens_on_the_wrapper_line() {
    // V8 counts top-level code only in the module-wide ranges (the script and the CJS wrapper
    // function), which start on wrapper line 0 and end past the source map comment — neither end
    // has a source position of its own.
    let src = "const a = 0;\nif (a) {\n  g();\n}\nfunction g() {\n  return a;\n}\nfunction h() {\n  return 0;\n}";
    let p = vpath("top-level.js");
    let w = identity_wrapped(src);
    register_meta(&p, &w, src);
    let meta = meta_for(&p).unwrap();
    let last_line = WRAP.len() + src.rfind('\n').unwrap() + 1;
    assert_eq!(meta.code_span, Some((WRAP.len(), last_line)));
    let ranges = vec![
        (0, w.len(), 1),                                          // the script
        (1, w.len() - 1, 1),                                      // the wrapper function
        (at(src, "{\n  g()", 0), at(src, "}\nfunction g", 1), 0), // the `if` never fires
        (at(src, "function g", 0), at(src, "}\nfunction h", 1), 0),
        (at(src, "function h", 0), WRAP.len() + src.len(), 0),
    ];
    map_statements(&p, &ranges);
    // const a (1), if (2), g() (3), return a (6), return 0 (9)
    assert_eq!(
        stmts_of(&p),
        BTreeMap::from([
            (0, (1, 1)),
            (1, (2, 1)),
            (2, (3, 0)),
            (3, (6, 0)),
            (4, (9, 0))
        ])
    );
    // the top-level `if` is reached: then 0, else 1 (was "-" / "-": never reached)
    map_branches(&p, &ranges);
    assert_eq!(
        brs_of(&p),
        BTreeMap::from([((0, 0), (2, true, 0)), ((0, 1), (2, true, 1))])
    );
    // a range that ends before the first mapped position (a runtime helper) is still dropped
    let q = vpath("top-level-helper.js");
    register_meta(&q, &w, src);
    map_statements(&q, &[(0, WRAP.len() - 1, 9)]);
    assert!(stmts_of(&q).values().all(|&(_, c)| c == 0));
}

#[test]
fn innermost_range_prefers_the_narrower_of_two_with_the_same_start() {
    // a CommonJS function on line 1 column 0 starts where the module range does (both map to
    // (0, 0)): its own range, not the module's, must count its statements — whatever the order.
    let src = "function f() {\n  return 1;\n}\nmodule.exports = f;";
    let p = vpath("same-start.js");
    let w = identity_wrapped(src);
    register_meta(&p, &w, src);
    let f = (at(src, "function f", 0), at(src, "}\nmodule", 1));
    map_statements(&p, &[(f.0, f.1, 0), (0, w.len(), 1)]);
    assert_eq!(stmts_of(&p), BTreeMap::from([(0, (2, 0)), (1, (4, 1))]));
}

#[test]
fn recorders_merge_by_max_and_or() {
    let p = vpath("recorders.js");
    record(&p, 3, 2);
    record(&p, 3, 1);
    record(&p, 3, 5);
    assert_eq!(lines_of(&p), BTreeMap::from([(3, 5)]));
    record_fn(&p, 1, "f", 4);
    record_fn(&p, 1, "f", 2);
    assert_eq!(fns_of(&p), BTreeMap::from([((1, "f".to_string()), 4)]));
    record_branch(&p, 0, 1, 7, false, 0);
    record_branch(&p, 0, 1, 8, true, 3);
    record_branch(&p, 0, 1, 8, false, 1);
    assert_eq!(brs_of(&p), BTreeMap::from([((0, 1), (8, true, 3))]));
    record_stmt(&p, 0, 4, 6);
    record_stmt(&p, 0, 5, 2);
    assert_eq!(stmts_of(&p), BTreeMap::from([(0, (5, 6))]));
}

// ---- reporters --------------------------------------------------------------------------------

#[test]
fn row_percentages_treat_empty_metrics_as_full() {
    let r = row("x", 1, 4, 1, 3, 0, 2, 3, 3);
    assert_eq!((r.lpct(), r.bpct(), r.spct()), (25.0, 0.0, 100.0));
    assert!((r.fpct() - 100.0 / 3.0).abs() < 1e-12);
    let e = row("x", 0, 0, 0, 0, 0, 0, 0, 0);
    assert_eq!(
        (e.lpct(), e.fpct(), e.bpct(), e.spct()),
        (100.0, 100.0, 100.0, 100.0)
    );
}

#[test]
fn build_rows_renders_lcov_and_totals() {
    let a = PathBuf::from("/p/src/a.ts");
    let b = PathBuf::from("/p/src/b.ts");
    let a_lines: LineHits = HashMap::from([(3, 0), (1, 2), (2, 1)]);
    let b_lines: LineHits = HashMap::from([(1, 0)]);
    let fns: FnHits = HashMap::from([(
        a.clone(),
        HashMap::from([
            ((5, "z".to_string()), 0),
            ((1, "b".to_string()), 2),
            ((1, "a".to_string()), 1),
        ]),
    )]);
    let brs: BrHits = HashMap::from([(
        a.clone(),
        HashMap::from([
            ((1, 0), (4, false, 0)),
            ((0, 1), (2, true, 0)),
            ((0, 0), (2, true, 3)),
        ]),
    )]);
    let sts: StHits = HashMap::from([(a.clone(), HashMap::from([(0, (1, 1)), (1, (3, 0))]))]);
    let files = vec![(&a, &a_lines), (&b, &b_lines)];
    let (lcov, rows, total) = build_rows(&files, Some(&fns), Some(&brs), Some(&sts));
    assert_eq!(
        lcov,
        "SF:/p/src/a.ts\n\
FN:1,a\nFN:1,b\nFN:5,z\nFNDA:1,a\nFNDA:2,b\nFNDA:0,z\nFNF:3\nFNH:2\n\
BRDA:2,0,0,3\nBRDA:2,0,1,0\nBRDA:4,1,0,-\nBRF:3\nBRH:1\n\
DA:1,2\nDA:2,1\nDA:3,0\nLF:3\nLH:2\nend_of_record\n\
SF:/p/src/b.ts\nDA:1,0\nLF:1\nLH:0\nend_of_record\n"
    );
    assert_eq!(
        rows,
        vec![
            row("/p/src/a.ts", 2, 3, 2, 3, 1, 3, 1, 2),
            row("/p/src/b.ts", 0, 1, 0, 0, 0, 0, 0, 0)
        ]
    );
    assert_eq!(total, row("", 2, 4, 2, 3, 1, 3, 1, 2));
    // no function / branch / statement maps at all
    let (lcov, rows, _) = build_rows(&files[1..], None, None, None);
    assert_eq!(lcov, "SF:/p/src/b.ts\nDA:1,0\nLF:1\nLH:0\nend_of_record\n");
    assert_eq!(rows.len(), 1);
}

#[test]
fn text_summary_lists_files_totals_and_outputs() {
    let rows = vec![
        row("/p/src/a.ts", 2, 3, 1, 1, 1, 2, 3, 4),
        row("/other/b.ts", 0, 0, 0, 0, 0, 0, 0, 0),
    ];
    let total = row("", 2, 3, 1, 1, 1, 2, 3, 4);
    let outs = vec![
        "/p/coverage/lcov.info".to_string(),
        "/p/coverage/index.html".to_string(),
    ];
    assert_eq!(
        text_summary(&total, &rows, Path::new("/p"), &outs),
        "\n Coverage — 2 files (lines | funcs | branches | stmts)\n\
\x20  66.67% ln  100.00% fn   50.00% br   75.00% st   src/a.ts\n\
\x20 100.00% ln  100.00% fn  100.00% br  100.00% st   /other/b.ts\n\
\x20 ------\n\
\x20  66.67% lines (2/3)   100.00% fns (1/1)    50.00% branches (1/2)    75.00% stmts (3/4)\n\
\x20 → /p/coverage/lcov.info  /p/coverage/index.html\n"
    );
    // text-only reporter set: no output-files line
    let s = text_summary(&total, &rows, Path::new("/p"), &[]);
    assert!(s.ends_with("stmts (3/4)\n"), "{s}");
    assert!(!s.contains('→'));
}

#[test]
fn json_summary_matches_the_c8_shape() {
    let rows = vec![
        row("/p/a\"q\\.ts", 1, 3, 1, 2, 0, 0, 2, 3),
        row("/p/b.ts", 1, 1, 0, 0, 1, 1, 0, 0),
    ];
    let total = row("", 2, 4, 1, 2, 1, 1, 2, 3);
    let s = json_summary(&total, &rows);
    assert_eq!(
        s,
        "{\n  \"total\": {\"lines\":{\"total\":4,\"covered\":2,\"skipped\":0,\"pct\":50},\"statements\":{\"total\":3,\"covered\":2,\"skipped\":0,\"pct\":66.67},\"functions\":{\"total\":2,\"covered\":1,\"skipped\":0,\"pct\":50},\"branches\":{\"total\":1,\"covered\":1,\"skipped\":0,\"pct\":100}},\n\
\x20 \"/p/a\\\"q\\\\.ts\": {\"lines\":{\"total\":3,\"covered\":1,\"skipped\":0,\"pct\":33.33},\"statements\":{\"total\":3,\"covered\":2,\"skipped\":0,\"pct\":66.67},\"functions\":{\"total\":2,\"covered\":1,\"skipped\":0,\"pct\":50},\"branches\":{\"total\":0,\"covered\":0,\"skipped\":0,\"pct\":100}},\n\
\x20 \"/p/b.ts\": {\"lines\":{\"total\":1,\"covered\":1,\"skipped\":0,\"pct\":100},\"statements\":{\"total\":0,\"covered\":0,\"skipped\":0,\"pct\":100},\"functions\":{\"total\":0,\"covered\":0,\"skipped\":0,\"pct\":100},\"branches\":{\"total\":1,\"covered\":1,\"skipped\":0,\"pct\":100}}\n}\n"
    );
    // and it is valid JSON whose keys round-trip the escaped path
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["/p/a\"q\\.ts"]["lines"]["covered"], 1);
}

#[test]
fn json_summary_without_file_rows_is_valid_json() {
    // turbo-test#34 item 5: zero rows used to leave a trailing comma after the total block.
    let s = json_summary(&row("", 0, 0, 0, 0, 0, 0, 0, 0), &[]);
    assert_eq!(
        s,
        "{\n  \"total\": {\"lines\":{\"total\":0,\"covered\":0,\"skipped\":0,\"pct\":100},\"statements\":{\"total\":0,\"covered\":0,\"skipped\":0,\"pct\":100},\"functions\":{\"total\":0,\"covered\":0,\"skipped\":0,\"pct\":100},\"branches\":{\"total\":0,\"covered\":0,\"skipped\":0,\"pct\":100}}\n}\n"
    );
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v.as_object().unwrap().len(), 1);
}

#[test]
fn html_summary_classifies_and_escapes() {
    let rows = vec![
        row("/p/src/<x>&y.ts", 9, 10, 3, 4, 1, 2, 10, 10), // 90 high, 75 med, 50 low, 100 high
        row("/q/out.ts", 0, 0, 0, 0, 0, 0, 0, 0),
    ];
    let total = row("", 9, 10, 3, 4, 1, 2, 10, 10);
    let h = html_summary(&total, &rows, Path::new("/p"));
    assert!(h.starts_with("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>turbo-test coverage</title>"), "{h}");
    assert!(h.contains("<h1>Coverage — 2 files</h1>"), "{h}");
    assert!(
        h.contains(
            "<tr><td class=\"file\">src/&lt;x&gt;&amp;y.ts</td>\
<td class=\"high\">90.00% <span class=\"frac\">(9/10)</span></td>\
<td class=\"high\">100.00% <span class=\"frac\">(10/10)</span></td>\
<td class=\"med\">75.00% <span class=\"frac\">(3/4)</span></td>\
<td class=\"low\">50.00% <span class=\"frac\">(1/2)</span></td></tr>\n"
        ),
        "{h}"
    );
    assert!(h.contains("<td class=\"file\">/q/out.ts</td>"), "{h}");
    assert!(
        h.contains("<tfoot><tr><td class=\"file\">total</td><td class=\"high\">90.00%"),
        "{h}"
    );
    assert!(h.ends_with("</table></body></html>\n"), "{h}");
}

#[test]
fn short_path_strips_only_a_matching_prefix() {
    assert_eq!(
        short_path(Path::new("/p/src/a.ts"), Path::new("/p")),
        "src/a.ts"
    );
    assert_eq!(short_path(Path::new("/q/a.ts"), Path::new("/p")), "/q/a.ts");
}

// ---- inspector channel ------------------------------------------------------------------------

#[test]
fn channel_keeps_only_the_latest_response_and_ignores_notifications() {
    use v8::inspector::{ChannelImpl, StringBuffer, StringView};
    let resp = Rc::new(RefCell::new(None));
    let ch = CovChannel { resp: resp.clone() };
    let buf = |s: &'static str| StringBuffer::create(StringView::from(s.as_bytes()));
    ch.send_response(3, buf("{\"id\":3,\"result\":{}}"));
    assert_eq!(resp.borrow().as_deref(), Some("{\"id\":3,\"result\":{}}"));
    // notifications (e.g. Profiler.consoleProfileStarted) never clobber the pending response
    ch.send_notification(buf("{\"method\":\"Profiler.consoleProfileStarted\"}"));
    ch.flush_protocol_notifications();
    // nor does a null response buffer
    ch.send_response(4, v8::UniquePtr::default());
    assert_eq!(resp.borrow().as_deref(), Some("{\"id\":3,\"result\":{}}"));
}
