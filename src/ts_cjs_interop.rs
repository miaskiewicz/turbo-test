//! TypeScript's explicit CommonJS import policy, including inherited JSONC options.
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Options {
    module: Option<Option<String>>,
    interop: Option<Option<bool>>,
}
impl Options {
    fn overlay(&mut self, other: Self) {
        self.module = other.module.or(self.module.take());
        self.interop = other.interop.or(self.interop);
    }
}
fn config(path: &Path) -> Option<Value> {
    let mut text = std::fs::read_to_string(path).ok()?;
    json_strip_comments::strip(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}
fn parent_path(path: &Path, spec: &str) -> Option<PathBuf> {
    if spec.starts_with('.') {
        let parent = path.parent()?.join(spec);
        return Some(if parent.is_file() {
            parent
        } else {
            PathBuf::from(format!("{}.json", parent.to_string_lossy()))
        });
    }
    oxc_resolver::Resolver::new(oxc_resolver::ResolveOptions {
        main_fields: vec!["tsconfig".into()],
        main_files: vec!["tsconfig".into()],
        extensions: vec![".json".into()],
        ..Default::default()
    })
    .resolve(path.parent()?, spec)
    .ok()
    .map(|result| result.full_path())
}
fn parent_specs(value: &Value) -> Vec<&str> {
    match &value["extends"] {
        Value::String(spec) => vec![spec.as_str()],
        Value::Array(specs) => specs.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}
fn enter_config(path: &Path, stack: &mut HashSet<PathBuf>) -> Option<PathBuf> {
    let path = crate::canon::canonicalize(path).ok()?;
    if stack.len() >= 32 || !stack.insert(path.clone()) {
        return None;
    }
    Some(path)
}
fn options(path: &Path, stack: &mut HashSet<PathBuf>) -> Option<Options> {
    let path = enter_config(path, stack)?;
    let value = config(&path)?;
    let mut result = Options::default();
    for spec in parent_specs(&value) {
        result.overlay(options(&parent_path(&path, spec)?, stack)?);
    }
    result.overlay(Options {
        module: value["compilerOptions"]
            .get("module")
            .map(|value| value.as_str().map(str::to_owned)),
        interop: value["compilerOptions"]
            .get("esModuleInterop")
            .map(Value::as_bool),
    });
    stack.remove(&path);
    Some(result)
}
pub(crate) fn enabled(config: &Path) -> bool {
    options(config, &mut HashSet::new()).is_some_and(|options| {
        options
            .module
            .flatten()
            .as_deref()
            .is_some_and(|module| module.eq_ignore_ascii_case("commonjs"))
            && options.interop == Some(Some(false))
    })
}

#[cfg(test)]
#[path = "ts_cjs_interop_tests.rs"]
mod tests;

fn literal_require(call: &oxc_ast::ast::CallExpression<'_>) -> bool {
    use oxc_ast::ast::{Argument, Expression};
    matches!(&call.callee, Expression::Identifier(id) if id.name == "require")
        && matches!(call.arguments.as_slice(), [Argument::StringLiteral(_)])
}
fn static_wrapper(call: &oxc_ast::ast::CallExpression<'_>) -> Option<oxc_span::Span> {
    use oxc_ast::ast::{Argument, Expression};
    if !matches!(&call.callee, Expression::Identifier(id) if id.name == "__toESM") {
        return None;
    }
    let [Argument::CallExpression(required)] = call.arguments.as_slice() else {
        return None;
    };
    literal_require(required).then_some(required.span)
}
fn generated_helper(binding: &oxc_ast::ast::VariableDeclarator<'_>, code: &str) -> bool {
    use oxc_ast::ast::{BindingPattern, Expression};
    let BindingPattern::BindingIdentifier(id) = &binding.id else {
        return false;
    };
    let Some(Expression::ArrowFunctionExpression(arrow)) = &binding.init else {
        return false;
    };
    let body = &code[arrow.span.start as usize..arrow.span.end as usize];
    id.name == "__toESM"
        && body.starts_with("(mod, isNodeMode, target) =>")
        && body.contains("__copyProps")
}
fn has_generated_helper(program: &oxc_ast::ast::Program<'_>, code: &str) -> bool {
    program.body.iter().any(|statement| match statement {
        oxc_ast::ast::Statement::VariableDeclaration(declaration) => declaration
            .declarations
            .iter()
            .any(|binding| generated_helper(binding, code)),
        _ => false,
    })
}
/// Remove only static-import wrappers; dynamic imports retain their namespace helper.
/// Whitespace replacements preserve generated line/column positions and esbuild source maps.
pub(crate) fn static_requires(code: String) -> String {
    use oxc_ast::ast::{Expression, Statement};
    let allocator = oxc_allocator::Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, &code, oxc_span::SourceType::cjs()).parse();
    if !has_generated_helper(&parsed.program, &code) {
        drop(parsed);
        return code;
    }
    let mut bytes = code.as_bytes().to_vec();
    for statement in &parsed.program.body {
        let Statement::VariableDeclaration(declaration) = statement else {
            continue;
        };
        for binding in &declaration.declarations {
            let Some(Expression::CallExpression(call)) = &binding.init else {
                continue;
            };
            if let Some(required) = static_wrapper(call) {
                bytes[call.span.start as usize..required.start as usize].fill(b' ');
                bytes[required.end as usize..call.span.end as usize].fill(b' ');
            }
        }
    }
    drop(parsed);
    String::from_utf8(bytes).unwrap_or(code)
}

#[path = "ts_cjs_resolver_fs.rs"]
mod resolver_fs;
pub(crate) use resolver_fs::ConfigFileSystem;
