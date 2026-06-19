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

/// Neutralize `design:type` decorator metadata that references an IMPORTED name used ONLY as a
/// type (never as a runtime value elsewhere in the module). Both oxc and `ts.transpileModule` are
/// single-file (no type checker), so for `@Column() declare x: SomeImportedIface` they keep the
/// type-only import and reference it in metadata — but `SomeImportedIface` is an interface with no
/// runtime export, so V8 fails ESM instantiation (`does not provide an export named …`). When the
/// imported name appears ONLY in `design:type`/`design:returntype` metadata (tsc form
/// `("design:type", IDENT)` or oxc's runtime-safe guard `typeof IDENT === "undefined" ? Object :
/// IDENT`), we replace the metadata reference with `Object` and drop the now-unused import
/// specifier — exactly what tsc-with-a-type-checker would emit for an interface.
///
/// Conservative by construction:
///   - Only NAMED imports (not default/namespace).
///   - Only a name whose every non-import occurrence is a `design:type`/`design:returntype` ref —
///     a name used as a real value (incl. anywhere else) is left untouched.
///   - `design:paramtypes` (constructor DI) is intentionally NOT touched: those are real injected
///     classes; downgrading them to `Object` would break Nest DI metadata.
///   - No-op unless the code contains decorator metadata, so non-decorated files are unchanged.
pub fn strip_type_only_metadata_imports(code: &str) -> String {
    if !code.contains("design:type") {
        return code.to_string();
    }
    let imported = collect_named_import_bindings(code);
    if imported.is_empty() {
        return code.to_string();
    }
    let mut out = code.to_string();
    for name in imported {
        // Occurrences as a `design:type`/`design:returntype` metadata ref (tsc + oxc-guard forms).
        let tsc_type = format!("\"design:type\", {name})");
        let tsc_ret = format!("\"design:returntype\", {name})");
        let oxc_guard = format!("typeof {name} === \"undefined\" ? Object : {name}");
        let meta_hits = count_occurrences(&out, &tsc_type)
            + count_occurrences(&out, &tsc_ret)
            + count_occurrences(&out, &oxc_guard);
        if meta_hits == 0 {
            continue;
        }
        // A metadata-only name: total identifier uses == (uses inside the metadata refs) + (1 import
        // specifier occurrence). oxc guard mentions the name twice; tsc forms once.
        let ident_uses = count_word(&out, &name);
        let guard_mentions = 2 * count_occurrences(&out, &oxc_guard);
        let single_mentions =
            count_occurrences(&out, &tsc_type) + count_occurrences(&out, &tsc_ret);
        let meta_mentions = guard_mentions + single_mentions;
        // import specifier mention(s): the name appears once per import clause it's in (≥1).
        let import_mentions = import_specifier_count(&out, &name);
        if ident_uses != meta_mentions + import_mentions {
            // used as a real value somewhere → keep it (it has a runtime export, import resolves).
            continue;
        }
        // Neutralize the metadata refs → Object, then drop the import specifier.
        out = out.replace(&tsc_type, "\"design:type\", Object)");
        out = out.replace(&tsc_ret, "\"design:returntype\", Object)");
        out = out.replace(&oxc_guard, "Object");
        out = remove_import_specifier(&out, &name);
    }
    out
}

/// Local binding names introduced by `import { a, b as c } from "..."` (the LOCAL name — `a`, `c`).
fn collect_named_import_bindings(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut i = 0;
    while let Some(rel) = code[i..].find("import") {
        let at = i + rel;
        i = at + 6;
        // must be a statement-level `import` followed by a `{ … } from` clause on this statement
        let rest = &code[at..];
        let Some(brace_open) = rest.find('{') else { continue };
        let Some(brace_close) = rest[brace_open..].find('}') else { continue };
        let after = &rest[brace_open + brace_close..];
        if !after.trim_start().starts_with("} from") && !after.trim_start().starts_with("}from") {
            continue;
        }
        let clause = &rest[brace_open + 1..brace_open + brace_close];
        for part in clause.split(',') {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            // `X as Y` → local is Y; plain `X` → X.
            let local = p.rsplit(" as ").next().unwrap_or(p).trim();
            if !local.is_empty() && local.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                names.push(local.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn count_occurrences(hay: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    hay.matches(needle).count()
}

/// Whole-word occurrences of `name` (not a substring of a longer identifier, not `.name` access).
fn count_word(hay: &str, name: &str) -> usize {
    let b = hay.as_bytes();
    let mut n = 0;
    let mut i = 0;
    while let Some(rel) = hay[i..].find(name) {
        let at = i + rel;
        i = at + name.len();
        let before_ok = at == 0 || !is_ident_byte(b[at - 1]);
        let after = at + name.len();
        let after_ok = after >= b.len() || !is_ident_byte(b[after]);
        let not_member = at == 0 || b[at - 1] != b'.';
        if before_ok && after_ok && not_member {
            n += 1;
        }
    }
    n
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// How many import-clause specifier slots mention `name` (`{ … name … }`). Counts the LOCAL name.
fn import_specifier_count(code: &str, name: &str) -> usize {
    let mut n = 0;
    for binding in collect_named_import_bindings_with_dups(code) {
        if binding == name {
            n += 1;
        }
    }
    n
}

fn collect_named_import_bindings_with_dups(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut i = 0;
    while let Some(rel) = code[i..].find("import") {
        let at = i + rel;
        i = at + 6;
        let rest = &code[at..];
        let Some(bo) = rest.find('{') else { continue };
        let Some(bc) = rest[bo..].find('}') else { continue };
        let after = &rest[bo + bc..];
        if !after.trim_start().starts_with("} from") && !after.trim_start().starts_with("}from") {
            continue;
        }
        for part in rest[bo + 1..bo + bc].split(',') {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            let local = p.rsplit(" as ").next().unwrap_or(p).trim();
            if !local.is_empty() && local.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                names.push(local.to_string());
            }
        }
    }
    names
}

/// Remove the named specifier `name` from every `import { … } from "…"` clause; drop a clause that
/// becomes empty (whole import statement removed).
fn remove_import_specifier(code: &str, name: &str) -> String {
    let mut result = String::with_capacity(code.len());
    let mut rest = code;
    loop {
        let Some(rel) = rest.find("import") else {
            result.push_str(rest);
            break;
        };
        let stmt_start = rel;
        let after_import = &rest[stmt_start..];
        let Some(bo) = after_import.find('{') else {
            // no clause — copy through `import` and continue
            result.push_str(&rest[..stmt_start + 6]);
            rest = &rest[stmt_start + 6..];
            continue;
        };
        let Some(bc_rel) = after_import[bo..].find('}') else {
            result.push_str(&rest[..stmt_start + 6]);
            rest = &rest[stmt_start + 6..];
            continue;
        };
        let bc = bo + bc_rel;
        let tail = &after_import[bc..];
        if !tail.trim_start().starts_with("} from") && !tail.trim_start().starts_with("}from") {
            result.push_str(&rest[..stmt_start + 6]);
            rest = &rest[stmt_start + 6..];
            continue;
        }
        // find end of statement (the closing quote of the `from "..."` + optional `;`)
        let from_clause = &after_import[bc..];
        let q = from_clause.find(['"', '\'']);
        let Some(q1) = q else {
            result.push_str(&rest[..stmt_start + 6]);
            rest = &rest[stmt_start + 6..];
            continue;
        };
        let quote = from_clause.as_bytes()[q1];
        let Some(q2_rel) = from_clause[q1 + 1..].find(quote as char) else {
            result.push_str(&rest[..stmt_start + 6]);
            rest = &rest[stmt_start + 6..];
            continue;
        };
        let mut stmt_end = bc + q1 + 1 + q2_rel + 1; // index in after_import, past closing quote
        if after_import[stmt_end..].starts_with(';') {
            stmt_end += 1;
        }
        let kept: Vec<&str> = after_import[bo + 1..bc]
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter(|s| s.rsplit(" as ").next().unwrap_or(s).trim() != name)
            .collect();
        result.push_str(&rest[..stmt_start]);
        if !kept.is_empty() {
            let src = &after_import[bc + 1..]; // after the `}`
            let from_part = &src[..stmt_end - (bc + 1)];
            result.push_str(&format!("import {{ {} }}{}", kept.join(", "), from_part));
        }
        // empty clause → drop whole statement (emit nothing)
        rest = &after_import[stmt_end..];
    }
    result
}

/// Extensions that require transformation before V8 can compile them.
pub fn needs_transform(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts" | "tsx" | "jsx" | "mts" | "cts")
    )
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
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).map_err(|e| format!("source type: {e:?}"))?;

    let parsed = Parser::new(&allocator, src, source_type).parse();
    if !parsed.errors.is_empty() {
        return Err(format!("parse errors: {:?}", parsed.errors));
    }
    let mut program = parsed.program;

    // semantic pass produces the scoping the transformer needs for correct renaming
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();

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

#[cfg(test)]
mod strip_tests {
    use super::strip_type_only_metadata_imports;

    #[test]
    fn strips_tsc_design_type_interface_import() {
        // tsc form: imported interface used only in design:type metadata.
        let code = "import { EntityCountrySpecificData } from \"./entity.types\";\n\
                    import { Column } from \"sequelize-typescript\";\n\
                    __decorate([Column(), __metadata(\"design:type\", EntityCountrySpecificData)], E.prototype, \"x\", void 0);";
        let out = strip_type_only_metadata_imports(code);
        assert!(!out.contains("EntityCountrySpecificData"), "import + ref removed:\n{out}");
        assert!(out.contains("\"design:type\", Object)"), "metadata → Object:\n{out}");
        assert!(out.contains("import { Column } from \"sequelize-typescript\""), "other import kept:\n{out}");
    }

    #[test]
    fn strips_oxc_typeof_guard_form() {
        let code = "import { Shape } from \"./types\";\n\
                    babelHelpers.decorateMetadata(\"design:type\", typeof Shape === \"undefined\" ? Object : Shape);";
        let out = strip_type_only_metadata_imports(code);
        assert!(!out.contains("Shape"), "guarded type import removed:\n{out}");
        assert!(out.contains("\"design:type\", Object)"), "guard → Object:\n{out}");
    }

    #[test]
    fn keeps_name_used_as_a_real_value() {
        // A real value (class) used as a field type AND instantiated elsewhere → keep it.
        let code = "import { Widget } from \"./widget\";\n\
                    const w = new Widget();\n\
                    __decorate([Column(), __metadata(\"design:type\", Widget)], E.prototype, \"x\", void 0);";
        let out = strip_type_only_metadata_imports(code);
        assert!(out.contains("import { Widget } from \"./widget\""), "value import kept:\n{out}");
        assert!(out.contains("\"design:type\", Widget)"), "metadata kept:\n{out}");
    }

    #[test]
    fn keeps_one_drops_other_in_mixed_import() {
        let code = "import { Iface, makeThing } from \"./mod\";\n\
                    const t = makeThing();\n\
                    __decorate([Column(), __metadata(\"design:type\", Iface)], E.prototype, \"x\", void 0);";
        let out = strip_type_only_metadata_imports(code);
        assert!(out.contains("makeThing"), "value specifier kept:\n{out}");
        assert!(!out.contains("Iface"), "type specifier dropped:\n{out}");
        assert!(out.contains("\"design:type\", Object)"), "metadata → Object:\n{out}");
    }

    #[test]
    fn noop_without_metadata() {
        let code = "import { A } from \"./a\";\nexport const x = A;";
        assert_eq!(strip_type_only_metadata_imports(code), code);
    }

    #[test]
    fn leaves_paramtypes_untouched() {
        // constructor DI paramtypes must NOT be downgraded.
        let code = "import { Service } from \"./service\";\n\
                    __decorate([__metadata(\"design:paramtypes\", [Service])], E);";
        let out = strip_type_only_metadata_imports(code);
        assert!(out.contains("import { Service }"), "paramtypes class import kept:\n{out}");
        assert!(out.contains("[Service]"), "paramtypes untouched:\n{out}");
    }
}
