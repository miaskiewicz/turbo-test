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
