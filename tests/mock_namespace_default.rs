//! A `vi.mock` factory without `default`: reading the module namespace's `default`
//! (`import * as ns` then `ns.default`, `(await import(x)).default`) throws vitest's
//! `[vitest] No "default" export is defined on the "x" mock…` (turbo-test#158), on the module
//! runner and the ESM graph loader, for an app module and a package's CommonJS module.
#![cfg(unix)]

mod common;
use common::{turbo_test, Project};

fn project(name: &str, esbuild: bool) -> Project {
    let p = Project::new(name);
    p.file("package.json", "{\"type\":\"module\"}\n");
    p.file("dep.ts", "export default 1;\nexport const named = 'real';\n");
    p.file("node_modules/pkg/package.json", "{\"name\":\"pkg\",\"main\":\"index.js\"}\n");
    p.file("node_modules/pkg/index.js", "module.exports = { named: 'real' };\n");
    p.file(
        "ns.test.ts",
        r#"import { expect, it, vi } from 'vitest';
import * as dep from './dep';
import * as pkg from 'pkg';

vi.mock('./dep', () => ({ named: 'only' }));
vi.mock('pkg', () => ({ named: 'only' }));

const msg = (m: string) =>
  `[vitest] No "default" export is defined on the "${m}" mock. Did you forget to return it from "vi.mock"?`;

it('static namespace', () => {
  expect(dep.named).toBe('only');
  expect(() => typeof dep.default).toThrow(msg('./dep'));
  expect(pkg.named).toBe('only');
  expect(() => typeof (pkg as any).default).toThrow(msg('pkg'));
});
it('import() namespace', async () => {
  const d = await import('./dep');
  expect(d.named).toBe('only');
  expect(() => typeof d.default).toThrow(msg('./dep'));
  const p: any = await import('pkg');
  expect(() => typeof p.default).toThrow(msg('pkg'));
});
"#,
    );
    if esbuild {
        p.with_esbuild();
    }
    p
}

fn assert_passes(p: &Project) {
    let r = turbo_test(&p.dir, &["ns.test.ts"], &[]);
    assert_eq!(r.code, Some(0), "{}", r.all());
}

#[test]
fn mock_namespace_default_throws_on_the_module_runner() {
    assert_passes(&project("mock-ns-default-mr", true));
}

#[test]
fn mock_namespace_default_throws_on_the_esm_graph_loader() {
    assert_passes(&project("mock-ns-default-graph", false));
}
