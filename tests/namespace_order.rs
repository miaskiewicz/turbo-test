//! An ES module's namespace as vitest gives it (turbo-test#141): its names in source order and
//! `Symbol.toStringTag` `'Module'` (vite's SSR transform defines the exports as the module
//! declares them; a package's ES module, imported through Node, has Node's sorted names,
//! turbo-test#159), on the module runner (native emit and esbuild) and the ESM graph loader.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

fn project(name: &str, esbuild: bool) -> Project {
    let p = Project::new(name);
    p.file("package.json", "{\"type\":\"module\"}\n");
    p.file(
        "dep.ts",
        "export const b = 1;\nexport const a = 2;\nexport default 3;\nexport function c() {}\n",
    );
    // a package's ES module: vitest imports it through Node, whose namespace is sorted (#159)
    p.file(
        "node_modules/pesm/package.json",
        "{\"name\":\"pesm\",\"type\":\"module\",\"main\":\"index.js\"}\n",
    );
    p.file(
        "node_modules/pesm/index.js",
        "export const b = 1;\nexport const a = 2;\nexport default 3;\nexport function c() {}\n",
    );
    p.file(
        "ns.test.ts",
        r#"import { expect, it } from 'vitest';
import * as dep from './dep';
import * as pesm from 'pesm';

it('static namespace', () => {
  expect(Object.keys(dep)).toEqual(['b', 'a', 'default', 'c']);
  expect(Object.prototype.toString.call(dep)).toBe('[object Module]');
  expect(dep.a).toBe(2);
});
it("a package's namespace", async () => {
  expect(Object.keys(pesm)).toEqual(['a', 'b', 'c', 'default']);
  expect(Object.prototype.toString.call(pesm)).toBe('[object Module]');
  expect(Object.keys(await import('pesm'))).toEqual(['a', 'b', 'c', 'default']);
});
it('import() namespace', async () => {
  const ns = await import('./dep');
  expect(Object.keys(ns)).toEqual(['b', 'a', 'default', 'c']);
  expect(Object.prototype.toString.call(ns)).toBe('[object Module]');
});
"#,
    );
    if esbuild {
        p.with_esbuild();
    }
    p
}

fn assert_passes(p: &Project, env: &[(&str, &str)]) {
    let r = turbo_test(&p.dir, &["ns.test.ts"], env);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
#[ignore = "turbo-test#159: fails until the fix"]
fn namespace_order_and_tag_on_the_module_runner() {
    assert_passes(&project("ns-order-mr", true), &[]);
}

#[test]
#[ignore = "turbo-test#159: fails until the fix"]
fn namespace_order_and_tag_on_the_module_runner_with_esbuild() {
    assert_passes(
        &project("ns-order-esbuild", true),
        &[("TURBO_NATIVE_CJS", "0")],
    );
}

#[test]
fn namespace_order_and_tag_on_the_esm_graph_loader() {
    assert_passes(&project("ns-order-graph", false), &[]);
}
