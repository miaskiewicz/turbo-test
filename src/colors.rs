//! tinyrainbow 3.2.0's colours, as vitest 4.1.12 paints its reporters with them (turbo-test#94).
//!
//! A colour wraps a string in its open and close codes; a close code of the same kind inside the
//! string (after the first `open.len()` characters) is replaced by `replace` (the open code, or
//! for bold / dim `\x1b[22m` and the open code), so nested colours resume the outer one.
//! [`supported`] is tinyrainbow's `isSupported` over an environment; vitest also turns colours
//! off under an AI agent.

/// Whether colours are on, and the colour functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Colors {
    pub on: bool,
}

/// The byte offset of the `n`th UTF-16 code unit of `s` (`s.len()` past the end).
fn utf16_offset(s: &str, n: usize) -> usize {
    let mut units = 0;
    for (i, ch) in s.char_indices() {
        if units >= n {
            return i;
        }
        units += ch.len_utf16();
    }
    s.len()
}

/// tinyrainbow's formatter: `open + s + close`, every `close` in `s` from the `open.len()`th
/// UTF-16 unit on replaced by `replace`.
fn paint(s: &str, open: &str, close: &str, replace: &str) -> String {
    let from = utf16_offset(s, open.len());
    let Some(first) = s[from..].find(close).map(|i| i + from) else {
        return format!("{open}{s}{close}");
    };
    let mut out = String::from(open);
    let mut at = first;
    let mut rest = 0;
    loop {
        out.push_str(&s[rest..at]);
        out.push_str(replace);
        rest = at + close.len();
        match s[rest..].find(close) {
            Some(i) => at = rest + i,
            None => break,
        }
    }
    out.push_str(&s[rest..]);
    out.push_str(close);
    out
}

macro_rules! colors {
    ($($name:ident = $open:literal, $close:literal $(, $replace:literal)?;)*) => {
        impl Colors {
            $(
                pub fn $name(&self, s: &str) -> String {
                    if !self.on {
                        return s.to_string();
                    }
                    #[allow(unused_variables)]
                    let replace = $open;
                    $(let replace = $replace;)?
                    paint(s, $open, $close, replace)
                }
            )*
        }
    };
}

colors! {
    bold = "\x1b[1m", "\x1b[22m", "\x1b[22m\x1b[1m";
    dim = "\x1b[2m", "\x1b[22m", "\x1b[22m\x1b[2m";
    inverse = "\x1b[7m", "\x1b[27m";
    black = "\x1b[30m", "\x1b[39m";
    red = "\x1b[31m", "\x1b[39m";
    green = "\x1b[32m", "\x1b[39m";
    yellow = "\x1b[33m", "\x1b[39m";
    blue = "\x1b[34m", "\x1b[39m";
    magenta = "\x1b[35m", "\x1b[39m";
    cyan = "\x1b[36m", "\x1b[39m";
    white = "\x1b[37m", "\x1b[39m";
    gray = "\x1b[90m", "\x1b[39m";
    bg_red = "\x1b[41m", "\x1b[49m";
    bg_cyan = "\x1b[46m", "\x1b[49m";
}

/// tinyrainbow's `isSupported`: no `NO_COLOR` and no `--no-color`, and `FORCE_COLOR`, `--color`,
/// a `TERM` other than `dumb` (unless `FORCE_TTY=false`) or `CI`.
pub fn supported(var: impl Fn(&str) -> Option<String>, argv: &[String]) -> bool {
    let has = |a: &str| argv.iter().any(|x| x == a);
    if var("NO_COLOR").is_some() || has("--no-color") {
        return false;
    }
    let tty = var("FORCE_TTY").as_deref() != Some("false");
    var("FORCE_COLOR").is_some()
        || has("--color")
        || (tty && var("TERM").as_deref() != Some("dumb"))
        || var("CI").is_some()
}

/// `s` without its ANSI colour codes (node:util `stripVTControlCharacters` for SGR sequences).
pub fn strip(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
#[path = "colors_tests.rs"]
mod tests;
