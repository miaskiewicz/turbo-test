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
use oxc_transformer::{TransformOptions, Transformer};

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

/// Transform only when needed; otherwise return source unchanged.
pub fn maybe_transform(path: &Path, src: String) -> Result<String, String> {
    if needs_transform(path) {
        transform(path, &src)
    } else {
        Ok(src)
    }
}
