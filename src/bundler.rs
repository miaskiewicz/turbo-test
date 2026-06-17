//! Native package bundler (P2b) — the Rust replacement for `esbuild --bundle --format=cjs
//! --packages=external` on a node_modules entry (`runner.rs::esbuild_bundle_dep_cjs`).
//!
//! Why bundle at all (vs the per-file path used for app code): a package's OWN relative files form
//! a graph with circular imports + init ordering that per-file `require` can't reproduce (the P2b
//! naive attempt broke MUI/emotion). esbuild solves it by bundling each package with lazy init
//! wrappers. We do the same:
//!
//!   - collect the package's transitive RELATIVE import graph (bare imports stay external → one
//!     shared instance via the require cache, exactly like `--packages=external`);
//!   - transform each module ESM→CJS via `esm_cjs::transform_to_parts`, rewriting relative
//!     imports/`require`s to `require_<id>()` calls into the other bundled modules;
//!   - wrap each module in `__commonJS((exports, module) => { ... })` — lazy, run-once. Because each
//!     module sets `module.exports = __toCommonJS(__tt_exports)` EARLY (live getters), a re-entrant
//!     `require_<id>()` during a cycle sees the partially-filled live exports object. This is
//!     esbuild's circular-safety mechanism, reusing our proven per-file emit verbatim inside the
//!     closure.
//!   - non-JS asset imports (`.css`, `.svg`, …) are stubbed (esbuild's `--loader:.css=empty` etc.).
//!
//! Returns `None` (→ caller falls back to the esbuild bundle) on any resolve/parse failure or an
//! oversized graph.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::esm_cjs::{self, LowerCtx, SourceRef};

/// Hard cap on bundled modules — a runaway graph bails to esbuild rather than ballooning.
const MAX_MODULES: usize = 4000;

const ASSET_EMPTY: [&str; 5] = ["css", "scss", "sass", "less", "styl"];
const ASSET_STRING: [&str; 6] = ["svg", "png", "jpg", "jpeg", "gif", "webp"];

fn is_relative(spec: &str) -> bool {
    spec.starts_with("./") || spec.starts_with("../")
}

fn asset_stub(path: &Path) -> Option<&'static str> {
    let ext = path.extension().and_then(|e| e.to_str())?;
    if ASSET_EMPTY.contains(&ext) {
        Some("module.exports = {};")
    } else if ASSET_STRING.contains(&ext) {
        Some("module.exports = \"\";")
    } else {
        None
    }
}

/// Resolver for relative specifiers within a package (extension + index resolution). Bare imports
/// are never passed here (they stay external).
fn make_resolver() -> oxc_resolver::Resolver {
    let strs = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    oxc_resolver::Resolver::new(oxc_resolver::ResolveOptions {
        extensions: strs(&[".js", ".mjs", ".cjs", ".jsx", ".ts", ".tsx", ".json", ".node"]),
        main_fields: strs(&["module", "browser", "main"]),
        condition_names: strs(&["import", "module", "browser", "default", "require"]),
        ..Default::default()
    })
}

/// Resolve a relative specifier from `importer`'s directory to an absolute file path.
fn resolve_rel(resolver: &oxc_resolver::Resolver, importer: &Path, spec: &str) -> Option<PathBuf> {
    let dir = importer.parent()?;
    resolver.resolve(dir, spec).ok().map(|r| r.full_path())
}

/// Collect static relative specifiers from a module's (TS-stripped) JS: import/export-from/`export
/// *` sources + `require("…")` literals. Dynamic `import()` is intentionally excluded (left to
/// runtime require, not bundled).
struct SpecCollector {
    specs: Vec<String>,
}

impl<'a> Visit<'a> for SpecCollector {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        self.specs.push(it.source.value.to_string());
        walk::walk_import_declaration(self, it);
    }
    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        if let Some(src) = &it.source {
            self.specs.push(src.value.to_string());
        }
        walk::walk_export_named_declaration(self, it);
    }
    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        self.specs.push(it.source.value.to_string());
        walk::walk_export_all_declaration(self, it);
    }
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if it.arguments.len() == 1 {
            if let Expression::Identifier(callee) = &it.callee {
                if callee.name == "require" {
                    if let Some(Argument::StringLiteral(s)) = it.arguments.first() {
                        self.specs.push(s.value.to_string());
                    }
                }
            }
        }
        walk::walk_call_expression(self, it);
    }
}

fn collect_relative_specs(js: &str) -> Vec<String> {
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, js, SourceType::mjs()).parse();
    let mut c = SpecCollector { specs: Vec::new() };
    c.visit_program(&parsed.program);
    c.specs.into_iter().filter(|s| is_relative(s)).collect()
}

struct Module {
    /// the (TS-stripped) JS, or `None` for an asset stub.
    js: Option<String>,
    /// stub body for an asset module.
    stub: Option<&'static str>,
    /// relative-import specifier → resolved absolute path (→ bundled-module id in pass 2).
    edges: HashMap<String, PathBuf>,
}

/// Bundle a node_modules package starting at `entry`. `None` → fall back to the esbuild bundle.
pub fn bundle(entry: &Path) -> Option<String> {
    let entry = entry.canonicalize().ok()?;
    let resolver = make_resolver();

    let mut order: Vec<PathBuf> = Vec::new();
    let mut id_of: HashMap<PathBuf, usize> = HashMap::new();
    let mut modules: Vec<Module> = Vec::new();

    // BFS the relative import graph. Store each module's JS + its spec→path edges; ids resolve in
    // pass 2 (targets may not be assigned an id until later in the walk).
    let mut queue: Vec<PathBuf> = vec![entry.clone()];
    while let Some(p) = queue.pop() {
        if id_of.contains_key(&p) {
            continue;
        }
        if order.len() >= MAX_MODULES {
            return None;
        }
        id_of.insert(p.clone(), order.len());
        order.push(p.clone());

        if let Some(stub) = asset_stub(&p) {
            modules.push(Module { js: None, stub: Some(stub), edges: HashMap::new() });
            continue;
        }

        let raw = std::fs::read_to_string(&p).ok()?;
        let js = if crate::transform::needs_transform(&p) {
            crate::transform::transform(&p, &raw).ok()?
        } else {
            raw
        };

        let mut edges = HashMap::new();
        for spec in collect_relative_specs(&js) {
            if let Some(abs) = resolve_rel(&resolver, &p, &spec) {
                if !id_of.contains_key(&abs) {
                    queue.push(abs.clone());
                }
                edges.insert(spec, abs);
            }
            // an unresolvable relative spec stays `External` (runtime require) — rare; harness gates.
        }
        modules.push(Module { js: Some(js), stub: None, edges });
    }

    // Pass 2: transform + wrap each module.
    let mut out = String::from(esm_cjs::PREAMBLE);
    for (idx, module) in modules.iter().enumerate() {
        let module_text = if let Some(stub) = module.stub {
            stub.to_string()
        } else {
            let js = module.js.as_ref()?;
            let edges = &module.edges;
            // relative spec → bundled module (require_<id>()); everything else external.
            let resolve = |spec: &str| -> SourceRef {
                if let Some(abs) = edges.get(spec) {
                    if let Some(&tid) = id_of.get(abs) {
                        return SourceRef::BundledCjs { id: format!("m{tid}") };
                    }
                }
                SourceRef::External
            };
            let ctx = LowerCtx { exports_obj: "__tt_exports".to_string(), resolve: Some(&resolve) };
            let parts = esm_cjs::transform_to_parts(js, &ctx)?;
            if parts.has_module_syntax {
                esm_cjs::assemble_module(&parts.requires, &parts.re_exports, &parts.exports, &parts.body_code)
            } else {
                parts.body_code
            }
        };
        out.push_str(&format!("var require_m{idx} = __commonJS((exports, module) => {{\n{module_text}\n}});\n"));
    }
    let entry_id = *id_of.get(&entry)?;
    out.push_str(&format!("module.exports = require_m{entry_id}();\n"));
    Some(out)
}
