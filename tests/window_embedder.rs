//! The embedder's window members under a DOM environment (turbo-test#208): turbo-dom leaves
//! dialogs, window management, printing and `open` to the embedder, and jsdom (vitest's DOM)
//! has them, so `vi.spyOn(window, 'open')` works as on vitest 4.1.12.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

#[test]
fn a_dom_window_has_the_embedder_members() {
    let p = Project::new("window-embedder");
    p.file(
        "package.json",
        r#"{"name":"window-embedder","type":"module"}"#,
    );
    p.file(
        "vitest.config.ts",
        "export default { test: { environment: 'jsdom' } };\n",
    );
    p.file(
        "a.test.js",
        "import { expect, it, vi } from 'vitest';\n\
         const names = ['alert', 'blur', 'close', 'confirm', 'focus', 'moveBy', 'moveTo', 'open', 'print', 'prompt', 'resizeBy', 'resizeTo', 'scroll', 'scrollBy', 'stop', 'captureEvents', 'releaseEvents', 'postMessage'];\n\
         it('members', () => {\n\
           expect(names.filter((n) => typeof window[n] !== 'function')).toEqual([]);\n\
           expect([window.alert('x'), window.confirm('x'), window.prompt('x'), window.open('/x'), window.print()]).toEqual([undefined, undefined, undefined, undefined, undefined]);\n\
           expect(Object.keys(window).includes('open')).toBe(false);\n\
           const spy = vi.spyOn(window, 'open').mockReturnValue(null);\n\
           expect(window.open('/y')).toBe(null);\n\
           expect(spy).toHaveBeenCalledWith('/y');\n\
           spy.mockRestore();\n\
         });\n\
         it('postMessage dispatches a message event', async () => {\n\
           const got = new Promise((r) => window.addEventListener('message', (e) => r(e.data), { once: true }));\n\
           window.postMessage({ a: 1 }, '*');\n\
           expect(await got).toEqual({ a: 1 });\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert!(r.stdout.contains("Tests  2 passed (2)"), "{}", r.all());
}

/// turbo-test#210: under `environment: 'jsdom'`, getComputedStyle reports the cascaded declared
/// value of a property (the last matching rule, the inline style), and a shorthand built from
/// them, as jsdom does: vitest 4.1.12 + jsdom passes this file. Under `turbo-dom`, Chrome's
/// resolved values stay (`14px`, `700`).
#[test]
fn jsdom_computed_style_reports_declared_values() {
    let p = Project::new("jsdom-computed-style");
    p.file(
        "package.json",
        r#"{"name":"jsdom-computed-style","type":"module"}"#,
    );
    p.file(
        "vitest.config.ts",
        "export default { test: { environment: 'jsdom' } };\n",
    );
    p.file("a.test.js", "import { expect, it } from 'vitest';\nit('jsdom computed style', () => {\n  const s = document.createElement('style');\n  s.textContent = '.a{font-size:1.25rem;padding:10px 18px 10px 12px;font-weight:bold;text-decoration:underline;color:#ff0000}'\n    + '.a.b{font-size:2rem;color:blue}'\n    + '@media (max-width: 1px){.a{font-size:9px}}';\n  document.head.append(s);\n  const el = document.createElement('div');\n  el.className = 'a b';\n  el.style.marginTop = '10px';\n  el.style.fontSize = 'calc(1em * 0.875)';\n  document.body.append(el);\n  const cs = getComputedStyle(el);\n  expect([cs.fontSize, cs.getPropertyValue('font-size'), cs.padding, cs.paddingTop, cs.fontWeight, cs.textDecoration.split(' ')[0], cs.color, cs.marginTop, cs.display]).toEqual([\n    el.style.fontSize, el.style.fontSize, '10px 18px 10px 12px', '10px', 'bold', 'underline', 'rgb(0, 0, 255)', '10px', 'block',\n  ]);\n});\n");
    p.file(
        "b.test.js",
        "// @vitest-environment turbo-dom\nimport { expect, it } from 'vitest';\nit('chrome', () => { const el = document.createElement('div'); el.style.fontSize = '1.25rem'; el.style.fontWeight = 'bold'; document.body.append(el); expect([getComputedStyle(el).fontSize, getComputedStyle(el).fontWeight]).toEqual(['20px', '700']); });\n",
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert!(r.stdout.contains("Tests  2 passed (2)"), "{}", r.all());
}

/// turbo-test#210: jsdom applies an `@media` block to getComputedStyle only for a plain
/// screen / all media list, never a feature query (vitest 4.1.12 + jsdom: 1 passed).
#[test]
fn jsdom_computed_style_skips_feature_media_queries() {
    let p = Project::new("jsdom-computed-media");
    p.file(
        "package.json",
        r#"{"name":"jsdom-computed-media","type":"module"}"#,
    );
    p.file(
        "vitest.config.ts",
        "export default { test: { environment: 'jsdom' } };\n",
    );
    p.file(
        "a.test.js",
        "import { expect, it } from 'vitest';\n\
         it('media', () => {\n\
           const s = document.createElement('style');\n\
           s.textContent = '.m{display:flex}@media (min-width:0px){.m{display:none}}@media screen{.m{color:red}}@media print{.m{color:blue}}';\n\
           document.head.append(s);\n\
           const el = document.createElement('div'); el.className = 'm'; document.body.append(el);\n\
           expect([getComputedStyle(el).display, getComputedStyle(el).color]).toEqual(['flex', 'rgb(255, 0, 0)']);\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert!(r.stdout.contains("Tests  1 passed (1)"), "{}", r.all());
}

/// turbo-test#210: jsdom's cascade walks the style sheets in the order they were created or last
/// re-parsed, not in tree order (emotion inserts its `<style>` tags before earlier ones):
/// vitest 4.1.12 + jsdom gives `2px` here.
#[test]
fn jsdom_computed_style_orders_sheets_by_insertion() {
    let p = Project::new("jsdom-computed-order");
    p.file(
        "package.json",
        r#"{"name":"jsdom-computed-order","type":"module"}"#,
    );
    p.file(
        "vitest.config.ts",
        "export default { test: { environment: 'jsdom' } };\n",
    );
    p.file(
        "a.test.js",
        "import { expect, it } from 'vitest';\n\
         it('order', () => {\n\
           const x = document.createElement('div'); x.className = 'x'; document.body.append(x);\n\
           const a = document.createElement('style'); a.textContent = '.x{padding-left:1px}'; document.head.append(a);\n\
           const b = document.createElement('style'); b.textContent = '.x{padding-left:2px}'; document.head.insertBefore(b, a);\n\
           expect(getComputedStyle(x).paddingLeft).toBe('2px');\n\
           a.textContent = '.x{padding-left:3px}';\n\
           expect(getComputedStyle(x).paddingLeft).toBe('3px');\n\
         });\n",
    );
    let r = turbo_test(&p.dir, &[], &[]);
    assert!(r.stdout.contains("Tests  1 passed (1)"), "{}", r.all());
}
