//! Test locations for vitest's `includeTaskLocation` (`--includeTaskLocation` /
//! `test.includeTaskLocation`, turbo-test#63): the `{ line, column }` of the call that registered
//! a test, as vitest reports it — V8's position of that call (1-based line, 1-based UTF-16 column:
//! a plain call's callee, a member call's property, a call-of-call's `(`) in the ORIGINAL source.
//!
//! The runtime asks for the call site (`__ttCallSite`, runner.rs) while a test is registered: the
//! innermost stack frame in the test file. Its generated position maps back to the original source
//! through the source map of the file's transform (registered by the loader with [`set_map`]) and
//! is then snapped to the original call it belongs to (the first V8 call position at or after the
//! mapped point: a map can be coarser than V8's positions). A file loaded untransformed has no map:
//! its positions are the original ones ([`set_identity`]). A file transformed without a map (the
//! esbuild fallbacks) has no locations.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{CallExpression, Expression};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

/// Per generated line, its source-map segments `(gen_col, src_line, src_col)` (0-based).
pub type Segments = Vec<Vec<(u32, u32, u32)>>;

/// Whether test locations are on for this run (`TURBO_TASK_LOCATION`, set by the CLI).
pub fn enabled() -> bool {
    std::env::var_os("TURBO_TASK_LOCATION").is_some()
}

/// The test file being run on this thread and how its generated positions map back.
struct Entry {
    path: PathBuf,
    /// Lines the loader put before the transformed code (the CJS wrapper's first line).
    line_offset: usize,
    /// The transform's source map.
    map: Option<Segments>,
    /// The loaded code is the original source (no map needed).
    identity: bool,
    /// V8 call positions `(line, column)` (1-based, UTF-16 columns) in the original source, sorted.
    calls: Vec<(u32, u32)>,
    /// The file writes inline snapshots, which need call sites too (turbo-test#73).
    inline: bool,
}

thread_local! {
    static ENTRY: RefCell<Option<Entry>> = const { RefCell::new(None) };
    /// The other transformed modules' maps, `(map, lines before the code)`, while a reporter
    /// prints failure frames ([`frames_enabled`]): an error's frames in a helper module map back
    /// too (turbo-test#81). Per worker thread; a module's map does not depend on the test file.
    static MODULES: RefCell<std::collections::HashMap<PathBuf, (Segments, usize)>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Whether failure frames are printed (`--reporter junit` sets `TURBO_ERROR_FRAMES`): the
/// loaders then keep a source map for every app module they transform.
pub fn frames_enabled() -> bool {
    std::env::var_os("TURBO_ERROR_FRAMES").is_some()
}

/// `path`'s transformed code maps back through `map`; `offset` lines precede the code.
pub fn set_module_map(path: &Path, map: Segments, offset: usize) {
    MODULES.with(|m| m.borrow_mut().insert(path.to_path_buf(), (map, offset)));
}

/// A new test file starts on this thread (no map, no wrapper offset yet). `src` is its source: one
/// that uses inline snapshots needs call sites (to write them back) even without test locations.
pub fn begin(path: &Path, src: &str) {
    ENTRY.with(|e| {
        *e.borrow_mut() = Some(Entry {
            path: path.to_path_buf(),
            line_offset: 0,
            map: None,
            identity: false,
            calls: Vec::new(),
            inline: src.contains("InlineSnapshot("),
        })
    });
}

/// Whether `path` is the test file being run and its call sites are needed (test locations on,
/// or the file uses inline snapshots).
pub fn is_entry(path: &Path) -> bool {
    ENTRY.with(|e| {
        e.borrow()
            .as_ref()
            .is_some_and(|e| e.path == path && (e.inline || enabled()))
    })
}

/// The loader compiled the test file with `offset` lines before its transformed code.
pub fn set_offset(path: &Path, offset: usize) {
    with_entry(path, |e| e.line_offset = offset);
}

/// The test file is loaded as written: its generated positions are the original ones.
pub fn set_identity(path: &Path) {
    with_entry(path, |e| e.identity = true);
}

/// The test file's transformed code maps back to `original` through `map`.
pub fn set_map(path: &Path, map: Segments, original: &str) {
    let calls = call_positions(path, original);
    with_entry(path, |e| {
        e.map = Some(map);
        e.calls = calls;
    });
}

fn with_entry(path: &Path, f: impl FnOnce(&mut Entry)) {
    ENTRY.with(|e| {
        if let Some(e) = e.borrow_mut().as_mut().filter(|e| e.path == path) {
            f(e);
        }
    });
}

/// The original `(line, column)` of an error's stack frame at generated `(line, column)`
/// (1-based) of the script named `script` — the mapped point itself, not snapped to a call (a
/// `throw new Error` is not one); `None` for a frame outside the test file (turbo-test#81).
pub fn resolve_frame(script: &str, line: usize, column: usize) -> Option<(u32, u32)> {
    let mapped = |map: &Segments, offset: usize| {
        let line = line.checked_sub(offset).filter(|l| *l > 0)?;
        let (l, c, _) = lookup(map, line - 1, column.saturating_sub(1) as u32)?;
        Some((l + 1, c + 1))
    };
    let entry = ENTRY.with(|e| {
        let e = e.borrow();
        let e = e.as_ref().filter(|e| e.path.as_os_str() == script)?;
        Some(match &e.map {
            Some(map) => mapped(map, e.line_offset),
            None => e
                .identity
                .then_some((line.checked_sub(e.line_offset)? as u32, column as u32)),
        })
    });
    match entry {
        Some(found) => found,
        None => MODULES.with(|m| {
            let m = m.borrow();
            let (map, offset) = m.get(Path::new(script))?;
            mapped(map, *offset)
        }),
    }
}

/// The original `(line, column)` of a stack frame at generated `(line, column)` (1-based) of the
/// script named `script`; `None` for a frame outside the test file.
pub fn resolve(script: &str, line: usize, column: usize) -> Option<(u32, u32)> {
    ENTRY.with(|e| {
        let e = e.borrow();
        let e = e.as_ref().filter(|e| e.path.as_os_str() == script)?;
        let line = line.checked_sub(e.line_offset).filter(|l| *l > 0)?;
        let Some(map) = &e.map else {
            return e.identity.then_some((line as u32, column as u32));
        };
        let (l, c, before) = lookup(map, line - 1, column.saturating_sub(1) as u32)?;
        let mapped = (l + 1, c + 1);
        // snap to the call the mapped point belongs to: the first call at or after it; a
        // position before the line's first segment (a statement-leading callee the emitter maps
        // only from its arguments) belongs to the last call before that segment
        let at = e.calls.partition_point(|p| *p < mapped);
        let snapped = if before {
            at.checked_sub(1).map(|i| e.calls[i])
        } else {
            e.calls.get(at).copied()
        };
        Some(snapped.unwrap_or(mapped))
    })
}

/// The original position (0-based) of generated `(line, col)`: the last segment of that line at
/// or before `col`, else the line's first segment (flagged `true`: `col` precedes it).
fn lookup(map: &Segments, line: usize, col: u32) -> Option<(u32, u32, bool)> {
    let segs = map.get(line)?;
    match segs.iter().rev().find(|s| s.0 <= col) {
        Some(seg) => Some((seg.1, seg.2, false)),
        None => segs.first().map(|seg| (seg.1, seg.2, true)),
    }
}

/// V8's position of every call expression in `src`: a plain call's callee start, a member call's
/// property start, any other call's `(`. 1-based lines and UTF-16 columns, sorted.
pub fn call_positions(path: &Path, src: &str) -> Vec<(u32, u32)> {
    let alloc = Allocator::default();
    let stype = SourceType::from_path(path).unwrap_or_default();
    let parsed = Parser::new(&alloc, src, stype).parse();
    let mut v = Calls {
        src,
        offsets: Vec::new(),
    };
    v.visit_program(&parsed.program);
    let starts: Vec<usize> = std::iter::once(0)
        .chain(src.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let mut out: Vec<(u32, u32)> = v
        .offsets
        .iter()
        .map(|&off| {
            let line = starts.partition_point(|s| *s <= off);
            let col = src[starts[line - 1]..off].encode_utf16().count() + 1;
            (line as u32, col as u32)
        })
        .collect();
    out.sort_unstable();
    out
}

struct Calls<'s> {
    src: &'s str,
    offsets: Vec<usize>,
}

impl<'a> Visit<'a> for Calls<'_> {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let off = match &it.callee {
            Expression::Identifier(id) => id.span.start as usize,
            Expression::StaticMemberExpression(m) => m.property.span.start as usize,
            callee => {
                let from = it
                    .type_arguments
                    .as_ref()
                    .map_or(callee.span().end, |t| t.span.end) as usize;
                from + self.src[from..].find('(').unwrap_or(0)
            }
        };
        self.offsets.push(off);
        walk::walk_call_expression(self, it);
    }
}

#[cfg(test)]
#[path = "location_tests.rs"]
mod tests;
