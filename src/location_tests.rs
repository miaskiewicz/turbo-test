use super::*;

#[test]
fn call_positions_follow_v8() {
    // a plain call: the callee; a member call: the property; a call of a call: its `(` (also on a
    // later line, and after type arguments); columns count UTF-16 units
    let src = "test('a', () => {})\n  it.skip('b')\ntest.each([1])('c')\n;(f)<T>  ('d')\nconst é = 1; g()\nx.each([\n])('e')\n";
    assert_eq!(
        call_positions(Path::new("a.ts"), src),
        [
            (1, 1),
            (2, 6),
            (3, 6),
            (3, 15),
            (4, 10),
            (5, 14),
            (6, 3),
            (7, 3)
        ]
    );
    // unparsable source: no calls
    assert_eq!(
        call_positions(Path::new("a.js"), "test(("),
        Vec::<(u32, u32)>::new()
    );
}

#[test]
fn resolve_maps_snaps_and_offsets() {
    let f = Path::new("/p/a.test.ts");
    let name = "/p/a.test.ts";
    // no entry / another file / an entry without a map or identity: nothing
    ENTRY.with(|e| *e.borrow_mut() = None);
    assert_eq!(resolve(name, 1, 1), None);
    begin(f, "");
    assert_eq!(resolve("/p/other.ts", 1, 1), None);
    assert_eq!(resolve(name, 1, 1), None);
    // identity, below a one-line wrapper: line 2 is the code's line 1; the wrapper line has none
    set_identity(f);
    set_offset(f, 1);
    assert_eq!(resolve(name, 3, 4), Some((2, 4)));
    assert_eq!(resolve(name, 1, 1), None);
    // a map: generated line 1 col 10 → the last segment at or before it (src 1:0) → snapped to the
    // first call at or after 2:1, i.e. `it`'s property `skip` at 2:6; a line whose first segment
    // starts later than the column uses that segment; a line past the map has none
    let original = "\n  it.skip('x')\n";
    set_map(
        f,
        vec![vec![(2, 1, 0), (20, 1, 5)], vec![(5, 1, 5)]],
        original,
    );
    assert_eq!(resolve(name, 2, 11), Some((2, 6)));
    assert_eq!(resolve(name, 9, 1), None);
    // a column before the line's first segment (src 1:5 → 2:6) belongs to the last call before
    // it (none here: the mapped point stays), then to `skip` once the segment points past it
    assert_eq!(resolve(name, 3, 1), Some((2, 6)));
    set_map(f, vec![vec![], vec![(5, 1, 9)]], original);
    assert_eq!(resolve(name, 3, 1), Some((2, 6)));
    // a mapped point past every call keeps the mapped position
    set_map(f, vec![vec![(0, 5, 2)]], original);
    assert_eq!(resolve(name, 2, 1), Some((6, 3)));
    // a map for another file is ignored
    set_map(Path::new("/p/other.ts"), vec![], original);
    assert_eq!(resolve(name, 2, 1), Some((6, 3)));
}

#[test]
fn the_entry_needs_call_sites_for_locations_or_inline_snapshots() {
    // without TURBO_TASK_LOCATION (unset in unit tests) only a file using inline snapshots
    let f = Path::new("/p/b.test.js");
    begin(f, "expect(1).toMatchInlineSnapshot(`1`)");
    assert!(is_entry(f) && !is_entry(Path::new("/p/c.test.js")));
    begin(f, "expect(1).toBe(1)");
    assert!(!is_entry(f));
    ENTRY.with(|e| *e.borrow_mut() = None);
    assert!(!is_entry(f));
}

#[test]
fn shifts_map_columns_back_to_before_the_edits() {
    let p = "/tmp/tt-shift-module.js";
    let s = |line, new, old| Shift { line, new, old };
    set_shifts(
        Path::new(p),
        vec![s(1, (4, 10), (4, 5)), s(1, (12, 20), (7, 8))],
    );
    // 1-based in and out; line 2 is shift line 1
    assert_eq!(
        [1, 4, 5, 6, 10, 11, 12, 13, 15, 21, 30].map(|c| unshift(p, 2, c)),
        [1, 4, 5, 5, 5, 6, 7, 8, 8, 9, 18]
    );
    assert_eq!(unshift(p, 1, 9), 9);
    assert_eq!(unshift("/elsewhere.js", 2, 9), 9);
    set_shifts(Path::new(p), Vec::new());
    assert_eq!(unshift(p, 2, 9), 9);
}
