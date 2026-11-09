//! `test.scoped({...})` / `test.override({...})` (turbo-test#276): vitest 4's per-describe
//! fixture overrides on a `test = base.extend({...})`. Checks the issue's probe passes, a nested
//! describe inherits its outer scope and may override it, overrides don't bleed between parallel
//! describes, keys not in the base extend are passed through, overrides called as fixture
//! functions resolve like plain extend fixtures, and `scoped` outside a describe applies to the
//! whole file (mirrors real vitest 4.1.12).
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

fn assert_passes(r: &common::Run, n: u32) {
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains(&format!("Tests  {n} passed ({n})")),
        "{}",
        r.all()
    );
}

fn project_with(src: &str) -> Project {
    let p = Project::new("rc-scoped");
    p.file("package.json", r#"{"name":"rc-scoped","type":"module"}"#);
    p.file("a.test.js", src);
    p
}

#[test]
fn scoped_basic_probe_passes() {
    // The exact probe from turbo-test#276.
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red' });
describe('s', () => {
  test.scoped({ color: 'blue' });
  test('b', ({ color }) => expect(color).toBe('blue'));
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn scoped_inherits_through_nested_describe_and_inner_overrides_outer() {
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red', size: 'small' });
test('default outside', ({ color, size }) => {
  expect(color).toBe('red');
  expect(size).toBe('small');
});
describe('outer', () => {
  test.scoped({ color: 'blue' });
  test('outer sees blue', ({ color, size }) => {
    expect(color).toBe('blue');
    expect(size).toBe('small');
  });
  describe('inner-plain', () => {
    test('inherits blue', ({ color, size }) => {
      expect(color).toBe('blue');
      expect(size).toBe('small');
    });
  });
  describe('inner-scoped', () => {
    test.scoped({ size: 'big' });
    test('blue + big', ({ color, size }) => {
      expect(color).toBe('blue');
      expect(size).toBe('big');
    });
  });
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 4);
}

#[test]
fn scoped_does_not_bleed_between_sibling_describes() {
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red' });
describe('scoped here', () => {
  test.scoped({ color: 'blue' });
  test('blue', ({ color }) => expect(color).toBe('blue'));
});
describe('sibling with no scope', () => {
  test('still red', ({ color }) => expect(color).toBe('red'));
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 2);
}

#[test]
fn scoped_accepts_keys_not_in_base_extend() {
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red' });
describe('adds extra', () => {
  test.scoped({ extra: 42 });
  test('extra arrives, color stays', ({ color, extra }) => {
    expect(color).toBe('red');
    expect(extra).toBe(42);
  });
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn scoped_resolves_fixture_functions() {
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red' });
describe('fn override', () => {
  test.scoped({ color: async ({}, use) => { await use('fn-value'); } });
  test('fn used', ({ color }) => expect(color).toBe('fn-value'));
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn override_is_scoped_without_the_deprecation() {
    // `test.override` is the non-deprecated name for the same operation (vitest 4).
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red' });
describe('override', () => {
  test.override({ color: 'purple' });
  test('purple', ({ color }) => expect(color).toBe('purple'));
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn scoped_at_top_level_applies_to_whole_file() {
    // Mirrors real vitest 4.1.12: a top-level test.scoped scopes to the file's root suite,
    // so tests inside AND outside describes see the override.
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red' });
test.scoped({ color: 'top' });
test('top-level is top', ({ color }) => expect(color).toBe('top'));
describe('inside', () => {
  test('inherits top', ({ color }) => expect(color).toBe('top'));
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 2);
}

#[test]
fn last_scoped_call_wins_within_one_describe() {
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red' });
describe('two scoped calls', () => {
  test.scoped({ color: 'green' });
  test.scoped({ color: 'yellow' });
  test('last wins', ({ color }) => expect(color).toBe('yellow'));
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn successive_scoped_calls_in_one_describe_layer_different_keys() {
    // Two scoped calls in the same describe that override DIFFERENT keys must both take
    // effect: the baseline for the second call is the first call's merged overrides, not
    // the parent's (mirrors real vitest 4.1.12).
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const test = base.extend({ color: 'red', size: 'small' });
describe('layered scoped calls', () => {
  test.scoped({ color: 'blue' });
  test.scoped({ size: 'big' });
  test('both apply', ({ color, size }) => {
    expect(color).toBe('blue');
    expect(size).toBe('big');
  });
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 1);
}

#[test]
fn scoped_on_two_chained_extends_in_one_describe_keeps_them_separate() {
    // Pins the WeakMap-of-Map keying for `test.scoped` / `test.override`: two distinct
    // `.extend`-returned helpers in the same describe each have their own overrides on the
    // same suite, keyed by the `ext` function identity. A refactor that keys the inner map
    // by something coarser (the suite alone) would conflate them.
    let p = project_with(
        r#"
import { describe, expect, test as base } from 'vitest';
const a = base.extend({ x: 'a0' });
const b = a.extend({ y: 'b0' });
describe('two chained extends', () => {
  a.scoped({ x: 'a-scoped' });
  b.scoped({ y: 'b-scoped' });
  a('a sees its scoped', ({ x }) => expect(x).toBe('a-scoped'));
  b('b sees both', ({ x, y }) => {
    expect(x).toBe('a0');
    expect(y).toBe('b-scoped');
  });
});
"#,
    );
    assert_passes(&turbo_test(&p.dir, &["a.test.js"], &[]), 2);
}
