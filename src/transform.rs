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

/// Transform a TS/JSX source file to plain JS. Returns generated code.
pub fn transform(path: &Path, src: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).map_err(|e| format!("source type: {e:?}"))?;

    let parsed = Parser::new(&allocator, src, source_type).parse();
    if !parsed.errors.is_empty() {
        return Err(format!("parse errors: {:?}", parsed.errors));
    }
    let mut program = parsed.program;

    // semantic pass produces the scoping the transformer needs for correct renaming
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();

    let options = TransformOptions::default();
    let ret =
        Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
    if !ret.errors.is_empty() {
        return Err(format!("transform errors: {:?}", ret.errors));
    }

    Ok(Codegen::new().build(&program).code)
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
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();

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

/// Transform only when needed; otherwise return source unchanged.
pub fn maybe_transform(path: &Path, src: String) -> Result<String, String> {
    if needs_transform(path) {
        transform(path, &src)
    } else {
        Ok(src)
    }
}
