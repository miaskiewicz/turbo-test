//! vite's asset imports as vitest 4.1.12 serves them (turbo-test#148): `?raw` is the file's text,
//! `?url` and a file of a known asset type its root-relative URL (`/@fs/<path>` outside the
//! root) with the rest of the query, and a small `.svg` (under vite's 4096-byte inline limit,
//! no `#` in the id, no `?no-inline`) vite's `data:` URL. The module's `default` is that string.

use std::path::Path;

/// vite's KNOWN_ASSET_TYPES (`jpe?g`, `woff2?` spelled out).
const KNOWN: &[&str] = &[
    "apng",
    "bmp",
    "png",
    "jpg",
    "jpeg",
    "jfif",
    "pjpeg",
    "pjp",
    "gif",
    "svg",
    "ico",
    "webp",
    "avif",
    "cur",
    "jxl",
    "mp4",
    "webm",
    "ogg",
    "mp3",
    "wav",
    "flac",
    "aac",
    "opus",
    "mov",
    "m4a",
    "vtt",
    "woff",
    "woff2",
    "eot",
    "ttf",
    "otf",
    "webmanifest",
    "pdf",
    "txt",
];

/// vite's `assetsInlineLimit` default.
const INLINE_LIMIT: usize = 4096;

fn split(id: &str) -> (&str, Option<&str>) {
    match id.split_once('?') {
        Some((f, q)) => (f, Some(q)),
        None => (id, None),
    }
}

fn has_flag(query: Option<&str>, flag: &str) -> bool {
    query.is_some_and(|q| q.split('&').any(|t| t == flag))
}

fn known(file: &str) -> bool {
    Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| KNOWN.contains(&e.to_ascii_lowercase().as_str()))
}

/// Whether the module id `id` (a path, maybe with `?query`) is one of vite's asset imports.
pub fn is_asset_id(id: &Path) -> bool {
    let id = id.to_string_lossy();
    let (file, query) = split(&id);
    has_flag(query, "raw") || has_flag(query, "url") || known(file)
}

/// The `default` string of the asset import `id`, `root` the vitest root; `None` when `id` is
/// not one (or the file cannot be read).
pub fn value(id: &Path, root: &Path) -> Option<String> {
    let id = id.to_string_lossy();
    let (file, query) = split(&id);
    if has_flag(query, "raw") {
        return Some(String::from_utf8_lossy(&std::fs::read(file).ok()?).into_owned());
    }
    let url_flag = has_flag(query, "url");
    if !url_flag && !known(file) {
        return None;
    }
    if file.ends_with(".svg") && !has_flag(query, "no-inline") && !id.contains('#') {
        let content = std::fs::read(file).ok()?;
        if content.len() < INLINE_LIMIT
            && !content.starts_with(b"version https://git-lfs.github.com")
        {
            return Some(svg_data_url(&content));
        }
    }
    // the `url` flag itself is dropped from the URL, any other query kept
    let rest: Vec<&str> = query
        .map(|q| q.split('&').filter(|t| *t != "url").collect())
        .unwrap_or_default();
    let rest = if rest.is_empty() {
        String::new()
    } else {
        format!("?{}", rest.join("&"))
    };
    let path = match Path::new(file).strip_prefix(root) {
        Ok(rel) => format!("/{}", rel.to_string_lossy()),
        Err(_) => format!("/@fs{file}"),
    };
    Some(format!("{path}{rest}"))
}

/// vite's svgToDataURL.
fn svg_data_url(content: &[u8]) -> String {
    use base64::Engine;
    let s = String::from_utf8_lossy(content);
    if s.contains("<text") || s.contains("<foreignObject") || nested_quotes(&s) {
        let b64 = base64::engine::general_purpose::STANDARD.encode(content);
        return format!("data:image/svg+xml;base64,{b64}");
    }
    // `>\s+<` -> `><`
    let trimmed = s.trim();
    let mut joined = String::new();
    let mut chars = trimmed.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        joined.push(c);
        if c == '>' {
            let rest = &trimmed[i + 1..];
            let ws = rest.len() - rest.trim_start().len();
            if ws > 0 && rest[ws..].starts_with('<') {
                while chars.peek().is_some_and(|(j, _)| *j < i + 1 + ws) {
                    chars.next();
                }
            }
        }
    }
    let mut out = String::from("data:image/svg+xml,");
    let mut in_ws = false;
    for c in joined.chars() {
        if c.is_whitespace() {
            if !in_ws {
                out.push_str("%20");
            }
            in_ws = true;
            continue;
        }
        in_ws = false;
        match c {
            '"' => out.push('\''),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '<' => out.push_str("%3c"),
            '>' => out.push_str("%3e"),
            c => out.push(c),
        }
    }
    out
}

/// vite's nestedQuotesRE: `"[^"']*'[^"]*"|'[^'"]*"[^']*'`.
fn nested_quotes(s: &str) -> bool {
    let b = s.as_bytes();
    let pair = |open: u8, inner: u8| {
        (0..b.len()).any(|i| {
            if b[i] != open {
                return false;
            }
            // open, then no open/inner, then inner, then no open, then open
            let mut j = i + 1;
            while j < b.len() && b[j] != open && b[j] != inner {
                j += 1;
            }
            if j >= b.len() || b[j] != inner {
                return false;
            }
            b[j + 1..].contains(&open)
        })
    };
    pair(b'"', b'\'') || pair(b'\'', b'"')
}

#[cfg(test)]
#[path = "vite_asset_tests.rs"]
mod tests;
