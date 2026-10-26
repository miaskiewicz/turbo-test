//! TS/JSX transform via oxc (spec §3/§M4). Reused as-is, not reimplemented.
//!
//! Strips TypeScript types and lowers TS-only constructs to plain JS the V8 module
//! loader can compile. This is the `transform()` hook the loaders call before handing
//! source to V8.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{HelperLoaderMode, Module, TransformOptions, Transformer};

/// Extensions that require transformation before V8 can compile them.
pub fn needs_transform(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts" | "tsx" | "jsx" | "mts" | "cts")
    )
}

/// The `Scoping` oxc's `Transformer` must be given: a semantic pass WITH TS enum member
/// evaluation. oxc's enum lowering reads the pre-computed member values from `Scoping`; without
/// `with_enum_eval(true)` it debug-asserts (debug builds) or, in release, treats every member
/// initializer as non-constant — so `enum E { A = 'a', B = A }` got a bogus reverse mapping
/// (`E[E["B"] = E.A] = "B"`) instead of tsc's `E["B"] = "a"`.
pub(crate) fn semantic_scoping(program: &oxc_ast::ast::Program) -> oxc_semantic::Scoping {
    SemanticBuilder::new()
        .with_enum_eval(true)
        .build(program)
        .semantic
        .into_scoping()
}

/// Transform a TS/JSX source file to plain JS. Returns generated code.
pub fn transform(path: &Path, src: &str) -> Result<String, String> {
    transform_with(path, src, false)
}

/// Like `transform`, but `legacy_decorators = true` lowers TypeScript **legacy** decorators +
/// `emitDecoratorMetadata` (NestJS / Sequelize-typescript / Mongoose). Without it, oxc's DEFAULT
/// options leave a decorator on an exported class as `export @Decorator class X {…}` — invalid JS
/// in any module/script context ("Unexpected token 'export'") — or, where they DO lower, with
/// 2022-standard semantics that re-read the class binding before initialization ("Cannot access
/// 'X' before initialization"). Legacy lowering matches tsc/ts-jest and emits the helpers as global
/// `babelHelpers.decorate/decorateParam/decorateMetadata` (External mode), which `runtime.js`
/// provides — so app `.ts` files loaded through the ESM graph (`load_graph` →`maybe_transform`) get
/// valid, runnable output with no esbuild/tsc dependency. The caller decides `legacy_decorators`
/// (project has `experimentalDecorators` AND the file uses a decorator).
pub fn transform_with(path: &Path, src: &str, legacy_decorators: bool) -> Result<String, String> {
    emit(path, src, legacy_decorators, false).map(|(code, _)| code)
}

/// `transform` with a source map: the code and the map's `mappings` (test locations,
/// location.rs).
pub fn transform_mapped(path: &Path, src: &str) -> Result<(String, String), String> {
    emit(path, src, false, true).map(|(code, map)| (code, map.unwrap_or_default()))
}

/// The `transform_with` pipeline; `with_map` also returns the codegen source map's `mappings`.
fn emit(
    path: &Path,
    src: &str,
    legacy_decorators: bool,
    with_map: bool,
) -> Result<(String, Option<String>), String> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).map_err(|e| format!("source type: {e:?}"))?;

    let parsed = Parser::new(&allocator, src, source_type).parse();
    if !parsed.errors.is_empty() {
        return Err(format!("parse errors: {:?}", parsed.errors));
    }
    let mut program = parsed.program;

    // semantic pass produces the scoping the transformer needs for correct renaming
    let scoping = semantic_scoping(&program);

    let mut options = TransformOptions::default();
    if legacy_decorators {
        // Match `transform_decorators_with_metadata` (and `esm_cjs::emit_with`): legacy decorators,
        // metadata, no strict-null elision, helpers as global `babelHelpers.*`.
        options.decorator.legacy = true;
        options.decorator.emit_decorator_metadata = true;
        options.decorator.strict_null_checks = false;
        options.helper_loader.mode = HelperLoaderMode::External;
    }
    let ret =
        Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
    if !ret.errors.is_empty() {
        return Err(format!("transform errors: {:?}", ret.errors));
    }

    let options = oxc_codegen::CodegenOptions {
        source_map_path: with_map.then(|| path.to_path_buf()),
        ..Default::default()
    };
    let out = Codegen::new().with_options(options).build(&program);
    Ok((out.code, out.map.map(|m| m.to_json().mappings)))
}

/// Transform a TS file lowering legacy decorators + `emitDecoratorMetadata` in-process via oxc
/// — esbuild cannot emit decorator metadata, which NestJS / Mongoose (`@Injectable`, `@Prop`,
/// `@Controller`) need at runtime: without `design:type` / `design:paramtypes` the class
/// decorators throw at load (e.g. "Cannot determine a type for the X field"). Returns ESM JS
/// (module syntax preserved) with helpers inlined; the runner runs a second esbuild
/// `--format=cjs` pass to convert it for the module loader. Requires `reflect-metadata` loaded
/// so `Reflect.metadata` exists.
pub fn transform_decorators_with_metadata(path: &Path, src: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).map_err(|e| format!("source type: {e:?}"))?;

    let parsed = Parser::new(&allocator, src, source_type).parse();
    if !parsed.errors.is_empty() {
        return Err(format!("parse errors: {:?}", parsed.errors));
    }
    let mut program = parsed.program;
    let scoping = semantic_scoping(&program);

    let mut options = TransformOptions::default();
    // oxc 0.134's CommonJS module transform is a no-op, so we leave imports as ESM and let a
    // second esbuild `--format=cjs` pass (in the runner) convert module syntax. oxc's job here is
    // purely TS + legacy decorators + metadata lowering.
    let _ = Module::CommonJS;
    options.decorator.legacy = true;
    options.decorator.emit_decorator_metadata = true;
    // Match TypeScript's metadata serialization: `T | null` / `T | undefined` (incl. optional
    // `x?: T`) emit the constructor of `T`, not `Object`. With oxc's `strict_null_checks = true`
    // those become `Object`, which breaks runtime type inference (Mongoose @Prop: "Cannot
    // determine a type for the X field"). `false` elides null/undefined to mirror tsc/ts-jest.
    options.decorator.strict_null_checks = false;
    // oxc 0.134 panics on Inline helper mode (`unreachable!`). Use External mode: the decorator
    // helpers are emitted as `babelHelpers.decorate / decorateParam / decorateMetadata(...)`,
    // which the runtime provides as a global `babelHelpers` object (standard tslib semantics).
    options.helper_loader.mode = HelperLoaderMode::External;

    let ret =
        Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
    if !ret.errors.is_empty() {
        return Err(format!("transform errors: {:?}", ret.errors));
    }
    Ok(Codegen::new().build(&program).code)
}

/// `code` (the transformed mock prepass, a classic script) with each `import.meta` replaced by
/// `import_meta_expr` (the entry's). Code that does not parse is returned unchanged.
pub fn replace_import_meta(code: &str, import_meta_expr: &str) -> String {
    use oxc_ast_visit::Visit;
    struct Metas(Vec<(u32, u32)>);
    impl<'a> Visit<'a> for Metas {
        fn visit_meta_property(&mut self, it: &oxc_ast::ast::MetaProperty<'a>) {
            if it.meta.name == "import" {
                self.0.push((it.span.start, it.span.end));
            }
        }
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
    if !parsed.errors.is_empty() {
        return code.to_string();
    }
    let mut metas = Metas(Vec::new());
    metas.visit_program(&parsed.program);
    let mut out = code.to_string();
    for (start, end) in metas.0.into_iter().rev() {
        out.replace_range(start as usize..end as usize, import_meta_expr);
    }
    out
}

/// Whether Node loads `src` (a `.js` file with no package.json `"type"`) as an ES module — its
/// syntax detection: an `import` / `export` declaration or `import.meta`, or code that only
/// parses as a module (top-level `await`). Anything else is CommonJS; words in comments and
/// strings do not count.
pub fn has_module_syntax(src: &str) -> bool {
    let parses = |source_type: SourceType| {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, src, source_type.with_jsx(true)).parse();
        (
            parsed.module_record.has_module_syntax,
            parsed.errors.is_empty(),
        )
    };
    let (module_syntax, _) = parses(SourceType::unambiguous());
    module_syntax || (!parses(SourceType::cjs()).1 && parses(SourceType::mjs()).1)
}

/// Transform only when needed; otherwise return source unchanged.
pub fn maybe_transform(path: &Path, src: String) -> Result<String, String> {
    if needs_transform(path) {
        transform(path, &src)
    } else {
        Ok(src)
    }
}

#[cfg(test)]
#[path = "transform_tests.rs"]
mod tests;
