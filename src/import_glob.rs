//! vite's `import.meta.glob` transform (turbo-test#130, #145), as vitest 4.1.12 runs it in tests:
//! a call by the full name with literal arguments is replaced, before any other transform, by an
//! object of the matching modules — eager (`{ eager: true }`: the module namespaces, imported
//! statically ahead of the module's own imports) or lazy (`() => import(...)`), each value the
//! module or its `import` export. Patterns are string literals (or an array of them; `!` negates)
//! relative to the module (`./`, `../`), the project root (`/`) or a `resolve.alias`; `base`
//! moves relative patterns and the keys to another directory; `query` (string or object) and the
//! deprecated `as` add a `?query` to each import. Keys are as vite writes them, sorted: relative
//! to `base`, else to the module for a relative pattern, else root-relative. As vite: the module
//! itself, dot files and `node_modules` are skipped unless `exhaustive: true`. A pattern it
//! cannot anchor fails the module with vite's "Invalid glob" error (thrown on its first line).
//! The options literal is evaluated as vite's `evalValue` does (spreads, computed keys, template
//! literals, `typeof`, arithmetic / comparison / logical operators, `?:`, member reads of objects,
//! strings, `globalThis` and the `Math` / `Number` constants; turbo-test#190, #195) and checked as
//! its parseGlobOptions does: arguments or options vite rejects fail the module the same way, with
//! vite's message ("Invalid glob import syntax: …", `Unknown glob option …`, a wrong type, a `base`
//! that is not anchored, a name that is "not static"). `caseSensitive: false` matches the patterns
//! case-insensitively. A call in the options fails the module with the first lines of vite's own
//! parse error (vite cuts the options at the call; turbo-test#197). A call whose options this does
//! not evaluate (a method, an array's `length`, a coercing `==`) is left alone and throws
//! vitest's error at run time (runtime.js).
//!
//! The replacement keeps the call's line count and the eager imports go on the first line, so
//! the module's line numbers (test locations, source maps) do not move.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrayExpression, ArrayExpressionElement, BinaryExpression, BinaryOperator, CallExpression,
    ComputedMemberExpression, ConditionalExpression, Expression, LogicalExpression,
    LogicalOperator, ObjectExpression, ObjectProperty, ObjectPropertyKind, PropertyKey,
    PropertyKind, StaticMemberExpression, TemplateLiteral, UnaryExpression, UnaryOperator,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::path::{Component, Path, PathBuf};

/// One `import.meta.glob(...)` call's literal arguments.
#[derive(Debug, PartialEq)]
struct Call {
    span: (u32, u32),
    patterns: Vec<String>,
    eager: bool,
    import: Option<String>,
    exhaustive: bool,
    /// the `?query` added to each import (`query`, or `as: X` -> `?X`)
    query: String,
    base: Option<String>,
    /// `caseSensitive` (default true): false matches the patterns case-insensitively
    case_sensitive: bool,
}

/// `src` (the module at `file`) with every replaceable `import.meta.glob` call expanded against
/// the file system, `root` the base of `/` patterns, `aliases` the config's `resolve.alias`
/// (find, replacement); `None` when there is none. The eager imports go first, on the first
/// line (after a hashbang), so they evaluate before the module's own imports, as vite's do.
pub fn expand(file: &Path, src: &str, root: &Path, aliases: &[(String, String)]) -> Option<String> {
    if !src.contains("glob") || !src.contains("import.meta") {
        return None;
    }
    let parsed = glob_calls(file, src);
    if parsed.is_empty() {
        return None;
    }
    let dir = file.parent()?;
    // vite fails the module at transform on a call whose arguments or options it rejects
    // (turbo-test#190), then on a pattern it cannot anchor (turbo-test#149): the module throws
    // vite's error when it is evaluated, before any of its code runs
    let (calls, invalid) = partition_parsed(parsed);
    let invalid = invalid.or_else(|| first_unanchored(&calls, dir, root, aliases));
    if let Some(message) = invalid {
        return expand_throw(src, &message);
    }
    expand_replace(src, file, dir, root, aliases, &calls)
}

/// Expand to a module that throws vite's error on first evaluation.
fn expand_throw(src: &str, message: &str) -> Option<String> {
    let mut out = src.to_string();
    let message = serde_json::to_string(message).ok()?;
    out.insert_str(after_hashbang(src), &format!("throw new Error({message});"));
    Some(out)
}

/// Expand each call into its `({...})` entries plus the eager imports (hoisted).
fn expand_replace(
    src: &str,
    file: &Path,
    dir: &Path,
    root: &Path,
    aliases: &[(String, String)],
    calls: &[Call],
) -> Option<String> {
    let mut out = src.to_string();
    let mut imports: Vec<String> = Vec::new();
    for (i, call) in calls.iter().enumerate().rev() {
        // every pattern and base anchors here (checked above and by glob_options)
        let found = matches(call, file, dir, root, aliases)?;
        let (entries, eager) = call_entries(call, i, &found)?;
        imports.push(eager);
        let (start, end) = (call.span.0 as usize, call.span.1 as usize);
        let lines = "\n".repeat(src[start..end].matches('\n').count());
        let replacement = format!("({{{}}}{lines})", entries.join(", "));
        out.replace_range(start..end, &replacement);
    }
    imports.reverse();
    let imports = imports.concat();
    out.insert_str(after_hashbang(&out), &imports);
    Some(out)
}

/// Split parsed items into successful calls and the first parse-time error (if any).
fn partition_parsed(parsed: Vec<Parsed>) -> (Vec<Call>, Option<String>) {
    let mut calls = Vec::new();
    let mut invalid = None;
    for p in parsed {
        match p {
            Parsed::Call(c) => calls.push(c),
            Parsed::Error(m) => {
                invalid.get_or_insert(m);
            }
        }
    }
    (calls, invalid)
}

/// The first pattern across every call that cannot be anchored; `None` when every one anchors.
fn first_unanchored(
    calls: &[Call],
    dir: &Path,
    root: &Path,
    aliases: &[(String, String)],
) -> Option<String> {
    calls.iter().flat_map(|c| &c.patterns).find_map(|p| {
        let p = p.strip_prefix('!').unwrap_or(p);
        anchor(p, dir, root, aliases).is_none().then(|| {
            format!("Invalid glob: \"{p}\" (resolved: \"{p}\"). It must start with '/' or './'")
        })
    })
}

/// The offset of the first line after a hashbang (0 without one).
fn after_hashbang(src: &str) -> usize {
    if src.starts_with("#!") {
        src.find('\n').map_or(src.len(), |n| n + 1)
    } else {
        0
    }
}

/// The object entries (`key: value`) of call `i`'s matches, and its eager imports.
fn call_entries(
    call: &Call,
    i: usize,
    found: &[(String, String)],
) -> Option<(Vec<String>, String)> {
    let mut entries = Vec::new();
    let mut eager = String::new();
    let pick = call
        .import
        .as_ref()
        .map(|n| format!("[{}]", serde_json::to_string(n).unwrap_or_default()))
        .unwrap_or_default();
    for (j, (key, spec)) in found.iter().enumerate() {
        let spec = serde_json::to_string(&format!("{spec}{}", call.query)).ok()?;
        let value = if call.eager {
            let local = format!("__tt_glob_{i}_{j}");
            eager.push_str(&format!("import * as {local} from {spec};"));
            format!("{local}{pick}")
        } else if pick.is_empty() {
            format!("() => import({spec})")
        } else {
            format!("() => import({spec}).then((m) => m{pick})")
        };
        entries.push(format!("{}: {value}", serde_json::to_string(key).ok()?));
    }
    Some((entries, eager))
}

/// The `import.meta.glob(...)` calls of `src` vite replaces or fails on, in source order.
fn glob_calls(file: &Path, src: &str) -> Vec<Parsed> {
    struct Finder(Vec<Parsed>);
    impl<'a> Visit<'a> for Finder {
        fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
            if is_import_meta_glob(&it.callee) {
                if let Some(call) = parse_call(it) {
                    self.0.push(call);
                }
            }
            walk::walk_call_expression(self, it);
        }
    }
    let allocator = Allocator::default();
    let mut stype = SourceType::from_path(file).unwrap_or_default();
    if !stype.is_typescript() {
        stype = stype.with_jsx(true);
    }
    let parsed = Parser::new(&allocator, src, stype).parse();
    if !parsed.errors.is_empty() {
        return Vec::new();
    }
    let mut f = Finder(Vec::new());
    f.visit_program(&parsed.program);
    f.0
}

/// `import.meta.glob`, the `import.meta` possibly parenthesized or cast (`(import.meta as any)`
/// — esbuild strips the cast before vite sees it).
fn is_import_meta_glob(callee: &Expression) -> bool {
    let Expression::StaticMemberExpression(m) = callee.without_parentheses() else {
        return false;
    };
    let mut object = &m.object;
    loop {
        object = match object {
            Expression::ParenthesizedExpression(p) => &p.expression,
            Expression::TSAsExpression(e) => &e.expression,
            Expression::TSSatisfiesExpression(e) => &e.expression,
            Expression::TSNonNullExpression(e) => &e.expression,
            Expression::TSTypeAssertion(e) => &e.expression,
            _ => break,
        };
    }
    m.property.name == "glob"
        && matches!(object, Expression::MetaProperty(p) if p.meta.name == "import" && p.property.name == "meta")
}

/// What vite's import-glob plugin makes of one call: the call it replaces, or the message it fails
/// the module with at transform (parseImportGlob's "Invalid glob import syntax" checks of the
/// arguments, parseGlobOptions' checks of the evaluated options).
#[derive(Debug, PartialEq)]
enum Parsed {
    Call(Call),
    Error(String),
}

/// A value of the options literal as vite's `evalValue` gives it (`Object` in own-key order).
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Object(Vec<(String, Value)>),
    /// a Node global (by name): its type is known, not its contents
    Global(String),
}

impl Value {
    fn type_of(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Null | Value::Object(_) => "object",
            Value::Global(name) if OBJECT_GLOBALS.contains(&name.as_str()) => "object",
            Value::Global(_) => "function",
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::Object(_) | Value::Global(_) => true,
        }
    }

    /// The value as a property key / query value (JS `String(v)`).
    fn to_js_string(&self) -> String {
        match self {
            Value::Undefined => "undefined".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => js_number(*n),
            Value::Str(s) => s.clone(),
            // (a global is never made a string: callers take it as unsupported first)
            Value::Object(_) | Value::Global(_) => "[object Object]".to_string(),
        }
    }
}

/// Why an options literal does not evaluate here: vite's evaluation throws on it (a name that is
/// neither declared there nor a Node global: "not static"), or this evaluator does not take it
/// (members, operators, a global's value — the call is left alone).
#[derive(Debug, PartialEq)]
enum Eval {
    NotStatic,
    Unsupported,
}

/// Node's global names (vitest 4.1.12 runs vite on Node), sorted: a bare reference to one in
/// the options evaluates in vite, to a value this evaluator does not model.
const NODE_GLOBALS: [&str; 135] = [
    "AbortController",
    "AbortSignal",
    "AggregateError",
    "Array",
    "ArrayBuffer",
    "AsyncDisposableStack",
    "Atomics",
    "BigInt",
    "BigInt64Array",
    "BigUint64Array",
    "Blob",
    "Boolean",
    "BroadcastChannel",
    "Buffer",
    "ByteLengthQueuingStrategy",
    "CloseEvent",
    "CompressionStream",
    "CountQueuingStrategy",
    "Crypto",
    "CryptoKey",
    "CustomEvent",
    "DOMException",
    "DataView",
    "Date",
    "DecompressionStream",
    "DisposableStack",
    "Error",
    "ErrorEvent",
    "EvalError",
    "Event",
    "EventTarget",
    "File",
    "FinalizationRegistry",
    "Float16Array",
    "Float32Array",
    "Float64Array",
    "FormData",
    "Function",
    "Headers",
    "Int16Array",
    "Int32Array",
    "Int8Array",
    "Intl",
    "Iterator",
    "JSON",
    "Map",
    "Math",
    "MessageChannel",
    "MessageEvent",
    "MessagePort",
    "Navigator",
    "Number",
    "Object",
    "Performance",
    "PerformanceEntry",
    "PerformanceMark",
    "PerformanceMeasure",
    "PerformanceObserver",
    "PerformanceObserverEntryList",
    "PerformanceResourceTiming",
    "Promise",
    "Proxy",
    "QuotaExceededError",
    "RangeError",
    "ReadableByteStreamController",
    "ReadableStream",
    "ReadableStreamBYOBReader",
    "ReadableStreamBYOBRequest",
    "ReadableStreamDefaultController",
    "ReadableStreamDefaultReader",
    "ReferenceError",
    "Reflect",
    "RegExp",
    "Request",
    "Response",
    "Set",
    "SharedArrayBuffer",
    "Storage",
    "String",
    "SubtleCrypto",
    "SuppressedError",
    "Symbol",
    "SyntaxError",
    "Temporal",
    "TextDecoder",
    "TextDecoderStream",
    "TextEncoder",
    "TextEncoderStream",
    "TransformStream",
    "TransformStreamDefaultController",
    "TypeError",
    "URIError",
    "URL",
    "URLPattern",
    "URLSearchParams",
    "Uint16Array",
    "Uint32Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "WeakMap",
    "WeakRef",
    "WeakSet",
    "WebAssembly",
    "WebSocket",
    "WritableStream",
    "WritableStreamDefaultController",
    "WritableStreamDefaultWriter",
    "atob",
    "btoa",
    "clearImmediate",
    "clearInterval",
    "clearTimeout",
    "crypto",
    "decodeURI",
    "decodeURIComponent",
    "encodeURI",
    "encodeURIComponent",
    "escape",
    "eval",
    "fetch",
    "globalThis",
    "isFinite",
    "isNaN",
    "localStorage",
    "navigator",
    "parseFloat",
    "parseInt",
    "performance",
    "queueMicrotask",
    "sessionStorage",
    "setImmediate",
    "setInterval",
    "setTimeout",
    "structuredClone",
    "unescape",
];

/// The Node globals that are objects (`typeof` "object"); the others are functions.
const OBJECT_GLOBALS: [&str; 15] = [
    "Atomics",
    "Intl",
    "JSON",
    "Math",
    "Reflect",
    "Temporal",
    "WebAssembly",
    "console",
    "crypto",
    "global",
    "globalThis",
    "navigator",
    "performance",
    "process",
    "sessionStorage",
];

/// Node's global `name` as vite's evaluation reads it off `globalThis` (`None`: no such global).
fn global_value(name: &str) -> Option<Value> {
    Some(match name {
        "undefined" | "localStorage" => Value::Undefined,
        "NaN" => Value::Num(f64::NAN),
        "Infinity" => Value::Num(f64::INFINITY),
        "console" | "global" | "process" => Value::Global(name.to_string()),
        n if NODE_GLOBALS.binary_search(&n).is_ok() => Value::Global(name.to_string()),
        _ => return None,
    })
}

/// The members of Object.prototype (a plain object's inherited names).
const OBJECT_PROTO: [&str; 12] = [
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
    "__proto__",
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
];

/// `object[key]` as JS reads it, for the values this evaluator models.
fn member(object: Value, key: &str) -> Result<Value, Eval> {
    let constant = |n: f64| Ok(Value::Num(n));
    let index = key.parse::<usize>().ok().filter(|i| i.to_string() == key);
    match (object, key) {
        // a TypeError in vite's evaluation
        (Value::Undefined | Value::Null, _) => Err(Eval::NotStatic),
        // (an array literal's `length` is not modelled)
        (Value::Object(props), k) => match props.into_iter().find(|(n, _)| n == k) {
            Some((_, v)) => Ok(v),
            None if k == "length" || OBJECT_PROTO.contains(&k) => Err(Eval::Unsupported),
            None => Ok(Value::Undefined),
        },
        (Value::Str(s), "length") => constant(s.encode_utf16().count() as f64),
        (Value::Str(s), _) if s.is_ascii() && index.is_some() => Ok(index
            .and_then(|i| s.get(i..i + 1))
            .map_or(Value::Undefined, |c| Value::Str(c.to_string()))),
        (Value::Global(g), k) => match (g.as_str(), k) {
            ("globalThis", k) => Ok(global_value(k).unwrap_or(Value::Undefined)),
            ("Math", "PI") => constant(std::f64::consts::PI),
            ("Math", "E") => constant(std::f64::consts::E),
            ("Math", "LN2") => constant(std::f64::consts::LN_2),
            ("Math", "LN10") => constant(std::f64::consts::LN_10),
            ("Math", "LOG2E") => constant(std::f64::consts::LOG2_E),
            ("Math", "LOG10E") => constant(std::f64::consts::LOG10_E),
            ("Math", "SQRT2") => constant(std::f64::consts::SQRT_2),
            ("Math", "SQRT1_2") => constant(std::f64::consts::FRAC_1_SQRT_2),
            ("Number", "MAX_SAFE_INTEGER") => constant(9007199254740991.0),
            ("Number", "MIN_SAFE_INTEGER") => constant(-9007199254740991.0),
            ("Number", "EPSILON") => constant(f64::EPSILON),
            ("Number", "MAX_VALUE") => constant(f64::MAX),
            ("Number", "MIN_VALUE") => constant(5e-324),
            ("Number", "POSITIVE_INFINITY") => constant(f64::INFINITY),
            ("Number", "NEGATIVE_INFINITY") => constant(f64::NEG_INFINITY),
            ("Number", "NaN") => constant(f64::NAN),
            _ => Err(Eval::Unsupported),
        },
        _ => Err(Eval::Unsupported),
    }
}

/// `l === r` (`None`: an object's identity, not modelled).
fn strict_equal(l: &Value, r: &Value) -> Option<bool> {
    match (l, r) {
        (Value::Object(_) | Value::Global(_), _) | (_, Value::Object(_) | Value::Global(_)) => None,
        (a, b) => Some(a == b),
    }
}

/// `l <op> r` for the operators and operand kinds this evaluator models.
fn binary(op: BinaryOperator, l: Value, r: Value) -> Result<Value, Eval> {
    use BinaryOperator as B;
    if let Some(v) = binary_arithmetic(op, &l, &r) {
        return v;
    }
    if is_relational(op) {
        return relational_result(op, l, r);
    }
    match (op, l, r) {
        (B::StrictEquality | B::StrictInequality, l, r) => {
            let eq = strict_equal(&l, &r).ok_or(Eval::Unsupported)?;
            Ok(Value::Bool(eq == (op == B::StrictEquality)))
        }
        (B::Equality | B::Inequality, l, r) => equality_result(op, l, r),
        _ => Err(Eval::Unsupported),
    }
}

/// `l <arith> r` for the arithmetic/concat operators; `None` when `op` is not one of them.
fn binary_arithmetic(op: BinaryOperator, l: &Value, r: &Value) -> Option<Result<Value, Eval>> {
    use BinaryOperator as B;
    let num = |n: f64| Ok(Value::Num(n));
    Some(match (op, l, r) {
        (B::Addition, Value::Num(a), Value::Num(b)) => num(a + b),
        (B::Addition, l @ (Value::Str(_) | Value::Num(_)), r @ (Value::Str(_) | Value::Num(_))) => {
            Ok(Value::Str(l.to_js_string() + &r.to_js_string()))
        }
        (B::Subtraction, Value::Num(a), Value::Num(b)) => num(a - b),
        (B::Multiplication, Value::Num(a), Value::Num(b)) => num(a * b),
        (B::Division, Value::Num(a), Value::Num(b)) => num(a / b),
        (B::Remainder, Value::Num(a), Value::Num(b)) => num(a % b),
        _ => return None,
    })
}

fn is_relational(op: BinaryOperator) -> bool {
    use BinaryOperator as B;
    matches!(
        op,
        B::LessThan | B::LessEqualThan | B::GreaterThan | B::GreaterEqualThan
    )
}

fn relational_result(op: BinaryOperator, l: Value, r: Value) -> Result<Value, Eval> {
    use BinaryOperator as B;
    let compare = |o: Option<std::cmp::Ordering>| {
        Ok(Value::Bool(o.is_some_and(|o| match op {
            B::LessThan => o.is_lt(),
            B::LessEqualThan => o.is_le(),
            B::GreaterThan => o.is_gt(),
            _ => o.is_ge(),
        })))
    };
    match (l, r) {
        (Value::Num(a), Value::Num(b)) => compare(a.partial_cmp(&b)),
        (Value::Str(a), Value::Str(b)) => compare(Some(a.encode_utf16().cmp(b.encode_utf16()))),
        _ => Err(Eval::Unsupported),
    }
}

fn equality_result(op: BinaryOperator, l: Value, r: Value) -> Result<Value, Eval> {
    use BinaryOperator as B;
    let nullish = |v: &Value| matches!(v, Value::Undefined | Value::Null);
    let eq = if nullish(&l) || nullish(&r) {
        nullish(&l) && nullish(&r)
    } else if std::mem::discriminant(&l) == std::mem::discriminant(&r) {
        strict_equal(&l, &r).ok_or(Eval::Unsupported)?
    } else {
        // a coercing comparison is not modelled
        return Err(Eval::Unsupported);
    };
    Ok(Value::Bool(eq == (op == B::Equality)))
}

/// JS `Number.prototype.toString()` of `n`.
fn js_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    if (1e-6..1e21).contains(&n.abs()) {
        return n.to_string();
    }
    // exponent notation, `e+` for a positive exponent
    let e = format!("{n:e}");
    match e.split_once('e') {
        Some((m, x)) if !x.starts_with('-') => format!("{m}e+{x}"),
        _ => e,
    }
}

/// An object's own keys in JS order: array indices ascending, then the rest as inserted.
fn own_keys(props: &[(String, Value)]) -> Vec<&(String, Value)> {
    let index = |k: &str| {
        k.parse::<u32>()
            .ok()
            .filter(|n| *n != u32::MAX && n.to_string() == k)
    };
    let mut indices: Vec<(u32, &(String, Value))> = props
        .iter()
        .filter_map(|p| index(&p.0).map(|n| (n, p)))
        .collect();
    indices.sort_by_key(|(n, _)| *n);
    let rest = props.iter().filter(|p| index(&p.0).is_none());
    indices.into_iter().map(|(_, p)| p).chain(rest).collect()
}

/// Define `key` on an object under construction (a later definition keeps the first position).
fn define(props: &mut Vec<(String, Value)>, key: String, value: Value) {
    match props.iter_mut().find(|(k, _)| *k == key) {
        Some(p) => p.1 = value,
        None => props.push((key, value)),
    }
}

/// `e` evaluated as vite's `evalValue` does (in source order, so the first unknown name decides).
fn eval(e: &Expression) -> Result<Value, Eval> {
    let inner = e.get_inner_expression();
    if let Some(v) = eval_literal(inner)? {
        return Ok(v);
    }
    eval_compound(inner)
}

/// Literals and identifiers: the leaf forms of `eval`. Returns `Ok(None)` when `e` is not one.
fn eval_literal(e: &Expression) -> Result<Option<Value>, Eval> {
    Ok(Some(match e {
        Expression::BooleanLiteral(b) => Value::Bool(b.value),
        Expression::NullLiteral(_) => Value::Null,
        Expression::NumericLiteral(n) => Value::Num(n.value),
        Expression::StringLiteral(s) => Value::Str(s.value.to_string()),
        Expression::TemplateLiteral(t) => eval_template(t)?,
        Expression::Identifier(id) => eval_identifier(&id.name)?,
        _ => return Ok(None),
    }))
}

/// Compound expression forms of `eval` (operators, members, collections).
fn eval_compound(e: &Expression) -> Result<Value, Eval> {
    if let Some(v) = eval_operator(e)? {
        return Ok(v);
    }
    eval_members_collections(e)
}

/// Unary/binary/logical/conditional operators; `None` when `e` is not one.
fn eval_operator(e: &Expression) -> Result<Option<Value>, Eval> {
    Ok(Some(match e {
        Expression::UnaryExpression(u) => eval_unary(u)?,
        Expression::BinaryExpression(b) => eval_binary(b)?,
        Expression::LogicalExpression(l) => eval_logical(l)?,
        Expression::ConditionalExpression(c) => eval_conditional(c)?,
        _ => return Ok(None),
    }))
}

/// Static/computed member accesses, array/object literals.
fn eval_members_collections(e: &Expression) -> Result<Value, Eval> {
    Ok(match e {
        Expression::StaticMemberExpression(m) => eval_static_member(m)?,
        Expression::ComputedMemberExpression(m) => eval_computed(m)?,
        Expression::ArrayExpression(a) => eval_array(a)?,
        Expression::ObjectExpression(o) => eval_object(o)?,
        _ => return Err(Eval::Unsupported),
    })
}

/// A binary expression: evaluate both sides in source order, then combine with `binary`.
fn eval_binary(b: &BinaryExpression) -> Result<Value, Eval> {
    let l = eval(&b.left)?;
    binary(b.operator, l, eval(&b.right)?)
}

/// A `.name` member access on a statically known object.
fn eval_static_member(m: &StaticMemberExpression) -> Result<Value, Eval> {
    member(eval(&m.object)?, &m.property.name)
}

fn eval_conditional(c: &ConditionalExpression) -> Result<Value, Eval> {
    if eval(&c.test)?.truthy() {
        eval(&c.consequent)
    } else {
        eval(&c.alternate)
    }
}

fn eval_computed(m: &ComputedMemberExpression) -> Result<Value, Eval> {
    let object = eval(&m.object)?;
    member(object, &eval_key(&m.expression)?)
}

/// A computed key evaluated to its string (a global object is no key).
fn eval_key(e: &Expression) -> Result<String, Eval> {
    match eval(e)? {
        Value::Global(_) => Err(Eval::Unsupported),
        k => Ok(k.to_js_string()),
    }
}

fn eval_template(t: &TemplateLiteral) -> Result<Value, Eval> {
    let mut out = String::new();
    for (i, q) in t.quasis.iter().enumerate() {
        out.push_str(q.value.cooked.ok_or(Eval::Unsupported)?.as_str());
        if let Some(e) = t.expressions.get(i) {
            out.push_str(&eval_key(e)?);
        }
    }
    Ok(Value::Str(out))
}

fn eval_identifier(name: &str) -> Result<Value, Eval> {
    match name {
        // evalValue declares these (`var console, exports, …`)
        "undefined" | "console" | "exports" | "global" | "module" | "process" | "require" => {
            Ok(Value::Undefined)
        }
        n => global_value(n).ok_or(Eval::NotStatic),
    }
}

fn eval_unary(u: &UnaryExpression) -> Result<Value, Eval> {
    // `typeof` of an undeclared name is "undefined"
    if u.operator == UnaryOperator::Typeof {
        return Ok(
            match (u.argument.get_inner_expression(), eval(&u.argument)) {
                (Expression::Identifier(_), Err(Eval::NotStatic)) => Value::Str("undefined".into()),
                (_, v) => Value::Str(v?.type_of().to_string()),
            },
        );
    }
    let v = eval(&u.argument)?;
    Ok(match (u.operator, v) {
        (UnaryOperator::LogicalNot, v) => Value::Bool(!v.truthy()),
        (UnaryOperator::UnaryNegation, Value::Num(n)) => Value::Num(-n),
        (UnaryOperator::UnaryPlus, Value::Num(n)) => Value::Num(n),
        (UnaryOperator::Void, _) => Value::Undefined,
        _ => return Err(Eval::Unsupported),
    })
}

fn eval_logical(l: &LogicalExpression) -> Result<Value, Eval> {
    let left = eval(&l.left)?;
    let take_left = match l.operator {
        LogicalOperator::And => !left.truthy(),
        LogicalOperator::Or => left.truthy(),
        LogicalOperator::Coalesce => !matches!(left, Value::Undefined | Value::Null),
    };
    if take_left {
        Ok(left)
    } else {
        eval(&l.right)
    }
}

fn eval_array(a: &ArrayExpression) -> Result<Value, Eval> {
    let mut props = Vec::new();
    for (i, el) in a.elements.iter().enumerate() {
        match el {
            ArrayExpressionElement::Elision(_) => {}
            ArrayExpressionElement::SpreadElement(_) => return Err(Eval::Unsupported),
            el => props.push((i.to_string(), eval(el.to_expression())?)),
        }
    }
    Ok(Value::Object(props))
}

fn eval_object(o: &ObjectExpression) -> Result<Value, Eval> {
    let mut props = Vec::new();
    for p in &o.properties {
        match p {
            ObjectPropertyKind::SpreadProperty(s) => eval_spread(&s.argument, &mut props)?,
            ObjectPropertyKind::ObjectProperty(p) => eval_plain_property(p, &mut props)?,
        }
    }
    Ok(Value::Object(props))
}

/// Define one `{ key: value }` property into `props`. Rejects accessor/method properties.
fn eval_plain_property(p: &ObjectProperty, props: &mut Vec<(String, Value)>) -> Result<(), Eval> {
    if p.kind != PropertyKind::Init || p.method {
        return Err(Eval::Unsupported);
    }
    let key = eval_property_key(p)?;
    let value = eval(&p.value)?;
    define(props, key, value);
    Ok(())
}

/// `...argument` spread into `props`.
fn eval_spread(argument: &Expression, props: &mut Vec<(String, Value)>) -> Result<(), Eval> {
    match eval(argument)? {
        Value::Object(inner) => {
            for (k, v) in own_keys(&inner) {
                define(props, k.clone(), v.clone());
            }
        }
        Value::Str(s) if s.is_ascii() => {
            for (i, c) in s.chars().enumerate() {
                define(props, i.to_string(), Value::Str(c.to_string()));
            }
        }
        Value::Str(_) => return Err(Eval::Unsupported),
        // null, undefined, booleans and numbers spread nothing
        _ => {}
    }
    Ok(())
}

fn eval_property_key(p: &ObjectProperty) -> Result<String, Eval> {
    match &p.key {
        // `__proto__: x` sets the prototype, it defines no key
        PropertyKey::StaticIdentifier(id) if id.name == "__proto__" => Err(Eval::Unsupported),
        PropertyKey::StaticIdentifier(id) => Ok(id.name.to_string()),
        PropertyKey::StringLiteral(s) if !p.computed && s.value == "__proto__" => {
            Err(Eval::Unsupported)
        }
        k => eval_key(k.as_expression().ok_or(Eval::Unsupported)?),
    }
}

/// The ESTree node type vite's message names for an argument.
fn estree_type(e: &Expression) -> &'static str {
    match e.get_inner_expression() {
        Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::StringLiteral(_) => "Literal",
        Expression::TemplateLiteral(_) => "TemplateLiteral",
        Expression::Identifier(_) => "Identifier",
        Expression::MetaProperty(_) => "MetaProperty",
        Expression::ArrayExpression(_) => "ArrayExpression",
        Expression::ArrowFunctionExpression(_) => "ArrowFunctionExpression",
        Expression::AssignmentExpression(_) => "AssignmentExpression",
        Expression::AwaitExpression(_) => "AwaitExpression",
        Expression::BinaryExpression(_) | Expression::PrivateInExpression(_) => "BinaryExpression",
        Expression::CallExpression(_) => "CallExpression",
        Expression::ChainExpression(_) => "ChainExpression",
        Expression::ClassExpression(_) => "ClassExpression",
        Expression::ConditionalExpression(_) => "ConditionalExpression",
        Expression::FunctionExpression(_) => "FunctionExpression",
        Expression::ImportExpression(_) => "ImportExpression",
        Expression::LogicalExpression(_) => "LogicalExpression",
        Expression::NewExpression(_) => "NewExpression",
        Expression::SequenceExpression(_) => "SequenceExpression",
        Expression::TaggedTemplateExpression(_) => "TaggedTemplateExpression",
        Expression::ThisExpression(_) => "ThisExpression",
        Expression::UnaryExpression(_) => "UnaryExpression",
        Expression::UpdateExpression(_) => "UpdateExpression",
        Expression::YieldExpression(_) => "YieldExpression",
        Expression::StaticMemberExpression(_)
        | Expression::ComputedMemberExpression(_)
        | Expression::PrivateFieldExpression(_) => "MemberExpression",
        Expression::JSXElement(_) => "JSXElement",
        Expression::JSXFragment(_) => "JSXFragment",
        _ => "Expression",
    }
}

/// A pattern argument as vite's `validateLiteral` takes it, else its message.
fn glob_literal(e: &Expression) -> Result<String, String> {
    let not_a_string = |t: &str| Err(format!("Expected glob to be a string, but got \"{t}\""));
    match e.get_inner_expression() {
        Expression::StringLiteral(s) => Ok(s.value.to_string()),
        Expression::TemplateLiteral(t) if !t.expressions.is_empty() => {
            Err("Expected glob to be a string, but got dynamic template literal".to_string())
        }
        Expression::TemplateLiteral(t) => {
            let q = t.quasis.first().ok_or("Could only use literals")?;
            Ok(q.value.cooked.unwrap_or(q.value.raw).to_string())
        }
        Expression::NumericLiteral(_) => not_a_string("number"),
        Expression::BooleanLiteral(_) => not_a_string("boolean"),
        Expression::BigIntLiteral(_) => not_a_string("bigint"),
        Expression::NullLiteral(_) | Expression::RegExpLiteral(_) => not_a_string("object"),
        _ => Err("Could only use literals".to_string()),
    }
}

/// vite's import-glob options (knownOptions) and the types each takes.
const KNOWN_OPTIONS: [(&str, &[&str]); 7] = [
    ("as", &["string"]),
    ("eager", &["boolean"]),
    ("import", &["string"]),
    ("exhaustive", &["boolean"]),
    ("query", &["object", "string"]),
    ("base", &["string"]),
    ("caseSensitive", &["boolean"]),
];

/// The evaluated options checked and normalized as vite's parseGlobOptions does, into `call`;
/// `Err` its message.
fn glob_options(opts: &[(String, Value)], call: &mut Call) -> Result<(), String> {
    check_option_types(opts)?;
    let get = |k: &str| opts.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    // a string option, "" when it is absent (falsy)
    let text = |k: &str| match get(k) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let base = text("base");
    check_base(&base)?;
    let query = match get("query") {
        Some(Value::Object(q)) => object_query(q)?,
        _ => text("query"),
    };
    let (import, query) = as_option(text("as"), text("import"), query)?;
    call.case_sensitive = get("caseSensitive") != Some(&Value::Bool(false));
    call.eager = get("eager") == Some(&Value::Bool(true));
    call.exhaustive = get("exhaustive") == Some(&Value::Bool(true));
    call.import = (!import.is_empty() && import != "*").then_some(import);
    call.query = query;
    call.base = (!base.is_empty()).then_some(base);
    Ok(())
}

/// Every option known and of a type it takes.
fn check_option_types(opts: &[(String, Value)]) -> Result<(), String> {
    for (key, value) in own_keys(opts) {
        let Some((_, allowed)) = KNOWN_OPTIONS.iter().find(|(k, _)| k == key) else {
            return Err(format!("Unknown glob option \"{key}\""));
        };
        let t = value.type_of();
        if !allowed.contains(&t) {
            return Err(format!(
                "Expected glob option \"{key}\" to be of type {}, but got {t}",
                allowed.join(" or ")
            ));
        }
    }
    Ok(())
}

fn check_base(base: &str) -> Result<(), String> {
    if base.starts_with('!') {
        return Err("Option \"base\" cannot start with \"!\"".to_string());
    }
    if !base.is_empty()
        && !base.starts_with('/')
        && !base.starts_with("./")
        && !base.starts_with("../")
    {
        return Err(format!(
            "Option \"base\" must start with '/', './' or '../', but got \"{base}\""
        ));
    }
    Ok(())
}

/// An object `query` as vite's stringifyQuery writes it.
fn object_query(q: &[(String, Value)]) -> Result<String, String> {
    let mut items = Vec::new();
    for (k, v) in own_keys(q) {
        let t = v.type_of();
        if !["string", "number", "boolean"].contains(&t) {
            return Err(format!(
                "Expected glob option \"query.{k}\" to be of type string, number, or boolean, but got {t}"
            ));
        }
        // vite's stringifyQuery: an empty value is the key alone
        let v = v.to_js_string();
        let item = match v.as_str() {
            "" => form_encode(k),
            _ => format!("{}={}", form_encode(k), form_encode(&v)),
        };
        if !item.is_empty() {
            items.push(item);
        }
    }
    Ok(items.join("&"))
}

/// `import` and `query` with the `as` option applied (and `query` given its `?`).
fn as_option(r#as: String, import: String, mut query: String) -> Result<(String, String), String> {
    let import = apply_as_to_import(&r#as, import)?;
    if !r#as.is_empty() {
        if !query.is_empty() {
            return Err("Options \"as\" and \"query\" cannot be used together".to_string());
        }
        query = r#as;
    }
    if !query.is_empty() && !query.starts_with('?') {
        query = format!("?{query}");
    }
    Ok((import, query))
}

/// Default `import` to `default` when `as` is `raw`/`url`, and reject non-default/`*` imports.
fn apply_as_to_import(r#as: &str, mut import: String) -> Result<String, String> {
    if r#as != "raw" && r#as != "url" {
        return Ok(import);
    }
    if !import.is_empty() && import != "default" && import != "*" {
        return Err(format!(
            "Option \"import\" can only be \"default\" or \"*\" when \"as\" is \"{as}\", but got \"{import}\"",
            as = r#as
        ));
    }
    if import.is_empty() {
        import = "default".to_string();
    }
    Ok(import)
}

/// vite (4.1.12) cuts the options at the end of their first call and fails its own parse of the
/// cut text: the closer it then expects is the one of the innermost `{…}`, `[…]` (array or
/// computed key) or `${…}` the call sits in. `None` when the options hold no call.
fn first_call_closer(opts: &Expression) -> Option<char> {
    #[derive(Default)]
    struct Finder {
        open: Vec<char>,
        found: Option<char>,
    }
    impl Finder {
        fn within(&mut self, closer: char, f: impl FnOnce(&mut Self)) {
            self.open.push(closer);
            f(self);
            self.open.pop();
        }
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_call_expression(&mut self, _: &CallExpression<'a>) {
            if self.found.is_none() {
                self.found = self.open.last().copied();
            }
        }
        fn visit_object_expression(&mut self, it: &ObjectExpression<'a>) {
            self.within('}', |v| walk::walk_object_expression(v, it));
        }
        fn visit_array_expression(&mut self, it: &ArrayExpression<'a>) {
            self.within(']', |v| walk::walk_array_expression(v, it));
        }
        fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
            self.within('}', |v| walk::walk_template_literal(v, it));
        }
        fn visit_property_key(&mut self, it: &PropertyKey<'a>) {
            self.within(']', |v| walk::walk_property_key(v, it));
        }
    }
    let mut finder = Finder::default();
    finder.visit_expression(opts);
    finder.found
}

/// One call as vite's import-glob plugin takes it (parseImportGlob): `None` when this transform
/// leaves it alone (options it does not evaluate).
fn parse_call(it: &CallExpression) -> Option<Parsed> {
    let syntax = |m: String| Some(Parsed::Error(format!("Invalid glob import syntax: {m}")));
    let n = it.arguments.len();
    if !(1..=2).contains(&n) {
        return syntax(format!("Expected 1-2 arguments, but got {n}"));
    }
    let mut call = Call {
        span: (it.span.start, it.span.end),
        patterns: Vec::new(),
        eager: false,
        import: None,
        exhaustive: false,
        query: String::new(),
        base: None,
        case_sensitive: true,
    };
    if let Err(m) = collect_glob_patterns(&it.arguments[0], &mut call) {
        return syntax(m);
    }
    if let Some(arg) = it.arguments.get(1) {
        match apply_glob_options_arg(arg, &mut call) {
            OptsOutcome::Ok => {}
            OptsOutcome::Error(e) => return Some(e),
            OptsOutcome::Skip => return None,
        }
    }
    Some(Parsed::Call(call))
}

/// Collect the first-argument glob literals (single string or array of strings) into `call`.
/// Returns the vite-shaped error message on syntax failure.
fn collect_glob_patterns(arg: &oxc_ast::ast::Argument, call: &mut Call) -> Result<(), String> {
    let literals = || "Could only use literals".to_string();
    match arg.as_expression().map(|e| e.get_inner_expression()) {
        Some(Expression::ArrayExpression(a)) => {
            for el in &a.elements {
                match el {
                    // a hole is skipped
                    ArrayExpressionElement::Elision(_) => {}
                    ArrayExpressionElement::SpreadElement(_) => return Err(literals()),
                    el => call.patterns.push(glob_literal(el.to_expression())?),
                }
            }
            Ok(())
        }
        Some(e) => {
            call.patterns.push(glob_literal(e)?);
            Ok(())
        }
        None => Err(literals()),
    }
}

/// Outcome of parsing the second argument of `import.meta.glob(...)`: the options were applied in
/// place, a propagatable error occurred, or the call should be left to vite's own transform (opts
/// that don't evaluate to an object literal).
enum OptsOutcome {
    Ok,
    Error(Parsed),
    Skip,
}

/// Parse the second argument of `import.meta.glob(...)` and fold its known options into `call`.
/// Returns `OptsOutcome::Error(Parsed::Error)` on a syntax / static-eval failure so `parse_call`
/// can propagate it, `OptsOutcome::Ok` on success (call mutated in place), and `OptsOutcome::Skip`
/// when the options evaluate to something other than an object literal — vite silently leaves
/// such calls to its own transform.
fn apply_glob_options_arg(arg: &oxc_ast::ast::Argument, call: &mut Call) -> OptsOutcome {
    let opts = match arg.as_expression() {
        Some(e) => match e.get_inner_expression() {
            Expression::ObjectExpression(_) => e,
            e => {
                return OptsOutcome::Error(Parsed::Error(format!(
                    "Invalid glob import syntax: Expected the second argument to be an object literal, but got \"{}\"",
                    estree_type(e)
                )))
            }
        },
        None => {
            return OptsOutcome::Error(Parsed::Error(
                "Invalid glob import syntax: Expected the second argument to be an object literal, but got \"SpreadElement\""
                    .to_string(),
            ))
        }
    };
    if let Some(closer) = first_call_closer(opts) {
        return OptsOutcome::Error(Parsed::Error(format!(
            "Parse failed with 1 error:\nExpected `{closer}` but found `EOF`"
        )));
    }
    let opts = match eval(opts) {
        Ok(Value::Object(o)) => o,
        Err(Eval::NotStatic) => {
            return OptsOutcome::Error(Parsed::Error(
                "Vite is unable to parse the glob options as the value is not static".to_string(),
            ))
        }
        _ => return OptsOutcome::Skip,
    };
    if let Err(m) = glob_options(&opts, call) {
        return OptsOutcome::Error(Parsed::Error(m));
    }
    OptsOutcome::Ok
}

/// The call's matches as (key, import specifier from the module), sorted by key; `None` when a
/// pattern is neither relative, root-absolute nor an alias (vite: an invalid glob). Keys are as
/// vite writes them: relative to `base` when there is one, else relative to the module for a
/// relative pattern, else root-relative (`/…`).
fn matches(
    call: &Call,
    file: &Path,
    dir: &Path,
    root: &Path,
    aliases: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    // the base directory: the whole path (its last segment is no glob)
    let base = match &call.base {
        Some(b) => {
            let (d, rest, _) = anchor(b, dir, root, aliases)?;
            Some(normal(&d.join(rest)))
        }
        None => None,
    };
    let from = base.as_deref().unwrap_or(dir);
    let mut pos = Vec::new();
    let mut neg = Vec::new();
    for p in &call.patterns {
        let (p, list) = match p.strip_prefix('!') {
            Some(p) => (p, &mut neg),
            None => (p.as_str(), &mut pos),
        };
        list.push(anchor(p, from, root, aliases)?);
    }
    let me = crate::canon::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    // `caseSensitive: false`: tinyglobby's caseSensitiveMatch off (the keys keep the files' case)
    let glob = |pattern: &str, path: &str| match call.case_sensitive {
        true => crate::coverage::glob_matches(pattern, path),
        false => crate::coverage::glob_matches(&pattern.to_lowercase(), &path.to_lowercase()),
    };
    let mut found: Vec<(String, String)> = Vec::new();
    for (start, rest, relative) in &pos {
        collect_pattern_matches(
            call,
            start,
            rest,
            *relative,
            &neg,
            &me,
            base.as_deref(),
            dir,
            root,
            &glob,
            &mut found,
        );
    }
    found.sort();
    Some(found)
}

/// Walk one positive pattern's directory and append its matches to `found`: skipping dotfile
/// entries unless the pattern asks for them, honouring the negative patterns, dropping the module
/// doing the glob, and deduplicating keys (first match wins, mirroring vite).
#[allow(clippy::too_many_arguments)]
fn collect_pattern_matches(
    call: &Call,
    start: &Path,
    rest: &str,
    relative: bool,
    neg: &[(PathBuf, String, bool)],
    me: &Path,
    base: Option<&Path>,
    dir: &Path,
    root: &Path,
    glob: &dyn Fn(&str, &str) -> bool,
    found: &mut Vec<(String, String)>,
) {
    let dots = call.exhaustive || rest.starts_with('.') || rest.contains("/.");
    let mut files = Vec::new();
    walk_dir(start, "", dots, call.exhaustive, &mut files);
    for (rel, abs) in files {
        if skip_candidate(&rel, &abs, rest, neg, me, glob) {
            continue;
        }
        let key = match_key(base, relative, dir, root, &abs);
        if found.iter().any(|(k, _)| *k == key) {
            continue;
        }
        found.push((key, relative_spec(dir, &abs)));
    }
}

/// Whether a walked candidate (`rel`, `abs`) fails the positive, negative, or self-exclude tests.
fn skip_candidate(
    rel: &str,
    abs: &Path,
    rest: &str,
    neg: &[(PathBuf, String, bool)],
    me: &Path,
    glob: &dyn Fn(&str, &str) -> bool,
) -> bool {
    if !glob(rest, rel) {
        return true;
    }
    let excluded = neg.iter().any(|(nd, nrest, _)| {
        abs.strip_prefix(nd)
            .is_ok_and(|r| glob(nrest, &crate::coverage::slash_path(r)))
    });
    excluded || crate::canon::canonicalize(abs).unwrap_or_else(|_| abs.to_path_buf()) == me
}

/// The key vite writes for match `abs`: relative to `base` when there is one, else to the
/// module's `dir` for a relative pattern, else root-relative (`/…`).
fn match_key(base: Option<&Path>, relative: bool, dir: &Path, root: &Path, abs: &Path) -> String {
    match base {
        Some(b) => relative_spec(b, abs),
        None if relative => relative_spec(dir, abs),
        None => match normal(abs).strip_prefix(normal(root)) {
            Ok(r) => format!("/{}", crate::coverage::slash_path(r)),
            Err(_) => crate::coverage::slash_path(abs),
        },
    }
}

/// A pattern as (the directory its fixed leading segments name, the glob below it, whether it
/// is relative to `from`): `./` / `../` from `from`, `/` from `root`, an alias from its
/// replacement; `None` for anything else.
fn anchor(
    pattern: &str,
    from: &Path,
    root: &Path,
    aliases: &[(String, String)],
) -> Option<(PathBuf, String, bool)> {
    let alias = aliases.iter().find_map(|(find, to)| {
        let rest = pattern.strip_prefix(find.as_str())?;
        (rest.is_empty() || rest.starts_with('/')).then(|| {
            let to = Path::new(to);
            let to = if to.is_absolute() {
                to.to_path_buf()
            } else {
                root.join(to)
            };
            // walk from the target's filesystem root (`/`, or `C:\\` on Windows); the rest of
            // the target joins the glob as `/`-separated segments
            let top = to.ancestors().last().unwrap_or(&to).to_path_buf();
            let below = crate::coverage::slash_path(to.strip_prefix(&top).unwrap_or(&to));
            (top, format!("{below}{rest}"))
        })
    });
    let (start, rest, relative) = match alias {
        Some((top, rel)) => (top, rel.trim_start_matches('/').to_string(), false),
        None if pattern.starts_with("./") || pattern.starts_with("../") => {
            (from.to_path_buf(), pattern.to_string(), true)
        }
        None if pattern.starts_with('/') => (
            root.to_path_buf(),
            pattern.trim_start_matches('/').to_string(),
            false,
        ),
        None => return None,
    };
    let segs: Vec<&str> = rest.split('/').collect();
    let fixed = segs[..segs.len() - 1]
        .iter()
        .take_while(|s| !s.contains(['*', '?', '[', ']', '{', '}']))
        .count();
    let dir = normal(&start.join(segs[..fixed].join("/")));
    Some((dir, segs[fixed..].join("/"), relative))
}

/// `p` with `.` and `..` folded away (lexically).
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// `application/x-www-form-urlencoded` (URLSearchParams) encoding of a query key or value.
fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Every file under `dir` as (path relative to the walk's start, absolute path); dot entries
/// only with `dots`, `node_modules` only when `exhaustive`.
fn walk_dir(dir: &Path, rel: &str, dots: bool, exhaustive: bool, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if walk_dir_skip(&name, dots, exhaustive) {
            continue;
        }
        let path = e.path();
        let r = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if path.is_dir() {
            walk_dir(&path, &r, dots, exhaustive, out);
        } else {
            out.push((r, path));
        }
    }
}

fn walk_dir_skip(name: &str, dots: bool, exhaustive: bool) -> bool {
    (!dots && name.starts_with('.')) || (!exhaustive && name == "node_modules")
}

/// `target` as an import specifier from `dir` (`./x`, `../x`).
fn relative_spec(dir: &Path, target: &Path) -> String {
    let norm = |p: &Path| -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for c in p.components() {
            match c {
                Component::Normal(s) => v.push(s.to_string_lossy().into_owned()),
                Component::ParentDir => {
                    v.pop();
                }
                _ => {}
            }
        }
        v
    };
    let (from, to) = (norm(dir), norm(target));
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    if parts.is_empty() {
        parts.push(".".to_string());
    }
    parts.extend(to[common..].iter().cloned());
    parts.join("/")
}

#[cfg(test)]
#[path = "import_glob_tests.rs"]
mod tests;
