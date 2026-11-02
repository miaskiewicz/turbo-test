//! Unit tests for the native package bundler: graph walk (cycles, duplicates, assets, TS, bare and
//! unresolvable specifiers), the per-module `__commonJS` wrapping and every bail-out path.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A fresh scratch package directory (outside the repo) with `files` written into it.
fn pkg(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join("turbo-test-bundler-ut")
        .join(format!("{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // stale dir from a recycled pid
    for (rel, src) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    dir.canonicalize().unwrap()
}

/// Split a bundle into its module bodies, indexed by id (`var require_m<id> = __commonJS(…)`).
fn modules_of(out: &str) -> Vec<String> {
    let mut mods = Vec::new();
    for (i, chunk) in out.split("var require_m").skip(1).enumerate() {
        let (id, rest) = chunk
            .split_once(" = __commonJS((exports, module) => {\n")
            .unwrap();
        assert_eq!(id, i.to_string(), "ids are dense and in order");
        let body = rest.rsplit_once("\n});\n").unwrap().0;
        mods.push(body.to_string());
    }
    mods
}

/// Id of the single module whose body contains `marker`.
fn id_with(mods: &[String], marker: &str) -> usize {
    let hits: Vec<usize> = (0..mods.len())
        .filter(|&i| mods[i].contains(marker))
        .collect();
    assert_eq!(hits.len(), 1, "{marker:?} in modules {hits:?}:\n{mods:#?}");
    hits[0]
}

#[test]
fn is_relative_accepts_dot_paths_only() {
    for s in ["./a", "../a", ".", "..", "./"] {
        assert!(is_relative(s), "{s}");
    }
    for s in ["a", "@s/a", "/abs", ".hidden", "..x", "node:fs"] {
        assert!(!is_relative(s), "{s}");
    }
}

#[test]
fn asset_stub_by_extension() {
    for f in ["a.css", "a.scss", "a.sass", "a.less", "a.styl"] {
        assert_eq!(
            asset_stub(Path::new(f)),
            Some("module.exports = {};"),
            "{f}"
        );
    }
    for f in ["a.svg", "a.png", "a.jpg", "a.jpeg", "a.gif", "a.webp"] {
        assert_eq!(
            asset_stub(Path::new(f)),
            Some("module.exports = \"\";"),
            "{f}"
        );
    }
    for f in ["a.js", "a.json", "README"] {
        assert_eq!(asset_stub(Path::new(f)), None, "{f}");
    }
}

#[test]
fn collect_relative_specs_finds_static_sources_only() {
    let specs = collect_relative_specs(
        "import a from './a'; import 'react'; export { b } from './b'; export * from '../c';\n\
         export const k = 1; const d = require('./d'); require('fs'); require(x); require('./e', 1);\n\
         import('./dyn');",
    );
    assert_eq!(specs, vec!["./a", "./b", "../c", "./d"]);
}

#[test]
fn bundles_a_cyclic_esm_graph_with_assets_ts_and_cjs() {
    let dir = pkg(
        "graph",
        &[
            (
                "index.mjs",
                "import React from 'react';\nimport { a } from './a.js';\nimport './style.css';\n\
                 import logo from './logo.svg';\nimport { twice } from './util';\n\
                 export { q } from './q.js';\nexport * from './r.js';\n\
                 const c = require('./c.cjs');\nconst m = require('./missing');\n\
                 export const entry = [React, a, logo, twice(2), c, m, import('./dyn.js')];\n",
            ),
            ("a.js", "import { b } from './b.js';\nexport const a = 'A' + b;\n"),
            ("b.js", "import * as A from './a.js';\nexport const b = 'B';\nexport const viaA = () => A.a;\n"),
            ("util.ts", "export const twice = (n: number): number => n * 2;\n"),
            ("q.js", "export const q = 'Q';\n"),
            ("r.js", "export const r = 'R';\n"),
            ("c.cjs", "module.exports = { cjs: 'CJSMARK' };\n"),
            ("sub/up.js", "module.exports = require('..');\n"),
            ("style.css", "body { color: red }\n"),
            ("logo.svg", "<svg/>\n"),
            ("dyn.js", "export const dyn = 1;\n"),
        ],
    );
    let out = bundle(&dir.join("index.mjs")).expect("bundle");
    assert!(out.starts_with(esm_cjs::PREAMBLE), "{out}");
    assert!(out.ends_with("module.exports = require_m0();\n"), "{out}");
    let mods = modules_of(&out);
    // index, a, b, util.ts, q, r, c.cjs, style.css, logo.svg — dyn.js and sub/up.js are not walked.
    assert_eq!(mods.len(), 9, "{mods:#?}");

    let entry = id_with(&mods, "entry: () => entry");
    assert_eq!(entry, 0);
    let a = id_with(&mods, "const a = \"A\"");
    let b = id_with(&mods, "const b = \"B\"");
    let util = id_with(&mods, "const twice = (n) => n * 2");
    let q = id_with(&mods, "const q = \"Q\"");
    let r = id_with(&mods, "const r = \"R\"");
    let c = id_with(&mods, "CJSMARK");
    let css = id_with(&mods, "module.exports = {};");
    let svg = id_with(&mods, "module.exports = \"\";");

    let e = &mods[entry];
    assert!(
        e.contains("var import_react_1 = __toESM(require(\"react\"));"),
        "bare stays external: {e}"
    );
    assert!(e.contains(&format!("= require_m{a}();")), "{e}");
    assert!(e.contains(&format!("require_m{css}();")), "{e}");
    assert!(e.contains(&format!("= __toESM(require_m{svg}());")), "{e}");
    assert!(e.contains(&format!("= require_m{util}();")), "{e}");
    assert!(e.contains(&format!("= require_m{q}();")), "{e}");
    assert!(
        e.contains(&format!(
            "__reExport(__tt_exports, require_m{r}(), module.exports);"
        )),
        "{e}"
    );
    assert!(e.contains(&format!("const c = require_m{c}();")), "{e}");
    assert!(
        e.contains("const m = require(\"./missing\");"),
        "unresolvable stays a runtime require: {e}"
    );
    assert!(
        e.contains("Promise.resolve().then(() => __toESM(require(\"./dyn.js\")))"),
        "dynamic import not bundled: {e}"
    );

    // the a <-> b cycle points both ways at the bundled ids
    assert!(
        mods[a].contains(&format!("= require_m{b}();")),
        "{}",
        mods[a]
    );
    assert!(
        mods[b].contains(&format!("var A = __toESM(require_m{a}());")),
        "{}",
        mods[b]
    );
    // a plain CJS module is wrapped unchanged (no ESM preamble pieces)
    assert!(!mods[c].contains("__toCommonJS"), "{}", mods[c]);
    assert!(
        mods[c].contains("module.exports = { cjs: \"CJSMARK\" };"),
        "{}",
        mods[c]
    );
}

#[test]
fn dot_dot_require_inlines_the_package_index() {
    let dir = pkg(
        "dotdot",
        &[
            (
                "dist/utils/x.js",
                "const root = require('..');\nmodule.exports = () => root;\n",
            ),
            ("dist/index.js", "module.exports = { root: 'INDEXMARK' };\n"),
            ("dist/utils/y.js", "module.exports = require('.');\n"),
            ("dist/utils/index.js", "module.exports = 'utils index';\n"),
        ],
    );
    let out = bundle(&dir.join("dist/utils/x.js")).unwrap();
    let mods = modules_of(&out);
    assert_eq!(mods.len(), 2, "{mods:#?}");
    let idx = id_with(&mods, "INDEXMARK");
    assert!(
        mods[0].contains(&format!("const root = require_m{idx}();")),
        "{}",
        mods[0]
    );

    let out = bundle(&dir.join("dist/utils/y.js")).unwrap();
    let mods = modules_of(&out);
    let ui = id_with(&mods, "utils index");
    assert!(
        mods[0].contains(&format!("module.exports = require_m{ui}();")),
        "{}",
        mods[0]
    );
}

#[test]
fn a_module_reached_by_two_specifiers_is_bundled_once() {
    let dir = pkg(
        "dup",
        &[
            ("main.js", "const a = require('./z');\nconst b = require('./z.js');\nmodule.exports = [a, b];\n"),
            ("z.js", "module.exports = 'Z';\n"),
        ],
    );
    let out = bundle(&dir.join("main.js")).unwrap();
    let mods = modules_of(&out);
    assert_eq!(mods.len(), 2, "{mods:#?}");
    assert!(
        mods[0].contains("const a = require_m1();\nconst b = require_m1();"),
        "{}",
        mods[0]
    );
}

#[test]
fn module_cap_bails_to_esbuild() {
    let dir = pkg(
        "cap",
        &[
            ("a.js", "require('./b');\n"),
            ("b.js", "require('./c');\n"),
            ("c.js", "module.exports = 1;\n"),
        ],
    );
    assert!(
        bundle_capped(&dir.join("a.js"), 2).is_none(),
        "3 modules > cap 2"
    );
    let out = bundle_capped(&dir.join("a.js"), 3).expect("3 modules fit cap 3");
    assert_eq!(modules_of(&out).len(), 3);
    assert_eq!(
        bundle(&dir.join("a.js")),
        Some(out),
        "production cap admits it too"
    );
}

#[test]
fn bails_on_missing_entry_unreadable_module_and_bad_syntax() {
    let dir = pkg(
        "bad",
        &[
            ("bad.ts", "export const = ;\n"),
            ("bad.js", "export const x = ;\n"),
            ("imports-bad-ts.js", "import './bad.ts';\n"),
            ("imports-bad-js.js", "require('./bad.js');\n"),
            ("dir/keep.txt", "\n"),
        ],
    );
    assert!(bundle(&dir.join("nope.js")).is_none(), "missing entry");
    assert!(
        bundle(&dir.join("dir")).is_none(),
        "entry is a directory (unreadable as a file)"
    );
    assert!(
        bundle(&dir.join("bad.ts")).is_none(),
        "TS transform failure"
    );
    assert!(
        bundle(&dir.join("imports-bad-ts.js")).is_none(),
        "TS failure deeper in the graph"
    );
    assert!(
        bundle(&dir.join("imports-bad-js.js")).is_none(),
        "JS parse failure in a dependency"
    );
}
