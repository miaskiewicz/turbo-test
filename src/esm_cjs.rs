//! Native ESM→CJS emitter (oxc) — the Rust replacement for the per-file `esbuild --format=cjs`
//! transform (`runner.rs::esbuild_transform_cjs`). oxc 0.134 has NO CommonJS module transform
//! (its `Module::CommonJS` only rewrites TS `import x = require()`), so this hand-writes the
//! ESM→CJS lowering, targeting esbuild's output CONTRACT so the existing downstream consumers
//! (`hoist_mock_setup`, `shared_mock_lets`, the loader) keep working:
//!
//!   - imports → `var import_<src> = require("src")` (or `__toESM(require(...))` when the import
//!     binds a default or namespace), references to named/default locals rewritten to
//!     `import_<src>.<name>` so bindings stay LIVE (spyOn / vi.mock observe writes);
//!   - exports → an `__export(__tt_exports, { name: () => binding, ... })` getter block +
//!     `module.exports = __toCommonJS(__tt_exports)`, `export *` via `__reExport`.
//!
//! Helpers are emitted in their runner-friendly form directly (configurable getters; an
//! identity-preserving `__toESM` that just ensures `.default`) — i.e. what `postprocess_mr_cjs`
//! patched esbuild's output into — so NO postprocess is needed on native output.
//!
//! Pipeline: oxc TS/JSX strip (reuse `transform::transform`) → re-parse the plain JS → semantic
//! (for scope-correct reference resolution) → ESM→CJS visit_mut → codegen → prepend preamble +
//! append export registration.
//!
//! Gated behind `TURBO_NATIVE_CJS`; esbuild remains the default and the fallback until the
//! conformity harness shows parity on the oracle suites.

use std::collections::HashMap;
use std::path::Path;

use oxc_allocator::{Allocator, Box as ABox};
use oxc_ast::ast::*;
use oxc_ast::{AstBuilder, NONE};
use oxc_ast_visit::{walk_mut, VisitMut};
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder, SymbolId};
use oxc_span::{SourceType, SPAN};

/// Whether the native ESM→CJS emitter is enabled for app files. **Default ON** (P2a cutover): the
/// conformity harness validated full parity on the payroll oracle (1057 files / 10471 tests) and
/// esbuild remains the automatic fallback for any form the emitter doesn't handle, so native-on is
/// zero-regression. Opt OUT with `TURBO_NATIVE_CJS=0` (or empty) — the harness uses this to run the
/// esbuild baseline. (esbuild is still used for node_modules bundling, coverage, and
/// decorator-metadata regardless — see the gate in `read_transformed`.)
pub fn enabled() -> bool {
    match std::env::var("TURBO_NATIVE_CJS") {
        Ok(v) => v != "0" && !v.is_empty(),
        Err(_) => true,
    }
}

/// Strict mode (conformity coverage measurement): when on, the runner does NOT fall back to esbuild
/// for app files the native emitter can't handle — the unhandled module surfaces as a load error so
/// the harness can count true native coverage instead of it being silently masked by the fallback.
pub fn strict() -> bool {
    std::env::var("TURBO_NATIVE_CJS_STRICT").map(|v| !v.is_empty()).unwrap_or(false)
}

/// Whether to use the native bundler for **node_modules** packages (P2b). **Default ON** (opt out
/// via `TURBO_NATIVE_DEPS=0`). Uses `crate::bundler` to bundle a package's relative graph with lazy
/// `__commonJS` init wrappers (circular-safe) + asset stubbing — esbuild's bundle semantics. The
/// conformity harness validated full parity on the payroll oracle (1057 files / 10471 tests) with
/// native app + native deps, and esbuild remains the automatic fallback for any package the bundler
/// can't handle, so this is zero-regression.
pub fn deps_enabled() -> bool {
    match std::env::var("TURBO_NATIVE_DEPS") {
        Ok(v) => v != "0" && !v.is_empty(),
        Err(_) => true,
    }
}

/// How a reference to an imported local must be rewritten so bindings stay live.
#[derive(Clone)]
struct Access {
    /// the `require`-result variable (`import_foo`)
    var: String,
    /// the property on it (`default` for a default import, the imported name for a named import)
    prop: String,
}

/// Sanitize a module specifier into the esbuild-style `import_<x>` variable suffix.
fn import_var(source: &str, seq: usize) -> String {
    let base = source.trim_end_matches('/').rsplit('/').next().unwrap_or(source);
    let mut s = String::from("import_");
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s.push('_');
    s.push_str(&seq.to_string());
    s
}

/// Collected export registration entry: `exported_name: () => <value_js>`.
pub(crate) struct ExportEntry {
    name: String,
    value_js: String,
}

/// How a module specifier should be referenced. App mode = everything `External` (runtime
/// `require`). Bundler mode (P2b) resolves a package's own RELATIVE files to bundled modules
/// referenced through their lazy init / `__commonJS` wrappers, while bare imports stay `External`.
pub(crate) enum SourceRef {
    /// runtime `require("spec")` (bare import, or app mode where everything is external)
    External,
    /// a bundled ESM module: `init_<id>()` then read `<id>_exports`
    BundledEsm { id: String },
    /// a bundled CJS module: `require_<id>()` returns its `module.exports`
    BundledCjs { id: String },
}

/// Strategy controlling how source-bearing statements (import / export-from / `export *`) and the
/// module's own exports object are lowered. App mode and bundler mode differ only here.
pub(crate) struct LowerCtx<'r> {
    /// the exports-registry object name (`__tt_exports` app; `<id>_exports` per bundled module)
    pub exports_obj: String,
    /// resolve a specifier → `SourceRef`. `None` = app mode (always `External`).
    pub resolve: Option<&'r dyn Fn(&str) -> SourceRef>,
}

impl LowerCtx<'_> {
    fn source_ref(&self, spec: &str) -> SourceRef {
        match &self.resolve {
            Some(f) => f(spec),
            None => SourceRef::External,
        }
    }
}

/// The transformed pieces of one module, before final assembly. App mode wraps these into a
/// self-contained CJS module; bundler mode wraps each into an `__esm`/`__commonJS` init closure.
pub(crate) struct ModuleParts {
    pub requires: Vec<String>,
    pub re_exports: Vec<String>,
    pub exports: Vec<ExportEntry>,
    pub body_code: String,
    pub default_needs_rewrite: bool,
    /// true if the module used any static ESM import/export (else it's CJS / a plain script).
    pub has_module_syntax: bool,
}

struct EmitState<'a, 'r> {
    ast: AstBuilder<'a>,
    imports: HashMap<SymbolId, Access>,
    scoping: Scoping,
    /// bundler mode: resolve a `require("./rel")` specifier to a bundled module to rewrite the call.
    /// `None` in app mode → require() calls are left untouched (runtime resolution).
    resolve: Option<&'r dyn Fn(&str) -> SourceRef>,
}

impl<'a> EmitState<'a, '_> {
    fn access_for_ref(&self, id: &IdentifierReference) -> Option<&Access> {
        let rid = id.reference_id.get()?;
        let sym = self.scoping.get_reference(rid).symbol_id()?;
        self.imports.get(&sym)
    }

    fn build_member(&self, access: &Access) -> Expression<'a> {
        let obj = self.ast.expression_identifier(SPAN, self.ast.ident(&access.var));
        Expression::StaticMemberExpression(self.ast.alloc_static_member_expression(
            SPAN,
            obj,
            self.ast.identifier_name(SPAN, self.ast.ident(&access.prop)),
            false,
        ))
    }

    /// `f(arg)` for a global function name.
    fn call_global(&self, name: &str, arg: Expression<'a>) -> Expression<'a> {
        let callee = self.ast.expression_identifier(SPAN, self.ast.ident(name));
        self.ast.expression_call(SPAN, callee, NONE, self.ast.vec1(Argument::from(arg)), false)
    }

    /// `obj.method(arg)`.
    fn call_member(&self, obj: &str, method: &str, arg: Expression<'a>) -> Expression<'a> {
        let object = self.ast.expression_identifier(SPAN, self.ast.ident(obj));
        let callee = Expression::StaticMemberExpression(self.ast.alloc_static_member_expression(
            SPAN,
            object,
            self.ast.identifier_name(SPAN, self.ast.ident(method)),
            false,
        ));
        self.ast.expression_call(SPAN, callee, NONE, self.ast.vec1(Argument::from(arg)), false)
    }

    /// Convert a dynamic `import(x)` into `Promise.resolve(__toESM(require(x)))` — esbuild's
    /// `--supported:dynamic-import=false` lowering. The runner's CJS `require` resolves relative
    /// specifiers the same way static requires do, and `__toESM` preserves default/named interop
    /// so `(await import('./m')).default` / `.named` keep working.
    fn lower_dynamic_import(&self, source: Expression<'a>) -> Expression<'a> {
        let require_call = self.call_global("require", source);
        let toesm = self.call_global("__toESM", require_call);
        self.call_member("Promise", "resolve", toesm)
    }

    /// `init_<id>(), <id>_exports` as a parenthesized sequence — the namespace of a bundled ESM
    /// module after ensuring it's initialized.
    fn build_init_exports(&self, id: &str) -> Expression<'a> {
        let init = self.ast.expression_identifier(SPAN, self.ast.ident(&format!("init_{id}")));
        let init_call = self.ast.expression_call(SPAN, init, NONE, self.ast.vec(), false);
        let exports = self.ast.expression_identifier(SPAN, self.ast.ident(&format!("{id}_exports")));
        self.ast.expression_sequence(SPAN, self.ast.vec_from_iter([init_call, exports]))
    }

    /// `require_<id>()` — the module.exports of a bundled CJS module.
    fn build_require_id(&self, id: &str) -> Expression<'a> {
        let callee = self.ast.expression_identifier(SPAN, self.ast.ident(&format!("require_{id}")));
        self.ast.expression_call(SPAN, callee, NONE, self.ast.vec(), false)
    }

    /// If `call` is a static `require("literal")`, return the specifier string.
    fn require_spec(call: &CallExpression) -> Option<String> {
        if call.arguments.len() != 1 {
            return None;
        }
        let Expression::Identifier(callee) = &call.callee else { return None };
        if callee.name != "require" {
            return None;
        }
        match call.arguments.first() {
            Some(Argument::StringLiteral(s)) => Some(s.value.to_string()),
            _ => None,
        }
    }
}

impl<'a> VisitMut<'a> for EmitState<'a, '_> {
    fn visit_expression(&mut self, expr: &mut Expression<'a>) {
        if let Expression::Identifier(id) = expr {
            if let Some(access) = self.access_for_ref(id).cloned() {
                *expr = self.build_member(&access);
                return;
            }
        }
        // bundler mode: rewrite `require("./rel")` for a bundled module to its init/require chain.
        if let (Some(resolve), Expression::CallExpression(call)) = (self.resolve, &*expr) {
            if let Some(spec) = Self::require_spec(call) {
                match resolve(&spec) {
                    SourceRef::BundledEsm { id } => { *expr = self.build_init_exports(&id); return; }
                    SourceRef::BundledCjs { id } => { *expr = self.build_require_id(&id); return; }
                    SourceRef::External => {}
                }
            }
        }
        if matches!(expr, Expression::ImportExpression(_)) {
            // take the node out, rewrite its (recursively-visited) source into a require chain.
            let taken = std::mem::replace(expr, self.ast.expression_boolean_literal(SPAN, false));
            let Expression::ImportExpression(imp) = taken else { unreachable!() };
            let mut imp = imp.unbox();
            self.visit_expression(&mut imp.source);
            *expr = self.lower_dynamic_import(imp.source);
            return;
        }
        walk_mut::walk_expression(self, expr);
    }

    fn visit_object_property(&mut self, prop: &mut ObjectProperty<'a>) {
        // `{ a }` where `a` is an imported local must become `{ a: import_x.a }`.
        if prop.shorthand {
            if let Expression::Identifier(id) = &prop.value {
                if self.access_for_ref(id).is_some() {
                    prop.shorthand = false;
                }
            }
        }
        walk_mut::walk_object_property(self, prop);
    }
}

/// Emit esbuild-shaped CJS for `path` (TS/JSX allowed). `None` on parse/transform failure or an
/// unhandled module form (caller falls back to esbuild).
pub fn emit(path: &Path, src: &str) -> Option<String> {
    // 1. TS/JSX strip → plain JS.
    let js = if crate::transform::needs_transform(path) {
        crate::transform::transform(path, src).ok()?
    } else {
        src.to_string()
    };
    // 2-5. Transform to parts in APP mode (everything external / runtime require).
    let ctx = LowerCtx { exports_obj: "__tt_exports".to_string(), resolve: None };
    let parts = transform_to_parts(&js, &ctx)?;
    // 6. No ESM module syntax → a CommonJS module or a plain script: return the transformed body
    //    WITHOUT CJS-wrapping it (wrapping would clobber the file's own module.exports/exports.x).
    //    import() was still lowered.
    if !parts.has_module_syntax {
        return Some(parts.body_code);
    }
    Some(assemble(&parts.requires, &parts.re_exports, &parts.exports, &parts.body_code))
}

/// Parse `js` (already TS/JSX-stripped, plain JS), classify imports/exports under `ctx`, rewrite
/// references to imported locals + lower dynamic `import()`, and codegen the body. Returns the
/// `ModuleParts` for the caller to assemble (app CJS module, or a bundled init closure). `None` on
/// parse/semantic failure or an unhandled form.
pub(crate) fn transform_to_parts(js: &str, ctx: &LowerCtx) -> Option<ModuleParts> {
    let alloc = Allocator::default();
    let stype = SourceType::mjs();
    let parsed = Parser::new(&alloc, js, stype).parse();
    if !parsed.errors.is_empty() {
        return None;
    }
    let mut program = parsed.program;
    let sem = SemanticBuilder::new().build(&program);
    if !sem.errors.is_empty() {
        return None;
    }
    let scoping = sem.semantic.into_scoping();
    let ast = AstBuilder::new(&alloc);

    let Plan { imports, requires, re_exports, exports, body_stmts, default_needs_rewrite } =
        plan(&ast, &scoping, ctx, &mut program)?;

    let mut state = EmitState { ast, imports, scoping, resolve: ctx.resolve };
    let mut body = body_stmts;
    for stmt in body.iter_mut() {
        state.visit_statement(stmt);
    }

    let prog2 = ast.program(SPAN, stype, "", ast.vec(), None, ast.vec(), ast.vec_from_iter(body));
    let mut body_code = Codegen::new().build(&prog2).code;
    if default_needs_rewrite {
        body_code = body_code.replacen("export default ", "var __tt_default = ", 1);
    }

    let has_module_syntax =
        !(requires.is_empty() && re_exports.is_empty() && exports.is_empty() && !default_needs_rewrite);
    Some(ModuleParts { requires, re_exports, exports, body_code, default_needs_rewrite, has_module_syntax })
}

struct Plan<'a> {
    imports: HashMap<SymbolId, Access>,
    requires: Vec<String>,
    re_exports: Vec<String>,
    exports: Vec<ExportEntry>,
    body_stmts: Vec<Statement<'a>>,
    default_needs_rewrite: bool,
}

fn plan<'a>(ast: &AstBuilder<'a>, scoping: &Scoping, ctx: &LowerCtx, program: &mut Program<'a>) -> Option<Plan<'a>> {
    let mut imports: HashMap<SymbolId, Access> = HashMap::new();
    let mut requires: Vec<String> = Vec::new();
    let mut re_exports: Vec<String> = Vec::new();
    let mut exports: Vec<ExportEntry> = Vec::new();
    let mut body_stmts: Vec<Statement<'a>> = Vec::new();
    let mut default_needs_rewrite = false;
    // (exported, local_name, local_symbol) for `export { a, b as c }` — resolved against `imports`
    // after all imports are collected.
    let mut deferred_named: Vec<(String, String, Option<SymbolId>)> = Vec::new();

    let stmts = std::mem::replace(&mut program.body, ast.vec());
    let mut seq = 0usize;

    for stmt in stmts {
        match stmt {
            Statement::ImportDeclaration(decl) => {
                seq += 1;
                if !collect_import(&decl, seq, ctx, &mut imports, &mut requires) {
                    return None;
                }
            }
            Statement::ExportNamedDeclaration(decl) => {
                seq += 1;
                if !collect_export_named(scoping, decl.unbox(), seq, ctx, &mut requires, &mut exports, &mut deferred_named, &mut body_stmts) {
                    return None;
                }
            }
            Statement::ExportDefaultDeclaration(decl) => {
                if collect_export_default(decl, &mut exports, &mut body_stmts) {
                    default_needs_rewrite = true;
                }
            }
            Statement::ExportAllDeclaration(decl) => {
                seq += 1;
                let src = decl.source.value.as_str();
                match &decl.exported {
                    // `export * as ns from "s"` → named export of the namespace object.
                    Some(name) => {
                        let (prelude, expr) = module_namespace_expr(ctx, src, seq, true);
                        let var = import_var(src, seq);
                        requires.extend(prelude);
                        requires.push(format!("var {var} = {expr};"));
                        exports.push(ExportEntry { name: module_export_name(name), value_js: var });
                    }
                    // `export * from "s"` → live merge into this module's exports object AND its
                    // module.exports. The 3rd arg is REQUIRED: `module.exports = __toCommonJS(obj)`
                    // (emitted earlier) snapshots `obj`'s getters into a NEW object, so re-exported
                    // names added to `obj` afterward must ALSO be copied onto module.exports. Inside
                    // a bundled module's `__commonJS` closure, `module.exports` is the closure param.
                    None => {
                        let (prelude, expr) = module_namespace_expr(ctx, src, seq, false);
                        re_exports.extend(prelude);
                        let obj = &ctx.exports_obj;
                        re_exports.push(format!("__reExport({obj}, {expr}, module.exports);"));
                    }
                }
            }
            other => body_stmts.push(other),
        }
    }

    for (exported, local, sym) in deferred_named {
        let value_js = match sym.and_then(|s| imports.get(&s)) {
            Some(access) => format!("{}.{}", access.var, access.prop),
            None => local,
        };
        exports.push(ExportEntry { name: exported, value_js });
    }

    Some(Plan { imports, requires, re_exports, exports, body_stmts, default_needs_rewrite })
}

/// An expression evaluating to a module's namespace object, plus any prelude lines needed first.
/// `toesm` wraps external/CJS results with `__toESM` (default-interop); a bundled ESM module's
/// exports object already has the right getters so it's used directly.
fn module_namespace_expr(ctx: &LowerCtx, src: &str, _seq: usize, toesm: bool) -> (Vec<String>, String) {
    match ctx.source_ref(src) {
        SourceRef::External => {
            let r = format!("require(\"{src}\")");
            (vec![], if toesm { format!("__toESM({r})") } else { r })
        }
        // `(init_<id>(), <id>_exports)` — ensure the bundled ESM module is initialized, then read it.
        SourceRef::BundledEsm { id } => (vec![], format!("(init_{id}(), {id}_exports)")),
        SourceRef::BundledCjs { id } => {
            let r = format!("require_{id}()");
            (vec![], if toesm { format!("__toESM({r})") } else { r })
        }
    }
}

/// Map var-based import specifiers (default/named) to `<var>.<prop>` accesses. Namespace specifiers
/// bind `var` directly (handled by the caller), so they get no entry.
fn map_var_specs(
    specifiers: &[ImportDeclarationSpecifier],
    var: &str,
    imports: &mut HashMap<SymbolId, Access>,
) {
    for spec in specifiers {
        match spec {
            ImportDeclarationSpecifier::ImportDefaultSpecifier(d) => {
                if let Some(sym) = d.local.symbol_id.get() {
                    imports.insert(sym, Access { var: var.to_string(), prop: "default".into() });
                }
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                if let Some(sym) = s.local.symbol_id.get() {
                    imports.insert(sym, Access { var: var.to_string(), prop: module_export_name(&s.imported) });
                }
            }
        }
    }
}

/// Render one ImportDeclaration into require/init line(s) + import-map entries, under `ctx`.
/// Returns false on an unhandled form.
fn collect_import(
    decl: &ImportDeclaration,
    seq: usize,
    ctx: &LowerCtx,
    imports: &mut HashMap<SymbolId, Access>,
    requires: &mut Vec<String>,
) -> bool {
    let src = decl.source.value.as_str();
    let sref = ctx.source_ref(src);

    let Some(specifiers) = &decl.specifiers else {
        // side-effect import: `import "s"`
        match &sref {
            SourceRef::External => {
                let var = import_var(src, seq);
                requires.push(format!("var {var} = require(\"{src}\");"));
            }
            SourceRef::BundledEsm { id } => requires.push(format!("init_{id}();")),
            SourceRef::BundledCjs { id } => requires.push(format!("require_{id}();")),
        }
        return true;
    };

    let has_default_or_ns = specifiers.iter().any(|s| {
        matches!(s, ImportDeclarationSpecifier::ImportDefaultSpecifier(_) | ImportDeclarationSpecifier::ImportNamespaceSpecifier(_))
    });
    let ns_local = specifiers.iter().find_map(|s| match s {
        ImportDeclarationSpecifier::ImportNamespaceSpecifier(n) => Some(n.local.name.to_string()),
        _ => None,
    });

    match &sref {
        // bundled ESM: refs go to `<id>_exports.<name>`; ensure init; bind a namespace local to it.
        SourceRef::BundledEsm { id } => {
            let exports_obj = format!("{id}_exports");
            if let Some(local) = &ns_local {
                requires.push(format!("var {local} = (init_{id}(), {exports_obj});"));
            } else {
                requires.push(format!("init_{id}();"));
            }
            for spec in specifiers {
                match spec {
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(d) => {
                        if let Some(sym) = d.local.symbol_id.get() {
                            imports.insert(sym, Access { var: exports_obj.clone(), prop: "default".into() });
                        }
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
                    ImportDeclarationSpecifier::ImportSpecifier(s) => {
                        if let Some(sym) = s.local.symbol_id.get() {
                            imports.insert(sym, Access { var: exports_obj.clone(), prop: module_export_name(&s.imported) });
                        }
                    }
                }
            }
        }
        // bundled CJS: `var v = require_<id>()` (or __toESM for default/ns), refs `v.name`.
        SourceRef::BundledCjs { id } => {
            let var = ns_local.unwrap_or_else(|| import_var(src, seq));
            let expr = if has_default_or_ns { format!("__toESM(require_{id}())") } else { format!("require_{id}()") };
            requires.push(format!("var {var} = {expr};"));
            map_var_specs(specifiers, &var, imports);
        }
        // external (bare, or app mode): runtime `require("spec")`.
        SourceRef::External => {
            let var = ns_local.unwrap_or_else(|| import_var(src, seq));
            let expr = if has_default_or_ns { format!("__toESM(require(\"{src}\"))") } else { format!("require(\"{src}\")") };
            requires.push(format!("var {var} = {expr};"));
            map_var_specs(specifiers, &var, imports);
        }
    }
    true
}

/// `export <decl>` / `export { ... } [from "s"]`.
fn collect_export_named<'a>(
    scoping: &Scoping,
    decl: ExportNamedDeclaration<'a>,
    seq: usize,
    ctx: &LowerCtx,
    requires: &mut Vec<String>,
    exports: &mut Vec<ExportEntry>,
    deferred_named: &mut Vec<(String, String, Option<SymbolId>)>,
    body_stmts: &mut Vec<Statement<'a>>,
) -> bool {
    if let Some(d) = decl.declaration {
        for name in declaration_binding_names(&d) {
            exports.push(ExportEntry { name: name.clone(), value_js: name });
        }
        // re-emit the bare declaration as a statement (Declaration → Statement variant by variant).
        match d {
            Declaration::VariableDeclaration(v) => body_stmts.push(Statement::VariableDeclaration(v)),
            Declaration::FunctionDeclaration(f) => body_stmts.push(Statement::FunctionDeclaration(f)),
            Declaration::ClassDeclaration(c) => body_stmts.push(Statement::ClassDeclaration(c)),
            _ => return false, // TS-only decl (enum/interface/type) shouldn't survive the strip
        }
        return true;
    }
    if let Some(source) = &decl.source {
        // `export { a, b as c } from "s"` — reference the module, getters read from it.
        let src = source.value.as_str();
        // bind the module's namespace to a local var, then getters read `var.local`.
        let var = import_var(src, seq + 1_000_000);
        let (prelude, expr) = module_namespace_expr(ctx, src, seq, false);
        requires.extend(prelude);
        requires.push(format!("var {var} = {expr};"));
        for spec in &decl.specifiers {
            let exported = module_export_name(&spec.exported);
            let local = module_export_name(&spec.local);
            exports.push(ExportEntry { name: exported, value_js: format!("{var}.{local}") });
        }
        return true;
    }
    // local re-export: `export { a, b as c }` — defer (local may be an import).
    for spec in &decl.specifiers {
        let exported = module_export_name(&spec.exported);
        let local_name = module_export_name(&spec.local);
        let sym = match &spec.local {
            ModuleExportName::IdentifierReference(id) => id
                .reference_id
                .get()
                .and_then(|rid| scoping.get_reference(rid).symbol_id()),
            _ => None,
        };
        deferred_named.push((exported, local_name, sym));
    }
    true
}

/// `export default <decl|expr>`. Returns true when a synthetic `__tt_default` var rewrite is
/// needed on the codegen output (anonymous func/class or an expression default).
fn collect_export_default<'a>(
    decl: ABox<'a, ExportDefaultDeclaration<'a>>,
    exports: &mut Vec<ExportEntry>,
    body_stmts: &mut Vec<Statement<'a>>,
) -> bool {
    // named func/class default keeps a hoisted binding; register it by name, no rewrite.
    let named = match &decl.declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(f) => f.id.as_ref().map(|i| i.name.to_string()),
        ExportDefaultDeclarationKind::ClassDeclaration(c) => c.id.as_ref().map(|i| i.name.to_string()),
        _ => None,
    };
    if let Some(name) = named {
        let inner = decl.unbox();
        match inner.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => body_stmts.push(Statement::FunctionDeclaration(f)),
            ExportDefaultDeclarationKind::ClassDeclaration(c) => body_stmts.push(Statement::ClassDeclaration(c)),
            _ => unreachable!(),
        }
        exports.push(ExportEntry { name: "default".into(), value_js: name });
        return false;
    }
    // anonymous func/class OR an expression: keep the node (so the visitor rewrites inner import
    // refs), register default → __tt_default, signal the codegen prefix-swap.
    body_stmts.push(Statement::ExportDefaultDeclaration(decl));
    exports.push(ExportEntry { name: "default".into(), value_js: "__tt_default".into() });
    true
}

/// Binding names introduced by a declaration (for export registration).
fn declaration_binding_names(d: &Declaration) -> Vec<String> {
    match d {
        Declaration::VariableDeclaration(v) => {
            let mut out = Vec::new();
            for decl in &v.declarations {
                collect_binding_names(&decl.id, &mut out);
            }
            out
        }
        Declaration::FunctionDeclaration(f) => {
            f.id.as_ref().map(|id| vec![id.name.to_string()]).unwrap_or_default()
        }
        Declaration::ClassDeclaration(c) => {
            c.id.as_ref().map(|id| vec![id.name.to_string()]).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Recursively collect identifier names from a binding pattern (handles destructuring exports).
fn collect_binding_names(bp: &BindingPattern, out: &mut Vec<String>) {
    match bp {
        BindingPattern::BindingIdentifier(id) => out.push(id.name.to_string()),
        BindingPattern::ObjectPattern(o) => {
            for p in &o.properties {
                collect_binding_names(&p.value, out);
            }
            if let Some(rest) = &o.rest {
                collect_binding_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(a) => {
            for el in a.elements.iter().flatten() {
                collect_binding_names(el, out);
            }
            if let Some(rest) = &a.rest {
                collect_binding_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(a) => collect_binding_names(&a.left, out),
    }
}

fn module_export_name(n: &ModuleExportName) -> String {
    match n {
        ModuleExportName::IdentifierName(id) => id.name.to_string(),
        ModuleExportName::IdentifierReference(id) => id.name.to_string(),
        ModuleExportName::StringLiteral(s) => s.value.to_string(),
    }
}

/// Runtime helper preamble (shared by app emit + the bundler — emitted ONCE per output). `__esm`
/// and `__commonJS` are the lazy module-init wrappers the bundler uses so circular dependencies +
/// init ordering work (each module's body runs once, on first require, with its exports object
/// already live).
pub(crate) const PREAMBLE: &str = r#"var __defProp = Object.defineProperty;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => { for (var name in all) __defProp(target, name, { get: all[name], enumerable: true, configurable: true }); };
var __copyProps = (to, from, except, desc) => { if (from && typeof from === "object" || typeof from === "function") { for (let key of __getOwnPropNames(from)) if (!__hasOwnProp.call(to, key) && key !== except) __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable, configurable: true }); } return to; };
var __reExport = (target, mod, secondTarget) => (__copyProps(target, mod, "default"), secondTarget && __copyProps(secondTarget, mod, "default"));
var __toESM = (mod) => (mod && mod.__esModule ? mod : (mod && (typeof mod === "object" || typeof mod === "function") && !("default" in mod) && Object.defineProperty(mod, "default", { value: mod, configurable: true }), mod));
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);
var __commonJS = (cb) => { var mod; return () => (mod || (mod = { exports: {} }, cb(mod.exports, mod)), mod.exports); };
var __esm = (fn) => { var done, res; return () => (done || (done = 1, res = fn()), res); };
"#;

/// Assemble one module's CJS text from its parts — NO preamble (the caller emits that once).
/// `module.exports = __toCommonJS(__tt_exports)` is written EARLY (before requires + body) so that
/// under circular requires a re-entrant `require_<id>()` sees the live exports object. Order mirrors
/// esbuild so the loader + `hoist_mock_setup` see the expected shape.
pub(crate) fn assemble_module(requires: &[String], re_exports: &[String], exports: &[ExportEntry], body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 512);
    out.push_str("var __tt_exports = {};\n");
    if !exports.is_empty() {
        out.push_str("__export(__tt_exports, {\n");
        for e in exports {
            out.push_str(&format!("  {}: () => {},\n", quote_key(&e.name), e.value_js));
        }
        out.push_str("});\n");
    }
    out.push_str("module.exports = __toCommonJS(__tt_exports);\n");
    for r in requires {
        out.push_str(r);
        out.push('\n');
    }
    for r in re_exports {
        out.push_str(r);
        out.push('\n');
    }
    out.push_str(body);
    out
}

/// App-mode full module: preamble + the assembled CJS module.
fn assemble(requires: &[String], re_exports: &[String], exports: &[ExportEntry], body: &str) -> String {
    let mut out = String::with_capacity(PREAMBLE.len() + body.len() + 512);
    out.push_str(PREAMBLE);
    out.push_str(&assemble_module(requires, re_exports, exports, body));
    out
}

pub(crate) fn quote_key(name: &str) -> String {
    let ok = !name.is_empty()
        && name.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$').unwrap_or(false)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if ok {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn e(src: &str) -> String {
        emit(Path::new("t.ts"), src).expect("emit should succeed")
    }

    #[test]
    fn named_import_rewrites_to_member() {
        let o = e("import { a, b as c } from \"s\"; console.log(a, c);");
        assert!(o.contains("var import_s_1 = require(\"s\");"), "{o}");
        assert!(o.contains("import_s_1.a"), "{o}");
        assert!(o.contains("import_s_1.b"), "{o}");
    }

    #[test]
    fn default_import_uses_toesm_and_default_member() {
        let o = e("import D from \"s\"; D();");
        assert!(o.contains("var import_s_1 = __toESM(require(\"s\"));"), "{o}");
        assert!(o.contains("import_s_1.default()"), "{o}");
    }

    #[test]
    fn namespace_import_keeps_local_var_no_rewrite() {
        let o = e("import * as ns from \"s\"; ns.x();");
        assert!(o.contains("var ns = __toESM(require(\"s\"));"), "{o}");
        assert!(o.contains("ns.x()"), "{o}");
    }

    #[test]
    fn side_effect_import() {
        let o = e("import \"s\";");
        assert!(o.contains("require(\"s\")"), "{o}");
    }

    #[test]
    fn export_const_and_function_registered() {
        let o = e("export const x = 1; export function fn() { return 2; }");
        assert!(o.contains("x: () => x"), "{o}");
        assert!(o.contains("fn: () => fn"), "{o}");
        assert!(o.contains("var x = 1") || o.contains("const x = 1"), "{o}");
        assert!(o.contains("function fn()"), "{o}");
    }

    #[test]
    fn export_default_expr() {
        let o = e("export default 42;");
        assert!(o.contains("var __tt_default = 42"), "{o}");
        assert!(o.contains("default: () => __tt_default"), "{o}");
    }

    #[test]
    fn export_default_named_function_keeps_binding() {
        let o = e("export default function foo() { return 1; }");
        assert!(o.contains("function foo()"), "{o}");
        assert!(o.contains("default: () => foo"), "{o}");
    }

    #[test]
    fn export_named_local_and_aliased() {
        let o = e("const x = 1; export { x, x as y };");
        assert!(o.contains("x: () => x"), "{o}");
        assert!(o.contains("y: () => x"), "{o}");
    }

    #[test]
    fn export_star() {
        let o = e("export * from \"s\";");
        assert!(o.contains("__reExport(__tt_exports, require(\"s\"), module.exports)"), "{o}");
    }

    #[test]
    fn object_shorthand_with_import_expands() {
        let o = e("import { a } from \"s\"; const o = { a };");
        // shorthand must expand so it reads the live import, not declare a new binding.
        assert!(o.contains("a: import_s_1.a"), "{o}");
    }

    #[test]
    fn reexport_from_source() {
        let o = e("export { a, b as c } from \"s\";");
        assert!(o.contains("require(\"s\")"), "{o}");
        assert!(o.contains("a: () =>") && o.contains(".a"), "{o}");
        assert!(o.contains("c: () =>") && o.contains(".b"), "{o}");
    }
}

#[cfg(test)]
mod tests_dynimport {
    use super::*;
    use std::path::Path;
    #[test]
    fn dynamic_import_lowers_to_require() {
        let o = emit(Path::new("t.ts"), "const m = await import(\"./x\"); m.default;").unwrap();
        assert!(o.contains("Promise.resolve(__toESM(require(\"./x\")))"), "{o}");
        assert!(!o.contains(" import("), "raw import() must be gone: {o}");
    }
    #[test]
    fn dynamic_import_with_imported_specifier_var() {
        // import() whose arg references an imported binding gets the live access rewrite too.
        let o = emit(Path::new("t.ts"), "import { p } from \"s\"; await import(p);").unwrap();
        assert!(o.contains("require(import_s_1.p)"), "{o}");
    }
}
