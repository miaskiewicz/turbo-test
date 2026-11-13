//! Native ESM→CJS emitter (oxc) — the Rust replacement for the per-file `esbuild --format=cjs`
//! transform (`runner.rs::esbuild_transform_cjs`). oxc 0.134 has NO CommonJS module transform
//! (its `Module::CommonJS` only rewrites TS `import x = require()`), so this hand-writes the
//! ESM→CJS lowering, targeting esbuild's output CONTRACT so the existing downstream consumers
//! (`hoist_mock_setup`, `shared_mock_lets`, the loader) keep working:
//!
//!   - imports → `var import_<src> = require("src")` (or `__toESM(require(...))` when the import
//!     binds a default or namespace), references to named/default locals rewritten to
//!     `import_<src>.<name>` (`import_<src>["a-b"]` for a non-identifier string name) so bindings
//!     stay LIVE (spyOn / vi.mock observe writes);
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
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder, SymbolFlags, SymbolId};
use oxc_span::{SourceType, Span, SPAN};
use oxc_transformer::{HelperLoaderMode, TransformOptions, Transformer};

/// Whether the native ESM→CJS emitter is enabled for app files. **Default ON** (P2a cutover): the
/// conformity harness validated full parity on the payroll oracle (1057 files / 10471 tests) and
/// esbuild remains the automatic fallback for any form the emitter doesn't handle, so native-on is
/// zero-regression. Opt OUT with `TURBO_NATIVE_CJS=0` (or empty) — the harness uses this to run the
/// esbuild baseline. (esbuild is still used for node_modules bundling, coverage, and
/// decorator-metadata regardless — see the gate in `read_transformed`.)
pub fn enabled() -> bool {
    default_on(std::env::var("TURBO_NATIVE_CJS").ok().as_deref())
}

/// A default-ON switch's value: unset (or non-UTF-8) → on; `0` or empty → off; anything else → on.
fn default_on(v: Option<&str>) -> bool {
    match v {
        Some(v) => v != "0" && !v.is_empty(),
        None => true,
    }
}

/// A default-OFF switch's value: on iff set to a non-empty (UTF-8) value.
fn non_empty(v: Option<&str>) -> bool {
    v.is_some_and(|v| !v.is_empty())
}

/// Strict mode (conformity coverage measurement): when on, the runner does NOT fall back to esbuild
/// for app files the native emitter can't handle — the unhandled module surfaces as a load error so
/// the harness can count true native coverage instead of it being silently masked by the fallback.
pub fn strict() -> bool {
    non_empty(std::env::var("TURBO_NATIVE_CJS_STRICT").ok().as_deref())
}

/// Whether to use the native bundler for **node_modules** packages (P2b). **Default ON** (opt out
/// via `TURBO_NATIVE_DEPS=0`). Uses `crate::bundler` to bundle a package's relative graph with lazy
/// `__commonJS` init wrappers (circular-safe) + asset stubbing — esbuild's bundle semantics. The
/// conformity harness validated full parity on the payroll oracle (1057 files / 10471 tests) with
/// native app + native deps, and esbuild remains the automatic fallback for any package the bundler
/// can't handle, so this is zero-regression.
pub fn deps_enabled() -> bool {
    default_on(std::env::var("TURBO_NATIVE_DEPS").ok().as_deref())
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
    let base = source
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(source);
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
    /// a bundled ESM module: `init_<id>()` then read `<id>_exports`. The lowering is unit-tested
    /// (esm_cjs_tests.rs); the bundler resolves package files as CJS only so far (bundler.rs),
    /// so outside the tests nothing constructs it.
    #[cfg_attr(not(test), allow(dead_code))]
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
    /// the module's path for `import.meta` (bundler: each bundled module's own); `None` = the CJS
    /// wrapper's `__filename` (app mode)
    pub module_path: Option<String>,
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
    /// true if the module used any static ESM import/export (else it's CJS / a plain script).
    pub has_module_syntax: bool,
    /// true if the body references `__toESM` (a lowered dynamic `import()`). Only the module
    /// preamble declares it, so a script without module syntax needs `TO_ESM_HELPER` prepended.
    pub uses_to_esm: bool,
    /// [`BUNDLE_HELPERS`] the module declares at its top level other than with `var` / `function`
    /// (`let`, `const`, `class`, an import): a closure parameter of the same name would be a
    /// redeclaration, so the bundler leaves those to the module (turbo-test#230).
    pub lexical_helpers: Vec<&'static str>,
}

/// The preamble helpers a bundled module's own text calls (its export registration, its
/// `module.exports`, its lowered imports and `export *`). The bundler hands them to each module's
/// `__commonJS` closure as parameters, so a module that declares one itself with `var` (an
/// esbuild / tsup build's own `var __export = ...`, as pinia's and vue-router's) does not hoist an
/// `undefined` over the bundle's (turbo-test#230): a `var` redeclaring a parameter keeps the
/// parameter's value until its own assignment runs. Same order as `__commonJS`'s `cb` call.
pub(crate) const BUNDLE_HELPERS: [&str; 4] = ["__export", "__toCommonJS", "__toESM", "__reExport"];

/// The [`BUNDLE_HELPERS`] declared at the top level of `scoping`'s program other than with `var`
/// or `function` (see `ModuleParts::lexical_helpers`).
fn lexical_helpers(scoping: &Scoping) -> Vec<&'static str> {
    BUNDLE_HELPERS
        .into_iter()
        .filter(|name| {
            scoping.get_root_binding((*name).into()).is_some_and(|id| {
                let flags = scoping.symbol_flags(id);
                !flags.is_function_scoped_declaration() && !flags.is_function()
            })
        })
        .collect()
}

struct EmitState<'a, 'r> {
    ast: AstBuilder<'a>,
    imports: HashMap<SymbolId, Access>,
    scoping: Scoping,
    /// bundler mode: resolve a `require("./rel")` specifier to a bundled module to rewrite the call.
    /// `None` in app mode → require() calls are left untouched (runtime resolution).
    resolve: Option<&'r dyn Fn(&str) -> SourceRef>,
    /// set when the body now calls `__toESM` (see `ModuleParts::uses_to_esm`).
    uses_to_esm: bool,
    /// `LowerCtx::module_path`
    module_path: Option<String>,
    /// Root-level bindings that shadow the CJS wrapper's parameters (`exports`, `module`,
    /// `require`, `__filename`, `__dirname`) must be renamed to avoid strict-mode redeclaration
    /// errors. Maps the SymbolId to the replacement name.
    wrapper_renames: HashMap<SymbolId, String>,
}

impl<'a> EmitState<'a, '_> {
    /// The new name of a reference to a renamed wrapper-parameter binding (`wrapper_renames`).
    fn renamed_ref(&self, id: &IdentifierReference) -> Option<&String> {
        let sym = self
            .scoping
            .get_reference(id.reference_id.get()?)
            .symbol_id()?;
        self.wrapper_renames.get(&sym)
    }

    /// `{ name }` / `{ name = init }` as `{ name: renamed }` / `{ name: renamed = init }`.
    fn expanded_shorthand_target(
        &self,
        p: &mut AssignmentTargetPropertyIdentifier<'a>,
        renamed: &str,
    ) -> AssignmentTargetProperty<'a> {
        let span = p.span;
        let key = self
            .ast
            .property_key_static_identifier(span, p.binding.name);
        let target = AssignmentTarget::from(
            self.ast
                .simple_assignment_target_assignment_target_identifier(
                    span,
                    self.ast.ident(renamed),
                ),
        );
        let binding = match p.init.take() {
            Some(init) => self
                .ast
                .assignment_target_maybe_default_assignment_target_with_default(span, target, init),
            None => AssignmentTargetMaybeDefault::from(target),
        };
        self.ast
            .assignment_target_property_assignment_target_property_property(
                span, key, binding, false,
            )
    }

    fn access_for_ref(&self, id: &IdentifierReference) -> Option<&Access> {
        let rid = id.reference_id.get()?;
        let sym = self.scoping.get_reference(rid).symbol_id()?;
        self.imports.get(&sym)
    }

    /// `var.prop`, or `var["prop"]` when `prop` (a string-named import such as `"a-b"`) is not an
    /// identifier name. Every node carries `span`, the replaced identifier's: the source map then
    /// maps the rewritten reference — and V8's position of a call of it, the property — back to
    /// the identifier (turbo-test#110).
    fn build_member(&self, access: &Access, span: Span) -> Expression<'a> {
        let obj = self
            .ast
            .expression_identifier(span, self.ast.ident(&access.var));
        if !is_plain_ident(&access.prop) {
            let key = self
                .ast
                .expression_string_literal(span, self.ast.str(&access.prop), None);
            return Expression::ComputedMemberExpression(
                self.ast
                    .alloc_computed_member_expression(span, obj, key, false),
            );
        }
        Expression::StaticMemberExpression(self.ast.alloc_static_member_expression(
            span,
            obj,
            self.ast.identifier_name(span, self.ast.ident(&access.prop)),
            false,
        ))
    }

    /// `globalThis.__ttImportMeta(<module path>)` (the module path a string literal in bundler
    /// mode, else the CJS wrapper's `__filename`), with `span` (the `import.meta`'s) for the map.
    fn build_import_meta(&self, span: Span) -> Expression<'a> {
        let path = match &self.module_path {
            Some(p) => self
                .ast
                .expression_string_literal(span, self.ast.str(p), None),
            None => self
                .ast
                .expression_identifier(span, self.ast.ident("__filename")),
        };
        let global = self
            .ast
            .expression_identifier(span, self.ast.ident("globalThis"));
        let callee = Expression::StaticMemberExpression(
            self.ast.alloc_static_member_expression(
                span,
                global,
                self.ast
                    .identifier_name(span, self.ast.ident("__ttImportMeta")),
                false,
            ),
        );
        let args = self.ast.vec1(Argument::from(path));
        self.ast.expression_call(span, callee, NONE, args, false)
    }

    /// `f(arg)` for a global function name.
    fn call_global(&self, name: &str, arg: Expression<'a>) -> Expression<'a> {
        let callee = self.ast.expression_identifier(SPAN, self.ast.ident(name));
        let args = self.ast.vec1(Argument::from(arg));
        self.ast.expression_call(SPAN, callee, NONE, args, false)
    }

    /// `object.method(args…)`.
    fn call_method(
        &self,
        object: Expression<'a>,
        method: &str,
        args: Vec<Expression<'a>>,
    ) -> Expression<'a> {
        let callee = Expression::StaticMemberExpression(self.ast.alloc_static_member_expression(
            SPAN,
            object,
            self.ast.identifier_name(SPAN, self.ast.ident(method)),
            false,
        ));
        let args = self.ast.vec_from_iter(args.into_iter().map(Argument::from));
        self.ast.expression_call(SPAN, callee, NONE, args, false)
    }

    /// `(<param>) => body` (no parameter when `param` is `None`).
    fn arrow(&self, param: Option<&str>, body: Expression<'a>) -> Expression<'a> {
        let ast = self.ast;
        let items = ast.vec_from_iter(param.map(|name| {
            let pattern = ast.binding_pattern_binding_identifier(SPAN, ast.ident(name));
            ast.plain_formal_parameter(SPAN, pattern)
        }));
        let kind = FormalParameterKind::ArrowFormalParameters;
        let params = ast.alloc_formal_parameters(SPAN, kind, items, NONE);
        let stmt = ast.statement_expression(SPAN, body);
        let body = ast.alloc_function_body(SPAN, ast.vec(), ast.vec1(stmt));
        ast.expression_arrow_function(SPAN, true, false, NONE, params, NONE, body)
    }

    /// `Promise.resolve().then(() => module)` — `module` is evaluated in a later job.
    fn deferred(&self, module: Expression<'a>) -> Expression<'a> {
        let promise = self
            .ast
            .expression_identifier(SPAN, self.ast.ident("Promise"));
        let resolved = self.call_method(promise, "resolve", vec![]);
        let load = self.arrow(None, module);
        self.call_method(resolved, "then", vec![load])
    }

    /// `__toESM(<module.exports>)`, noting that the body now needs the helper.
    fn wrap_to_esm(&mut self, module_exports: Expression<'a>) -> Expression<'a> {
        self.uses_to_esm = true;
        self.call_global("__toESM", module_exports)
    }

    /// Lower a dynamic `import(x)` like esbuild's `--supported:dynamic-import=false`:
    /// `Promise.resolve().then(() => __toESM(require(x)))`. The load runs in a later job, so a
    /// module that throws REJECTS the promise instead of throwing out of `import()` (Node / vitest),
    /// and a `vi.doMock` registered before the call is seen. The runner's CJS `require` resolves
    /// relative specifiers like static requires do, and `__toESM` keeps default/named interop so
    /// `(await import('./m')).default` / `.named` work. A computed specifier is evaluated at the
    /// call, as in Node (esbuild evaluates it inside the `.then`):
    /// `((__tt_spec) => Promise.resolve().then(() => __toESM(require(__tt_spec))))(x)`. In bundler
    /// mode a literal specifier of a bundled module loads that module's lazy init / `require_<id>`.
    fn lower_dynamic_import(&mut self, source: Expression<'a>) -> Expression<'a> {
        let Expression::StringLiteral(spec) = &source else {
            let spec = self
                .ast
                .expression_identifier(SPAN, self.ast.ident("__tt_spec"));
            let require_call = self.call_global("require", spec);
            let load = self.wrap_to_esm(require_call);
            let deferred = self.deferred(load);
            let lower = self.arrow(Some("__tt_spec"), deferred);
            let lower = self.ast.expression_parenthesized(SPAN, lower);
            let args = self.ast.vec1(Argument::from(source));
            return self.ast.expression_call(SPAN, lower, NONE, args, false);
        };
        let module = match self.resolve.map(|resolve| resolve(spec.value.as_str())) {
            Some(SourceRef::BundledEsm { id }) => self.build_init_exports(&id),
            Some(SourceRef::BundledCjs { id }) => {
                let require_call = self.build_require_id(&id);
                self.wrap_to_esm(require_call)
            }
            _ => {
                let require_call = self.call_global("require", source);
                self.wrap_to_esm(require_call)
            }
        };
        self.deferred(module)
    }

    /// `init_<id>(), <id>_exports` as a parenthesized sequence — the namespace of a bundled ESM
    /// module after ensuring it's initialized.
    fn build_init_exports(&self, id: &str) -> Expression<'a> {
        let init = self
            .ast
            .expression_identifier(SPAN, self.ast.ident(&format!("init_{id}")));
        let init_call = self
            .ast
            .expression_call(SPAN, init, NONE, self.ast.vec(), false);
        let exports = self
            .ast
            .expression_identifier(SPAN, self.ast.ident(&format!("{id}_exports")));
        self.ast
            .expression_sequence(SPAN, self.ast.vec_from_iter([init_call, exports]))
    }

    /// `require_<id>()` — the module.exports of a bundled CJS module.
    fn build_require_id(&self, id: &str) -> Expression<'a> {
        let callee = self
            .ast
            .expression_identifier(SPAN, self.ast.ident(&format!("require_{id}")));
        self.ast
            .expression_call(SPAN, callee, NONE, self.ast.vec(), false)
    }

    /// If `call` is a static `require("literal")`, return the specifier string.
    fn require_spec(call: &CallExpression) -> Option<String> {
        if call.arguments.len() != 1 {
            return None;
        }
        let Expression::Identifier(callee) = &call.callee else {
            return None;
        };
        if callee.name != "require" {
            return None;
        }
        match call.arguments.first() {
            Some(Argument::StringLiteral(s)) => Some(s.value.to_string()),
            _ => None,
        }
    }

    /// Rewrite an imported-local identifier to its `import_x.name` access, returning whether a
    /// rewrite happened (so `visit_expression` can skip the normal walk).
    fn rewrite_ident_ref(&mut self, expr: &mut Expression<'a>) -> bool {
        let Expression::Identifier(id) = expr else {
            return false;
        };
        if let Some(renamed) = self.renamed_ref(id) {
            *expr = self
                .ast
                .expression_identifier(id.span, self.ast.ident(renamed));
            return true;
        }
        let Some(access) = self.access_for_ref(id).cloned() else {
            return false;
        };
        *expr = self.build_member(&access, id.span);
        true
    }

    /// Rewrite a bundler-resolved `require("./rel")` into the bundled init/require chain.
    fn rewrite_bundled_require(&mut self, expr: &mut Expression<'a>) -> bool {
        let (Some(resolve), Expression::CallExpression(call)) = (self.resolve, &*expr) else {
            return false;
        };
        let Some(spec) = Self::require_spec(call) else {
            return false;
        };
        match resolve(&spec) {
            SourceRef::BundledEsm { id } => {
                *expr = self.build_init_exports(&id);
                true
            }
            SourceRef::BundledCjs { id } => {
                *expr = self.build_require_id(&id);
                true
            }
            SourceRef::External => false,
        }
    }

    /// `import.meta` → runtime.js `__ttImportMeta(<module path>)` (a CJS script cannot hold
    /// `import.meta`: V8's "Cannot use 'import.meta' outside a module", turbo-test#123).
    fn rewrite_import_meta(&mut self, expr: &mut Expression<'a>) -> bool {
        let Expression::MetaProperty(m) = &*expr else {
            return false;
        };
        if m.meta.name != "import" || m.property.name != "meta" {
            return false;
        }
        *expr = self.build_import_meta(m.span);
        true
    }
}

impl<'a> VisitMut<'a> for EmitState<'a, '_> {
    fn visit_expression(&mut self, expr: &mut Expression<'a>) {
        if self.rewrite_ident_ref(expr) {
            return;
        }
        if self.rewrite_bundled_require(expr) {
            return;
        }
        if self.rewrite_import_meta(expr) {
            return;
        }
        if matches!(expr, Expression::ImportExpression(_)) {
            // take the node out, rewrite its (recursively-visited) source into a require chain.
            let taken = std::mem::replace(expr, self.ast.expression_boolean_literal(SPAN, false));
            let Expression::ImportExpression(imp) = taken else {
                unreachable!()
            };
            let mut imp = imp.unbox();
            self.visit_expression(&mut imp.source);
            *expr = self.lower_dynamic_import(imp.source);
            return;
        }
        walk_mut::walk_expression(self, expr);
    }

    /// Assignment / update targets (`module = 2`, `exports++`, `[exports] = …`) of a renamed
    /// binding; expression reads are rewritten by `rewrite_ident_ref`.
    fn visit_identifier_reference(&mut self, id: &mut IdentifierReference<'a>) {
        if let Some(renamed) = self.renamed_ref(id) {
            id.name = self.ast.ident(renamed);
        }
    }

    /// `({ module } = obj)`: the shorthand's name is also the property read, so a renamed target
    /// becomes `({ module: _module } = obj)`.
    fn visit_assignment_target_property(&mut self, prop: &mut AssignmentTargetProperty<'a>) {
        if let AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(p) = prop {
            if let Some(renamed) = self.renamed_ref(&p.binding).cloned() {
                *prop = self.expanded_shorthand_target(p, &renamed);
            }
        }
        walk_mut::walk_assignment_target_property(self, prop);
    }

    fn visit_binding_identifier(&mut self, id: &mut BindingIdentifier<'a>) {
        if let Some(sym) = id.symbol_id.get() {
            if let Some(renamed) = self.wrapper_renames.get(&sym) {
                id.name = self.ast.ident(renamed);
            }
        }
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
/// unhandled module form (caller falls back to esbuild). Single-pass: TS/JSX strip + ESM→CJS on ONE
/// AST so, under coverage, a single codegen source map maps the output back to the ORIGINAL `.ts`
/// lines (no esbuild). The map is appended inline (`//# sourceMappingURL=…`) for `coverage.rs`.
pub fn emit(path: &Path, src: &str) -> Option<String> {
    emit_with(path, src, false)
}

/// Like `emit`, but `legacy_decorators = true` lowers TypeScript **legacy** decorators +
/// `emitDecoratorMetadata` in the SAME oxc TS-strip pass (NestJS / Sequelize-typescript /
/// Mongoose: `@Injectable`, `@Table`, `@Column`, `@Prop`, parameter `@Inject`). The caller passes
/// `true` only for app files in an `experimentalDecorators` project that actually use a decorator
/// (see `runner.rs::native_transform_cjs`). Without this, oxc's DEFAULT options leave a decorator
/// either un-lowered (`export @Table class …` → "Unexpected token 'export'" in the CJS wrapper) or
/// lowered with 2022-standard semantics (a class-binding self-reference → "Cannot access 'X' before
/// initialization"). Legacy lowering emits `babelHelpers.decorate/decorateParam/decorateMetadata`
/// (External helper mode), which `runtime.js` provides globally — so the native path no longer needs
/// the esbuild/tsc metadata fallback (which is unavailable when the project ships no esbuild binary).
pub fn emit_with(path: &Path, src: &str, legacy_decorators: bool) -> Option<String> {
    emit_inner(path, src, legacy_decorators, crate::coverage::enabled())
}

/// `emit_with` that always appends the inline source map (test locations need it, location.rs).
pub fn emit_mapped(path: &Path, src: &str, legacy_decorators: bool) -> Option<String> {
    emit_inner(path, src, legacy_decorators, true)
}

/// `emit_with` with the coverage decision injected: `with_map = true` appends the inline source map.
fn emit_inner(path: &Path, src: &str, legacy_decorators: bool, with_map: bool) -> Option<String> {
    let alloc = Allocator::default();
    let mut program = parse_source_file(&alloc, path, src)?;

    // TS/JSX strip IN PLACE (original spans preserved → the codegen map points at the .ts source).
    if crate::transform::needs_transform(path)
        && !strip_ts_jsx_in_place(&alloc, path, &mut program, legacy_decorators)
    {
        return None;
    }

    // Rebuild semantic on the now-plain-JS AST for the ESM→CJS reference rewriting (no TS enums
    // remain, so no `with_enum_eval` here or in `transform_to_parts`).
    let scoping = build_plain_js_scoping(&program)?;
    let ast = AstBuilder::new(&alloc);

    let ctx = LowerCtx {
        exports_obj: "__tt_exports".to_string(),
        resolve: None,
        module_path: None,
    };
    let map_src = if with_map { Some((path, src)) } else { None };
    let (parts, mappings) = lower(ast, scoping, &ctx, &mut program, map_src)?;
    Some(assemble_output(path, parts, mappings.as_ref()))
}

/// Parse `src` as a module whose source type comes from `path`; `None` on parse error.
fn parse_source_file<'a>(
    alloc: &'a Allocator,
    path: &Path,
    src: &'a str,
) -> Option<oxc_ast::ast::Program<'a>> {
    let stype = SourceType::from_path(path).ok()?;
    let parsed = Parser::new(alloc, src, stype).parse();
    if !parsed.errors.is_empty() {
        return None;
    }
    Some(parsed.program)
}

/// Build semantic/scoping for a plain-JS program (no TS enums remain), returning `None` on errors.
fn build_plain_js_scoping(program: &Program) -> Option<Scoping> {
    let sem = SemanticBuilder::new().build(program);
    if !sem.errors.is_empty() {
        return None;
    }
    Some(sem.semantic.into_scoping())
}

/// TS/JSX strip (and optional legacy-decorator lowering) IN PLACE. Returns `false` on error.
fn strip_ts_jsx_in_place<'a>(
    alloc: &'a Allocator,
    path: &Path,
    program: &mut oxc_ast::ast::Program<'a>,
    legacy_decorators: bool,
) -> bool {
    let scoping0 = crate::transform::semantic_scoping(program);
    let mut opts = TransformOptions::default();
    if legacy_decorators {
        // Mirror `transform::transform_decorators_with_metadata` so output is identical to the
        // tsc/esbuild metadata path: legacy decorators, metadata, no strict-null elision (so
        // `x?: T` / `T | null` emit `T`, not `Object` — matches tsc/ts-jest, which Mongoose
        // `@Prop`/Sequelize `@Column` rely on), helpers as global `babelHelpers.*`.
        opts.decorator.legacy = true;
        opts.decorator.emit_decorator_metadata = true;
        opts.decorator.strict_null_checks = false;
        opts.helper_loader.mode = HelperLoaderMode::External;
    }
    let ret = Transformer::new(alloc, path, &opts).build_with_scoping(scoping0, program);
    ret.errors.is_empty()
}

/// Assemble the final output source for `emit_inner`, appending the inline source map when one was
/// produced. Chooses the unwrapped (script / CJS) shape when the module has no ESM syntax, else the
/// `assemble` wrapper.
fn assemble_output(path: &Path, parts: ModuleParts, mappings: Option<&String>) -> String {
    if !parts.has_module_syntax {
        let helper = if parts.uses_to_esm { TO_ESM_HELPER } else { "" };
        let mut out = format!("{helper}{}", parts.body_code);
        if let Some(m) = mappings {
            append_inline_map(&mut out, path, helper.matches('\n').count(), m);
        }
        return out;
    }
    let mut out = assemble(
        &parts.requires,
        &parts.re_exports,
        &parts.exports,
        &parts.body_code,
    );
    if let Some(m) = mappings {
        // lines before body_code in the output = preamble + the assembled-module prefix.
        let prefix = PREAMBLE.matches('\n').count()
            + assemble_module(&parts.requires, &parts.re_exports, &parts.exports, "")
                .matches('\n')
                .count();
        append_inline_map(&mut out, path, prefix, m);
    }
    out
}

/// Parse `js` (already TS/JSX-stripped, plain JS) and lower to `ModuleParts` under `ctx`. Used by
/// the bundler (per module); no source map. `None` on parse/semantic failure or an unhandled form.
pub(crate) fn transform_to_parts(js: &str, ctx: &LowerCtx) -> Option<ModuleParts> {
    let alloc = Allocator::default();
    let stype = SourceType::mjs();
    let parsed = Parser::new(&alloc, js, stype).parse();
    if !parsed.errors.is_empty() {
        return None;
    }
    let mut program = parsed.program;
    let scoping = build_plain_js_scoping(&program)?;
    let ast = AstBuilder::new(&alloc);
    let (parts, _) = lower(ast, scoping, ctx, &mut program, None)?;
    Some(parts)
}

/// Top-level bindings named like a CJS wrapper parameter, with fresh names. A `let` / `const` /
/// `class` one always: in the wrapper it redeclares the parameter (a SyntaxError), also in an ES
/// module whose only module syntax (`export {}`, `import.meta`) lowers to nothing. In an ES module
/// (`module_syntax`) a `var` / function one too: it would replace the `module` / `require` the
/// emitted `module.exports = …` and import `require(…)` calls use. Imports are lowered away and
/// keep their name; a CommonJS file's `var exports = exports || {}` re-declares the parameter on
/// purpose, as Node runs it.
fn wrapper_renames(scoping: &Scoping, module_syntax: bool) -> HashMap<SymbolId, String> {
    let lexical = SymbolFlags::BlockScopedVariable | SymbolFlags::Class;
    ["exports", "module", "require", "__filename", "__dirname"]
        .into_iter()
        .filter_map(|name| Some((scoping.get_root_binding(name.into())?, name)))
        .filter(|(sym, _)| {
            let flags = scoping.symbol_flags(*sym);
            !flags.contains(SymbolFlags::Import) && (module_syntax || flags.intersects(lexical))
        })
        .map(|(sym, name)| (sym, unused_name(scoping, name)))
        .collect()
}

/// `_name`, else `_name2`, `_name3`, … : the first one the module neither binds nor reads as a
/// global.
fn unused_name(scoping: &Scoping, name: &str) -> String {
    let taken = |n: &str| {
        scoping.symbol_names().any(|s| s == n)
            || scoping
                .root_unresolved_references()
                .keys()
                .any(|k| k.as_str() == n)
    };
    (1..)
        .map(|i| match i {
            1 => format!("_{name}"),
            _ => format!("_{name}{i}"),
        })
        .find(|n| !taken(n))
        .unwrap_or_default()
}

/// A renamed function / class keeps its source `.name` (`function module` stays `module`), set
/// once the module body has defined it.
fn source_names(renames: &HashMap<SymbolId, String>, scoping: &Scoping) -> String {
    let named = SymbolFlags::Function | SymbolFlags::Class;
    let mut fixes: Vec<String> = renames
        .iter()
        .filter(|(sym, _)| scoping.symbol_flags(**sym).intersects(named))
        .map(|(sym, renamed)| {
            format!(
                "\nObject.defineProperty({renamed}, \"name\", {{ value: \"{}\", configurable: true }});",
                scoping.symbol_name(*sym)
            )
        })
        .collect();
    fixes.sort();
    fixes.concat()
}

/// Point a local export's getter at its renamed binding (`export const require` → `_require`).
fn renamed_exports(
    exports: Vec<ExportEntry>,
    renames: &HashMap<SymbolId, String>,
    scoping: &Scoping,
) -> Vec<ExportEntry> {
    let by_name: HashMap<&str, &str> = renames
        .iter()
        .map(|(sym, renamed)| (scoping.symbol_name(*sym), renamed.as_str()))
        .collect();
    exports
        .into_iter()
        .map(|mut e| {
            if let Some(renamed) = by_name.get(e.value_js.as_str()) {
                e.value_js = (*renamed).to_string();
            }
            e
        })
        .collect()
}

/// Core: classify imports/exports under `ctx`, rewrite imported-local refs + dynamic `import()` +
/// (bundler) `require()` calls, codegen the body. `map_src = Some((path, original_src))` enables a
/// codegen source map (returned as the raw VLQ `mappings` string for the body).
fn lower<'a>(
    ast: AstBuilder<'a>,
    scoping: Scoping,
    ctx: &LowerCtx,
    program: &mut Program<'a>,
    map_src: Option<(&Path, &'a str)>,
) -> Option<(ModuleParts, Option<String>)> {
    let lexical_helpers = lexical_helpers(&scoping);
    let p = plan(&ast, &scoping, ctx, program)?;
    let (requires, re_exports, exports) = (p.requires, p.re_exports, p.exports);

    let has_module_syntax = !(requires.is_empty()
        && re_exports.is_empty()
        && exports.is_empty()
        && !p.default_needs_rewrite);
    let wrapper_renames = wrapper_renames(&scoping, has_module_syntax);
    let exports = renamed_exports(exports, &wrapper_renames, &scoping);
    let name_fixes = source_names(&wrapper_renames, &scoping);
    let mut state = EmitState {
        ast,
        imports: p.imports,
        scoping,
        resolve: ctx.resolve,
        uses_to_esm: false,
        module_path: ctx.module_path.clone(),
        wrapper_renames,
    };
    let mut body = p.body_stmts;
    for stmt in body.iter_mut() {
        state.visit_statement(stmt);
    }

    let (mut body_code, mappings) = codegen_body(ast, body, map_src);
    if p.default_needs_rewrite {
        // same-line replacement → does not shift line numbers, map stays valid.
        body_code = body_code.replacen("export default ", "var __tt_default = ", 1);
    }
    // after the body: no source line moves
    body_code.push_str(&name_fixes);

    let uses_to_esm = state.uses_to_esm;
    Some((
        ModuleParts {
            requires,
            re_exports,
            exports,
            body_code,
            has_module_syntax,
            uses_to_esm,
            lexical_helpers,
        },
        mappings,
    ))
}

/// Codegen the lowered body, with an inline source map when `map_src` is provided.
fn codegen_body<'a>(
    ast: AstBuilder<'a>,
    body: Vec<Statement<'a>>,
    map_src: Option<(&Path, &'a str)>,
) -> (String, Option<String>) {
    // codegen resolves sourcemap spans against `program.source_text` (NOT with_source_text), so the
    // synthetic program must carry the ORIGINAL source text for the map to point at the .ts lines.
    let prog_src = map_src.map(|(_, s)| s).unwrap_or("");
    let stype = SourceType::mjs();
    let body = ast.vec_from_iter(body);
    let prog2 = ast.program(SPAN, stype, prog_src, ast.vec(), None, ast.vec(), body);
    match map_src {
        Some((path, _)) => {
            let ret = Codegen::new()
                .with_options(CodegenOptions {
                    source_map_path: Some(path.to_path_buf()),
                    ..Default::default()
                })
                .build(&prog2);
            (ret.code, ret.map.map(|m| m.to_json().mappings))
        }
        None => (Codegen::new().build(&prog2).code, None),
    }
}

/// Append an inline source map (`//# sourceMappingURL=data:…base64`) whose generated lines align
/// with the FINAL output: `prefix_lines` empty groups are prepended so the body's mappings start at
/// the right output line. `coverage.rs::decode_inline_map` reads this exactly like esbuild's.
fn append_inline_map(out: &mut String, path: &Path, prefix_lines: usize, body_mappings: &str) {
    use base64::Engine;
    let mappings = format!("{}{}", ";".repeat(prefix_lines), body_mappings);
    let src_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let json = format!(
        "{{\"version\":3,\"sources\":[{}],\"sourcesContent\":[null],\"mappings\":{}}}",
        json_string(&src_name),
        json_string(&mappings),
    );
    let b64 = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());
    out.push_str("\n//# sourceMappingURL=data:application/json;base64,");
    out.push_str(&b64);
    out.push('\n');
}

/// Minimal JSON string encoder (quotes + escapes) for the inline-map fields.
fn json_string(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

struct Plan<'a> {
    imports: HashMap<SymbolId, Access>,
    requires: Vec<String>,
    re_exports: Vec<String>,
    exports: Vec<ExportEntry>,
    body_stmts: Vec<Statement<'a>>,
    default_needs_rewrite: bool,
}

fn plan<'a>(
    ast: &AstBuilder<'a>,
    scoping: &Scoping,
    ctx: &LowerCtx,
    program: &mut Program<'a>,
) -> Option<Plan<'a>> {
    let mut imports: HashMap<SymbolId, Access> = HashMap::new();
    let mut requires: Vec<String> = Vec::new();
    let mut re_exports: Vec<String> = Vec::new();
    let mut exports: Vec<ExportEntry> = Vec::new();
    let mut body_stmts: Vec<Statement<'a>> = Vec::new();
    let mut default_needs_rewrite = false;
    // (exported, local_name, local_symbol) for `export { a, b as c }` — resolved against `imports`
    // after all imports are collected.
    let mut deferred_named: Vec<DeferredNamed> = Vec::new();

    let stmts = std::mem::replace(&mut program.body, ast.vec());
    let mut seq = 0usize;

    for stmt in stmts {
        match stmt {
            Statement::ImportDeclaration(decl) => {
                seq += 1;
                collect_import(&decl, seq, ctx, &mut imports, &mut requires);
            }
            Statement::ExportNamedDeclaration(decl) => {
                seq += 1;
                if !collect_export_named(
                    scoping,
                    decl.unbox(),
                    seq,
                    ctx,
                    &mut requires,
                    (&mut exports, &mut deferred_named),
                    &mut body_stmts,
                ) {
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
                collect_export_all(ctx, seq, decl, &mut requires, &mut re_exports, &mut exports);
            }
            other => body_stmts.push(other),
        }
    }

    for (exported, local, sym) in deferred_named {
        let value_js = match sym.and_then(|s| imports.get(&s)) {
            Some(access) => member_js(&access.var, &access.prop),
            None => local,
        };
        exports.push(ExportEntry {
            name: exported,
            value_js,
        });
    }

    Some(Plan {
        imports,
        requires,
        re_exports,
        exports,
        body_stmts,
        default_needs_rewrite,
    })
}

/// Handle one `export * [as ns] from "src"` declaration: appends the right lines to `requires` /
/// `re_exports` / `exports`.
fn collect_export_all(
    ctx: &LowerCtx,
    seq: usize,
    decl: ABox<'_, oxc_ast::ast::ExportAllDeclaration>,
    requires: &mut Vec<String>,
    re_exports: &mut Vec<String>,
    exports: &mut Vec<ExportEntry>,
) {
    let src = decl.source.value.as_str();
    match &decl.exported {
        // `export * as ns from "s"` → named export of the namespace object.
        Some(name) => {
            let (prelude, expr) = module_namespace_expr(ctx, src, seq, true);
            let var = import_var(src, seq);
            requires.extend(prelude);
            requires.push(format!("var {var} = {expr};"));
            exports.push(ExportEntry {
                name: module_export_name(name),
                value_js: var,
            });
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

/// An expression evaluating to a module's namespace object, plus any prelude lines needed first.
/// `toesm` wraps external/CJS results with `__toESM` (default-interop); a bundled ESM module's
/// exports object already has the right getters so it's used directly.
fn module_namespace_expr(
    ctx: &LowerCtx,
    src: &str,
    _seq: usize,
    toesm: bool,
) -> (Vec<String>, String) {
    match ctx.source_ref(src) {
        SourceRef::External => {
            let r = require_js(src);
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
                    imports.insert(
                        sym,
                        Access {
                            var: var.to_string(),
                            prop: "default".into(),
                        },
                    );
                }
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                if let Some(sym) = s.local.symbol_id.get() {
                    imports.insert(
                        sym,
                        Access {
                            var: var.to_string(),
                            prop: module_export_name(&s.imported),
                        },
                    );
                }
            }
        }
    }
}

/// Render one ImportDeclaration into require/init line(s) + import-map entries, under `ctx`.
/// Every import form is handled.
fn collect_import(
    decl: &ImportDeclaration,
    seq: usize,
    ctx: &LowerCtx,
    imports: &mut HashMap<SymbolId, Access>,
    requires: &mut Vec<String>,
) {
    let src = decl.source.value.as_str();
    let sref = ctx.source_ref(src);

    let Some(specifiers) = &decl.specifiers else {
        // side-effect import: `import "s"`
        match &sref {
            SourceRef::External => {
                let var = import_var(src, seq);
                requires.push(format!("var {var} = {};", require_js(src)));
            }
            SourceRef::BundledEsm { id } => requires.push(format!("init_{id}();")),
            SourceRef::BundledCjs { id } => requires.push(format!("require_{id}();")),
        }
        return;
    };

    let has_default_or_ns = specifiers.iter().any(|s| {
        matches!(
            s,
            ImportDeclarationSpecifier::ImportDefaultSpecifier(_)
                | ImportDeclarationSpecifier::ImportNamespaceSpecifier(_)
        )
    });
    let ns_local = specifiers.iter().find_map(|s| match s {
        ImportDeclarationSpecifier::ImportNamespaceSpecifier(n) => Some(n.local.name.to_string()),
        _ => None,
    });

    match &sref {
        // bundled ESM: refs go to `<id>_exports.<name>`; ensure init; bind a namespace local to it.
        SourceRef::BundledEsm { id } => {
            collect_import_bundled_esm(id, &ns_local, specifiers, imports, requires);
        }
        // bundled CJS: `var v = require_<id>()` (or __toESM for default/ns), refs `v.name`.
        SourceRef::BundledCjs { id } => {
            let var = ns_local.unwrap_or_else(|| import_var(src, seq));
            let expr = if has_default_or_ns {
                format!("__toESM(require_{id}())")
            } else {
                format!("require_{id}()")
            };
            requires.push(format!("var {var} = {expr};"));
            map_var_specs(specifiers, &var, imports);
        }
        // external (bare, or app mode): runtime `require("spec")`.
        SourceRef::External => {
            let var = ns_local.unwrap_or_else(|| import_var(src, seq));
            let expr = if has_default_or_ns {
                format!("__toESM({})", require_js(src))
            } else {
                require_js(src)
            };
            requires.push(format!("var {var} = {expr};"));
            map_var_specs(specifiers, &var, imports);
        }
    }
}

/// Emit the require/init line and imports map for a BundledEsm source (`<id>_exports.<name>`).
fn collect_import_bundled_esm(
    id: &str,
    ns_local: &Option<String>,
    specifiers: &[ImportDeclarationSpecifier],
    imports: &mut HashMap<SymbolId, Access>,
    requires: &mut Vec<String>,
) {
    let exports_obj = format!("{id}_exports");
    if let Some(local) = ns_local {
        requires.push(format!("var {local} = (init_{id}(), {exports_obj});"));
    } else {
        requires.push(format!("init_{id}();"));
    }
    for spec in specifiers {
        match spec {
            ImportDeclarationSpecifier::ImportDefaultSpecifier(d) => {
                if let Some(sym) = d.local.symbol_id.get() {
                    imports.insert(
                        sym,
                        Access {
                            var: exports_obj.clone(),
                            prop: "default".into(),
                        },
                    );
                }
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                if let Some(sym) = s.local.symbol_id.get() {
                    imports.insert(
                        sym,
                        Access {
                            var: exports_obj.clone(),
                            prop: module_export_name(&s.imported),
                        },
                    );
                }
            }
        }
    }
}

/// An `export { a as b }` name: (exported, local name, local symbol).
type DeferredNamed = (String, String, Option<SymbolId>);

/// `export <decl>` / `export { ... } [from "s"]`.
fn collect_export_named<'a>(
    scoping: &Scoping,
    decl: ExportNamedDeclaration<'a>,
    seq: usize,
    ctx: &LowerCtx,
    requires: &mut Vec<String>,
    // the export entries, and the `export { a as b }` names resolved once all imports are known
    (exports, deferred_named): (&mut Vec<ExportEntry>, &mut Vec<DeferredNamed>),
    body_stmts: &mut Vec<Statement<'a>>,
) -> bool {
    if let Some(d) = decl.declaration {
        return emit_export_decl(d, exports, body_stmts);
    }
    if let Some(source) = &decl.source {
        // `export { a, b as c } from "s"` — reference the module, getters read from it.
        let src = source.value.as_str();
        // bind the module's namespace to a local var, then getters read `var.local`.
        let var = import_var(src, seq + 1_000_000);
        // re-exporting `default` reads it through the ES-module interop, as an import of it does
        // (a CommonJS module's default is its `module.exports`, turbo-test#122)
        let reads_default = decl
            .specifiers
            .iter()
            .any(|spec| module_export_name(&spec.local) == "default");
        let (prelude, expr) = module_namespace_expr(ctx, src, seq, reads_default);
        requires.extend(prelude);
        requires.push(format!("var {var} = {expr};"));
        for spec in &decl.specifiers {
            let exported = module_export_name(&spec.exported);
            let local = module_export_name(&spec.local);
            exports.push(ExportEntry {
                name: exported,
                value_js: member_js(&var, &local),
            });
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

/// Append each binding name in `d` to `exports` and re-emit the declaration as a statement.
/// Returns `false` for a TS-only declaration that shouldn't survive the strip.
fn emit_export_decl<'a>(
    d: Declaration<'a>,
    exports: &mut Vec<ExportEntry>,
    body_stmts: &mut Vec<Statement<'a>>,
) -> bool {
    for name in declaration_binding_names(&d) {
        exports.push(ExportEntry {
            name: name.clone(),
            value_js: name,
        });
    }
    match d {
        Declaration::VariableDeclaration(v) => body_stmts.push(Statement::VariableDeclaration(v)),
        Declaration::FunctionDeclaration(f) => body_stmts.push(Statement::FunctionDeclaration(f)),
        Declaration::ClassDeclaration(c) => body_stmts.push(Statement::ClassDeclaration(c)),
        _ => return false, // TS-only decl (enum/interface/type) shouldn't survive the strip
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
        ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
            f.id.as_ref().map(|i| i.name.to_string())
        }
        ExportDefaultDeclarationKind::ClassDeclaration(c) => {
            c.id.as_ref().map(|i| i.name.to_string())
        }
        _ => None,
    };
    if let Some(name) = named {
        let inner = decl.unbox();
        match inner.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                body_stmts.push(Statement::FunctionDeclaration(f))
            }
            ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                body_stmts.push(Statement::ClassDeclaration(c))
            }
            _ => unreachable!(),
        }
        exports.push(ExportEntry {
            name: "default".into(),
            value_js: name,
        });
        return false;
    }
    // anonymous func/class OR an expression: keep the node (so the visitor rewrites inner import
    // refs), register default → __tt_default, signal the codegen prefix-swap.
    body_stmts.push(Statement::ExportDefaultDeclaration(decl));
    exports.push(ExportEntry {
        name: "default".into(),
        value_js: "__tt_default".into(),
    });
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
            f.id.as_ref()
                .map(|id| vec![id.name.to_string()])
                .unwrap_or_default()
        }
        Declaration::ClassDeclaration(c) => {
            c.id.as_ref()
                .map(|id| vec![id.name.to_string()])
                .unwrap_or_default()
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

/// The identity-preserving `__toESM` helper line: runtime.js's `__ttToESM` (vitest's interop,
/// turbo-test#122). Part of `PREAMBLE`; also prepended alone to a script (no module syntax) whose
/// lowered `import()` calls it.
macro_rules! to_esm_helper {
    () => {
        "var __toESM = (mod) => globalThis.__ttToESM(mod);\n"
    };
}
pub(crate) const TO_ESM_HELPER: &str = to_esm_helper!();

/// What `export * from` re-exports (the `__reExport` source): the module's namespace as an
/// importer sees it (`__toESM`'s), so a package's CommonJS gives Node's names (with
/// `'module.exports'`) and an ES module no `__esModule` (turbo-test#133). Part of `PREAMBLE`.
macro_rules! star_helper {
    () => {
        "var __toESMStar = (mod) => __toESM(mod);\n"
    };
}
pub(crate) const STAR_HELPER: &str = star_helper!();

/// Runtime helper preamble (shared by app emit + the bundler — emitted ONCE per output). `__esm`
/// and `__commonJS` are the lazy module-init wrappers the bundler uses so circular dependencies +
/// init ordering work (each module's body runs once, on first require, with its exports object
/// already live).
pub(crate) const PREAMBLE: &str = concat!(
    r#"var __defProp = Object.defineProperty;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => { for (var name in all) __defProp(target, name, { get: all[name], enumerable: true, configurable: true }); };
var __copyProps = (to, from, except, desc) => { if (from && typeof from === "object" || typeof from === "function") { for (let key of __getOwnPropNames(from)) if (!__hasOwnProp.call(to, key) && key !== except) __defProp(to, key, { get: () => from[key], ...globalThis.__ttExportSetter?.(to, key), enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable, configurable: true }); } return to; };
var __reExport = (target, mod, secondTarget) => (mod = __toESMStar(mod), __copyProps(target, mod, "default"), secondTarget && __copyProps(secondTarget, mod, "default"));
"#,
    to_esm_helper!(),
    star_helper!(),
    r#"var __toCommonJS = (mod) => __copyProps(__defProp(__defProp({}, "__esModule", { value: true, configurable: true }), Symbol.for("turbo-test.esm"), { value: true }), mod);
var __commonJS = (cb) => { var mod; return () => { if (!mod) { mod = { exports: {} }; try { cb(mod.exports, mod, __export, __toCommonJS, __toESM, __reExport); } catch (e) { mod = undefined; throw e; } } return mod.exports; }; };
var __esm = (fn) => { var done, res; return () => (done || (done = 1, res = fn()), res); };
"#
);

/// Assemble one module's CJS text from its parts — NO preamble (the caller emits that once).
/// `module.exports = __toCommonJS(__tt_exports)` is written EARLY (before requires + body) so that
/// under circular requires a re-entrant `require_<id>()` sees the live exports object. Order mirrors
/// esbuild so the loader + `hoist_mock_setup` see the expected shape.
pub(crate) fn assemble_module(
    requires: &[String],
    re_exports: &[String],
    exports: &[ExportEntry],
    body: &str,
) -> String {
    let mut out = String::with_capacity(body.len() + 512);
    out.push_str("var __tt_exports = {};\n");
    if !exports.is_empty() {
        out.push_str("__export(__tt_exports, {\n");
        for e in exports {
            out.push_str(&format!(
                "  {}: () => {},\n",
                quote_key(&e.name),
                e.value_js
            ));
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
fn assemble(
    requires: &[String],
    re_exports: &[String],
    exports: &[ExportEntry],
    body: &str,
) -> String {
    let mut out = String::with_capacity(PREAMBLE.len() + body.len() + 512);
    out.push_str(PREAMBLE);
    out.push_str(&assemble_module(requires, re_exports, exports, body));
    out
}

/// Whether `name` can be written bare as an object key or after `.` (an ASCII identifier name;
/// reserved words are fine there). Conservative: non-ASCII identifiers are quoted, still valid.
fn is_plain_ident(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// An object key for an export name: bare if an identifier, else a JS string literal (a JSON
/// string is one; ES module export names are well-formed Unicode, so no lone surrogates).
pub(crate) fn quote_key(name: &str) -> String {
    if is_plain_ident(name) {
        name.to_string()
    } else {
        json_string(name)
    }
}

/// `require("<spec>")` source text, the specifier as an escaped string literal (a JSON string is
/// one; a raw `"` or `\` in the specifier would otherwise end the literal or start an escape).
fn require_js(spec: &str) -> String {
    format!("require({})", json_string(spec))
}

/// Source text reading property `prop` of `obj`: `obj.prop`, or `obj["a-b"]` for a string export
/// name that isn't an identifier (`obj.a-b` would parse as `obj.a - b`).
fn member_js(obj: &str, prop: &str) -> String {
    if is_plain_ident(prop) {
        format!("{obj}.{prop}")
    } else {
        format!("{obj}[{}]", json_string(prop))
    }
}

/// An ES module's (plain JS) export names in source order, as vitest's namespace lists them
/// (turbo-test#141); `export *` names are not known here. `None` when it does not parse.
pub(crate) fn export_order(src: &str) -> Option<Vec<String>> {
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, SourceType::mjs().with_jsx(true)).parse();
    if ret.panicked || !ret.errors.is_empty() {
        return None;
    }
    let mut names: Vec<String> = Vec::new();
    for stmt in &ret.program.body {
        match stmt {
            Statement::ExportNamedDeclaration(d) => {
                if let Some(decl) = &d.declaration {
                    names.extend(declaration_binding_names(decl));
                }
                names.extend(d.specifiers.iter().map(|s| module_export_name(&s.exported)));
            }
            Statement::ExportDefaultDeclaration(_) => names.push("default".to_string()),
            Statement::ExportAllDeclaration(d) => {
                if let Some(n) = &d.exported {
                    names.push(module_export_name(n));
                }
            }
            _ => {}
        }
    }
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    Some(names)
}

/// The query suffix of an ES-module-graph request for the namespace (vitest's interop object)
/// of a CommonJS module, `import * as ns` / `export * as ns` (turbo-test#133).
pub(crate) const NS_QUERY: &str = "?__tt_ns";
/// ... for what `export * from` a CommonJS module re-exports (that namespace's names).
pub(crate) const STAR_QUERY: &str = "?__tt_star";
/// ... for a CommonJS module imported as Node imports it (an ES module Node runs as one).
pub(crate) const NODE_QUERY: &str = "?__tt_node";

/// What a graph module's request reaches (`graph_interop_rewrite`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Target {
    /// a CommonJS module
    Cjs,
    /// an app's ES module (vitest lists its namespace's names in source order)
    AppEsm,
    /// anything else (left as it is)
    Other,
}

/// Rewrite an ES module's (plain JS) requests of CommonJS modules (`target(specifier)`) for the
/// ES-module graph loader, whose CommonJS modules are synthetic modules: a V8 namespace object
/// cannot be vitest's interop namespace (`__esModule` read but not listed, `'module.exports'`),
/// so `import * as ns` / `export * as ns` read one as the default export of `spec?__tt_ns`,
/// `export *` re-exports from `spec?__tt_star`; in an ES module Node runs as one (`node`), every
/// request of a CommonJS module is `spec?__tt_node`, Node's namespace of it (turbo-test#133).
/// An app ES module's namespace (`import * as` / `export * as`) is read the same way, after a
/// plain import of the module (which evaluates it first): vitest's lists the names in source
/// order (turbo-test#141). Line breaks are kept. `None` when nothing is rewritten.
pub(crate) fn graph_interop_rewrite(
    src: &str,
    target: &dyn Fn(&str) -> Target,
    node: bool,
) -> Option<String> {
    if !interop_candidate(src, node) {
        return None;
    }
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, SourceType::mjs()).parse();
    if ret.panicked || !ret.errors.is_empty() {
        return None;
    }
    let kind = |spec: &str| {
        if spec.contains('?') {
            Target::Other
        } else {
            target(spec)
        }
    };
    // (start, end, replacement), in source order
    let edits: Vec<(u32, u32, String)> = ret
        .program
        .body
        .iter()
        .filter_map(|stmt| interop_edit(stmt, &kind, node))
        .collect();
    if edits.is_empty() {
        return None;
    }
    Some(apply_interop_edits(src, edits))
}

/// Fast guard: a source worth scanning for `graph_interop_rewrite` edits.
fn interop_candidate(src: &str, node: bool) -> bool {
    (src.contains("import") || src.contains("export")) && (node || src.contains('*'))
}

/// Splice `edits` into `src`, keeping the replaced text's line breaks so later line numbers stay.
fn apply_interop_edits(src: &str, edits: Vec<(u32, u32, String)>) -> String {
    let mut out = String::with_capacity(src.len() + edits.len() * 16);
    let mut at = 0usize;
    for (start, end, text) in edits {
        let (start, end) = (start as usize, end as usize);
        out.push_str(&src[at..start]);
        out.push_str(&text);
        // keep the replaced text's line breaks, so later lines keep their numbers
        out.extend(src[start..end].matches('\n').map(|_| '\n'));
        at = end;
    }
    out.push_str(&src[at..]);
    out
}

/// `spec` with the query `q`, as a JS string literal.
fn lit(spec: &str, q: &str) -> String {
    json_string(&format!("{spec}{q}"))
}

/// The [`graph_interop_rewrite`] edit of one top-level statement, if it needs one.
fn interop_edit(
    stmt: &Statement,
    kind: &dyn Fn(&str) -> Target,
    node: bool,
) -> Option<(u32, u32, String)> {
    match stmt {
        Statement::ImportDeclaration(d) if d.with_clause.is_none() => {
            import_decl_edit(d, kind, node)
        }
        Statement::ExportAllDeclaration(d) if d.with_clause.is_none() => {
            export_all_decl_edit(d, kind, node)
        }
        Statement::ExportNamedDeclaration(d) if node && d.with_clause.is_none() => {
            let source = d.source.as_ref()?;
            let spec = source.value.as_str();
            (kind(spec) == Target::Cjs)
                .then(|| (source.span.start, source.span.end, lit(spec, NODE_QUERY)))
        }
        _ => None,
    }
}

/// `interop_edit` for an `import …` declaration.
fn import_decl_edit(
    d: &ImportDeclaration,
    kind: &dyn Fn(&str) -> Target,
    node: bool,
) -> Option<(u32, u32, String)> {
    let spec = d.source.value.as_str();
    let k = kind(spec);
    if k == Target::Other || (node && k != Target::Cjs) {
        return None;
    }
    if node {
        return Some((
            d.source.span.start,
            d.source.span.end,
            lit(spec, NODE_QUERY),
        ));
    }
    import_ns_edit(d, spec, k)
}

/// `interop_edit` for an `export * …` declaration.
fn export_all_decl_edit(
    d: &ExportAllDeclaration,
    kind: &dyn Fn(&str) -> Target,
    node: bool,
) -> Option<(u32, u32, String)> {
    let spec = d.source.value.as_str();
    let k = kind(spec);
    if k == Target::Other || (node && k != Target::Cjs) {
        return None;
    }
    if node {
        return Some((
            d.source.span.start,
            d.source.span.end,
            lit(spec, NODE_QUERY),
        ));
    }
    export_all_edit(d, spec, k)
}

/// `import * as ns` (with its default import, if any) of the module `spec` of kind `k`.
fn import_ns_edit(d: &ImportDeclaration, spec: &str, k: Target) -> Option<(u32, u32, String)> {
    let specs = d.specifiers.as_ref()?;
    let ns = specs.iter().find_map(|s| match s {
        ImportDeclarationSpecifier::ImportNamespaceSpecifier(n) => Some(n.local.name.to_string()),
        _ => None,
    })?;
    let default = specs.iter().find_map(|s| match s {
        ImportDeclarationSpecifier::ImportDefaultSpecifier(n) => Some(n.local.name.to_string()),
        _ => None,
    });
    let mut text = String::new();
    if let Some(def) = default {
        text.push_str(&format!("import {def} from {}; ", json_string(spec)));
    } else if k == Target::AppEsm {
        text.push_str(&format!("import {}; ", json_string(spec)));
    }
    text.push_str(&format!("import {ns} from {};", lit(spec, NS_QUERY)));
    Some((d.span.start, d.span.end, text))
}

/// `export *` / `export * as name` from the module `spec` of kind `k`.
fn export_all_edit(d: &ExportAllDeclaration, spec: &str, k: Target) -> Option<(u32, u32, String)> {
    let Some(name) = &d.exported else {
        return (k == Target::Cjs).then(|| {
            (
                d.source.span.start,
                d.source.span.end,
                lit(spec, STAR_QUERY),
            )
        });
    };
    let name = module_export_name(name);
    let name = if is_plain_ident(&name) {
        name
    } else {
        json_string(&name)
    };
    let first = if k == Target::AppEsm {
        format!("import {}; ", json_string(spec))
    } else {
        String::new()
    };
    Some((
        d.span.start,
        d.span.end,
        format!(
            "{first}export {{ default as {name} }} from {};",
            lit(spec, NS_QUERY)
        ),
    ))
}

/// The export names Node's cjs-module-lexer detects in a CommonJS module whose one export form
/// is `module.exports = { ... }` (turbo-test#133): the object literal's keys up to the first
/// property whose value is not an identifier (`{ a, b: c, 'd': e, ...f }`; `{ a: 1 }` gives none).
/// `None` for any other module, whose own enumerable names stand for the lexer's.
pub(crate) fn cjs_literal_export_names(src: &str) -> Option<Vec<String>> {
    let b = exports_object_literal(src)?;
    let mut sc = KeyScan { b, i: 0 };
    let mut names = Vec::new();
    loop {
        sc.ws();
        if sc.at_end_or(b'}') {
            break;
        }
        if !scan_one_entry(&mut sc, &mut names)? {
            break;
        }
        if !sc.eat(b',') {
            break;
        }
    }
    Some(names)
}

/// Scan one `{ ... }` entry, extend `names` with its exported key(s). Returns `false` to stop the
/// whole scan (an unsupported value form), `true` to continue.
fn scan_one_entry(sc: &mut KeyScan, names: &mut Vec<String>) -> Option<bool> {
    let key = match sc.entry()? {
        Entry::Stop => return Some(false),
        Entry::Spread => None,
        Entry::Key(k) => Some(k),
    };
    sc.ws();
    if let Some(k) = key {
        if !sc.value() {
            return Some(false);
        }
        names.extend(k);
    }
    Some(true)
}

/// The text after the `{` of a module's one export form `module.exports = {`.
fn exports_object_literal(src: &str) -> Option<&[u8]> {
    if !sole_module_exports_write(src) {
        return None;
    }
    let at = src.find("module.exports")? + "module.exports".len();
    let rest = src[at..].trim_start().strip_prefix('=')?;
    if rest.starts_with('=') {
        return None;
    }
    Some(rest.trim_start().strip_prefix('{')?.as_bytes())
}

/// Whether `src`'s only CommonJS export form is a single `module.exports = ...`.
fn sole_module_exports_write(src: &str) -> bool {
    src.matches("module.exports").count() == 1
        && !src.contains("exports.")
        && !src.contains("exports[")
        && !src.contains("defineProperty(exports")
        && !src.contains("__export")
}

/// A cursor over an object literal's text for [`cjs_literal_export_names`].
struct KeyScan<'b> {
    b: &'b [u8],
    i: usize,
}

impl KeyScan<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    /// Whether the text ends here or `c` is next.
    fn at_end_or(&self, c: u8) -> bool {
        self.i >= self.b.len() || self.b[self.i] == c
    }

    /// Step over `c` when it is next.
    fn eat(&mut self, c: u8) -> bool {
        let next = self.i < self.b.len() && self.b[self.i] == c;
        if next {
            self.i += 1;
        }
        next
    }

    fn ident(&mut self) -> Option<String> {
        let (b, s) = (self.b, self.i);
        let is_start = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c == b'$';
        let is_part = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
        if s >= b.len() || !is_start(b[s]) {
            return None;
        }
        while self.i < b.len() && is_part(b[self.i]) {
            self.i += 1;
        }
        Some(String::from_utf8_lossy(&b[s..self.i]).into_owned())
    }

    /// The next entry (`None` when the scan gives up).
    fn entry(&mut self) -> Option<Entry> {
        if self.b[self.i..].starts_with(b"...") {
            self.i += 3;
            self.ws();
            self.ident()?;
            return Some(Entry::Spread);
        }
        if self.b[self.i] == b'\'' || self.b[self.i] == b'"' {
            let (key, end) = string_key(self.b, self.i)?;
            self.i = end;
            return Some(Entry::Key(key));
        }
        Some(self.ident().map_or(Entry::Stop, |k| Entry::Key(Some(k))))
    }

    /// Past a key: its `: identifier` value, if any; `false` when the value is not an identifier.
    fn value(&mut self) -> bool {
        if self.eat(b':') {
            self.ws();
            if self.ident().is_none() {
                return false;
            }
            self.ws();
        }
        true
    }
}

/// An object literal entry as [`cjs_literal_export_names`] reads it.
enum Entry {
    /// `...name`
    Spread,
    /// a key (`None` when it is not a well-formed string)
    Key(Option<String>),
    /// anything else: the names stop here
    Stop,
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
        assert!(
            o.contains("var import_s_1 = __toESM(require(\"s\"));"),
            "{o}"
        );
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
        assert!(
            o.contains("__reExport(__tt_exports, require(\"s\"), module.exports)"),
            "{o}"
        );
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
        let o = emit(
            Path::new("t.ts"),
            "const m = await import(\"./x\"); m.default;",
        )
        .unwrap();
        assert!(
            o.contains("Promise.resolve().then(() => __toESM(require(\"./x\")))"),
            "{o}"
        );
        assert!(!o.contains(" import("), "raw import() must be gone: {o}");
    }
    #[test]
    fn dynamic_import_with_imported_specifier_var() {
        // import() whose arg references an imported binding gets the live access rewrite too.
        let o = emit(
            Path::new("t.ts"),
            "import { p } from \"s\"; await import(p);",
        )
        .unwrap();
        assert!(o.contains("require(__tt_spec))))(import_s_1.p)"), "{o}");
    }
}

/// The string literal starting at `b[at]` (its quote) as cjs-module-lexer reads an object key:
/// its escapes decoded (`\\<line break>` keeps the line break), `None` for a value that is not
/// well-formed UTF-16 (a lone surrogate); with the index past its closing quote. `None` when the
/// literal is unterminated (turbo-test#189).
fn string_key(b: &[u8], at: usize) -> Option<(Option<String>, usize)> {
    let q = b[at];
    let mut units: Vec<u16> = Vec::new();
    let mut i = at + 1;
    loop {
        let c = *b.get(i)?;
        if c == q {
            return Some((String::from_utf16(&units).ok(), i + 1));
        }
        advance_string_unit(b, q, &mut i, c, &mut units)?;
    }
}

/// Consume one character (plain run or escape) at `i`, appending to `units`; `None` if truncated.
fn advance_string_unit(b: &[u8], q: u8, i: &mut usize, c: u8, units: &mut Vec<u16>) -> Option<()> {
    if c != b'\\' {
        *i += extend_plain_run(b, q, *i, units)?;
        return Some(());
    }
    let e = *b.get(*i + 1)?;
    *i += 2;
    let cp = escape_code_point(b, i, e)?;
    push_code_point(units, cp);
    Some(())
}

/// Append a run of source text (from `i`, as UTF-16) until the next quote or escape, returning the
/// run's byte length.
fn extend_plain_run(b: &[u8], q: u8, i: usize, units: &mut Vec<u16>) -> Option<usize> {
    let len = b[i..].iter().position(|&c| c == q || c == b'\\')?;
    units.extend(String::from_utf8_lossy(&b[i..i + len]).encode_utf16());
    Some(len)
}

/// Push `cp` as UTF-16 code unit(s), falling back to the raw unit when it is a lone surrogate.
fn push_code_point(units: &mut Vec<u16>, cp: u32) {
    match char::from_u32(cp) {
        Some(ch) => units.extend(ch.encode_utf16(&mut [0; 2]).iter()),
        None => units.push(cp as u16),
    }
}

/// The code point of the escape `\e` whose text follows at `*i` (moved past it).
fn escape_code_point(b: &[u8], i: &mut usize, e: u8) -> Option<u32> {
    if let Some(cp) = simple_escape_code_point(e) {
        return Some(cp);
    }
    escape_code_point_extended(b, i, e)
}

/// Single-char escapes (`\n`, `\t`, `\0`, …), or `None` to defer to the extended form.
fn simple_escape_code_point(e: u8) -> Option<u32> {
    Some(match e {
        b'n' => 0x0a,
        b't' => 0x09,
        b'r' => 0x0d,
        b'b' => 0x08,
        b'f' => 0x0c,
        b'v' => 0x0b,
        b'0' => 0,
        _ => return None,
    })
}

/// Variable-length escapes (`\xHH`, `\uHHHH`, `\u{H+}`) and the identity fallback.
fn escape_code_point_extended(b: &[u8], i: &mut usize, e: u8) -> Option<u32> {
    match e {
        b'x' => fixed_hex_escape(b, i, 2),
        b'u' => unicode_escape_code_point(b, i),
        _ => identity_escape(b, i),
    }
}

fn fixed_hex_escape(b: &[u8], i: &mut usize, n: usize) -> Option<u32> {
    *i += n;
    hex_u32(b.get(*i - n..*i)?)
}

fn unicode_escape_code_point(b: &[u8], i: &mut usize) -> Option<u32> {
    if b.get(*i) == Some(&b'{') {
        let len = b[*i..].iter().position(|&c| c == b'}')?;
        let cp = hex_u32(&b[*i + 1..*i + len])?;
        *i += len + 1;
        return Some(cp);
    }
    fixed_hex_escape(b, i, 4)
}

/// Any other character stands for itself (a line break included).
fn identity_escape(b: &[u8], i: &mut usize) -> Option<u32> {
    let ch = String::from_utf8_lossy(&b[*i - 1..]).chars().next()?;
    *i += ch.len_utf8() - 1;
    Some(ch as u32)
}

fn hex_u32(s: &[u8]) -> Option<u32> {
    u32::from_str_radix(std::str::from_utf8(s).ok()?, 16).ok()
}

#[cfg(test)]
#[path = "esm_cjs_tests.rs"]
mod tests_more;
