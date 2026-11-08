//! vitest 4.1.12's code-frame syntax highlighting (turbo-test#94): js-tokens 9's tokenizer and
//! tinyhighlight's `highlightTokens` with vitest's colours (`getDefs`), as `Logger.highlight`
//! paints a test file before `generateCodeFrame` cuts its frame out.
//!
//! The tokenizer is js-tokens' sticky regular expressions written out as scanners: each tries
//! its alternatives in the regular expression's order at the current position (no longest
//! match), and the mode stack (templates, JSX) and the "does a regex or an expression come next"
//! state follow js-tokens exactly. tinyhighlight keeps an identifier until the next significant
//! token tells whether it is called (`IdentifierCallable`); one left at the end of the source is
//! dropped, as there.

use crate::colors::Colors;
use std::path::Path;

/// A js-tokens token type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    StringLiteral,
    NoSubstitutionTemplate,
    TemplateHead,
    TemplateMiddle,
    TemplateTail,
    RegularExpressionLiteral,
    MultiLineComment,
    SingleLineComment,
    IdentifierName,
    PrivateIdentifier,
    NumericLiteral,
    Punctuator,
    WhiteSpace,
    LineTerminatorSequence,
    Invalid,
    JSXString,
    JSXText,
    JSXIdentifier,
    JSXPunctuator,
    JSXInvalid,
}

/// A token: its type and text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: Kind,
    pub value: &'a str,
}

/// js-tokens' mode stack tags (`JS`, `JSNonExpressionParen`, …, `JSXTag`, `JSXTagEnd`,
/// `JSXChildren`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tag {
    Js,
    JsNonExpressionParen,
    InterpolationInTemplate,
    InterpolationInJsx,
    JsxOpen,
    JsxClose,
    JsxChildren,
}

#[derive(Clone, Copy)]
struct Mode {
    tag: Tag,
    nesting: usize,
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_id_start(c: char) -> bool {
    c == '$' || c == '_' || unicode_id_start::is_id_start(c)
}

fn is_id_continue(c: char) -> bool {
    c == '$'
        || c == '_'
        || c == '\u{200c}'
        || c == '\u{200d}'
        || unicode_id_start::is_id_continue(c)
}

/// `\p{Zs}`, tab, vertical tab, form feed and the BOM.
fn is_white_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\u{b}' | '\u{c}' | '\u{feff}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

/// The character at byte `i` of `s` (None past the end).
fn at(s: &str, i: usize) -> Option<char> {
    s.get(i..).and_then(|r| r.chars().next())
}

/// After `i`, the byte index past one more character.
fn next(s: &str, i: usize) -> usize {
    i + at(s, i).map_or(0, char::len_utf8)
}

/// The byte index past the `]` that closes a regex character class starting at `i` (points at
/// `[`); falls back to `i + 1` when no closing `]` is found on the line (matches the inline loop).
fn regex_class_end(s: &str, i: usize, plain: impl Fn(char) -> bool) -> usize {
    let mut j = i + 1;
    let closed = loop {
        match at(s, j) {
            Some(']') => break Some(j + 1),
            Some('\\') if at(s, j + 1).is_some_and(&plain) => j = next(s, j + 1),
            Some(c) if c != '\\' && plain(c) => j = next(s, j),
            _ => break None,
        }
    };
    closed.unwrap_or(i + 1)
}

/// `/(?![*\/])(?:\[(?:(?![\]\\]).|\\.)*\]|(?![\/\\]).|\\.)*(\/[$_‌‍\p{ID_Continue}]*|\\)?/yu`
/// at a `/`: the end of the literal.
fn regex_literal(s: &str, start: usize) -> usize {
    let mut i = start + 1;
    if matches!(at(s, i), Some('*' | '/')) {
        // the lookahead fails: no match at all; js-tokens then tries a punctuator
        return start;
    }
    let plain = |c: char| !is_line_terminator(c);
    loop {
        match at(s, i) {
            Some('[') => i = regex_class_end(s, i, plain),
            Some('\\') if at(s, i + 1).is_some_and(plain) => i = next(s, i + 1),
            Some(c) if c != '/' && c != '\\' && plain(c) => i = next(s, i),
            _ => break,
        }
    }
    regex_literal_tail(s, i)
}

/// After the body: consume the closing `/` and its flags, a trailing `\`, or stop.
fn regex_literal_tail(s: &str, mut i: usize) -> usize {
    match at(s, i) {
        Some('/') => {
            i += 1;
            while let Some(c) = at(s, i).filter(|c| is_id_continue(*c)) {
                i += c.len_utf8();
            }
            i
        }
        Some('\\') => i + 1,
        _ => i,
    }
}

/// js-tokens' `Punctuator`, its alternatives in order: the end, if one matches at `i`.
fn punctuator(s: &str, i: usize) -> Option<usize> {
    let r = &s[i..];
    if let Some(end) = punctuator_fixed(r, i) {
        return Some(end);
    }
    if let Some(end) = punctuator_dot(s, r, i) {
        return Some(end);
    }
    // (?:&&|\|\||\?\?|[+\-%&|^]|\*{1,2}|<{1,2}|>{1,3}|!=?|={1,2}|\/(?![\/*]))=?
    if let Some(n) = operator_len(s, i) {
        let end = i + n;
        return Some(if s[end..].starts_with('=') {
            end + 1
        } else {
            end
        });
    }
    match r.chars().next() {
        Some('?' | '~' | ',' | ':' | ';' | '[' | ']' | '(' | ')' | '{' | '}') => Some(i + 1),
        _ => None,
    }
}

/// Fixed-length punctuators: `--`, `++`, `=>`, `...`.
fn punctuator_fixed(r: &str, i: usize) -> Option<usize> {
    for p in ["--", "++", "=>", "..."] {
        if r.starts_with(p) {
            return Some(i + p.len());
        }
    }
    None
}

/// `?.` / `.` when not followed by a digit (`\??\.(?!\d)`).
fn punctuator_dot(s: &str, r: &str, i: usize) -> Option<usize> {
    let not_digit = |j: usize| !at(s, j).is_some_and(|c| c.is_ascii_digit());
    if r.starts_with("?.") && not_digit(i + 2) {
        return Some(i + 2);
    }
    if r.starts_with('.') && not_digit(i + 1) {
        return Some(i + 1);
    }
    None
}

/// The length of the operator (before its optional `=`) at `i`.
fn operator_len(s: &str, i: usize) -> Option<usize> {
    let r = &s[i..];
    if r.starts_with("&&") || r.starts_with("||") || r.starts_with("??") {
        Some(2)
    } else {
        match r.chars().next() {
            Some('+' | '-' | '%' | '&' | '|' | '^') => Some(1),
            Some(c @ ('*' | '<' | '>' | '=')) => {
                let max = match c {
                    '>' => 3,
                    _ => 2,
                };
                Some(r.chars().take(max).take_while(|x| *x == c).count())
            }
            Some('!') => Some(if r[1..].starts_with('=') { 2 } else { 1 }),
            Some('/') if !matches!(at(s, i + 1), Some('/' | '*')) => Some(1),
            _ => None,
        }
    }
}

/// `\\u[\da-fA-F]{4}|\\u\{[\da-fA-F]+\}` at `i`: its end.
fn unicode_escape(s: &str, i: usize) -> Option<usize> {
    let r = s.get(i..)?;
    let body = r.strip_prefix("\\u")?;
    if let Some(end) = unicode_escape_fixed(body) {
        return Some(i + 2 + end);
    }
    unicode_escape_braced(body).map(|end| i + 2 + end)
}

/// `[\da-fA-F]{4}`: 4 if the next four characters are all hex.
fn unicode_escape_fixed(body: &str) -> Option<usize> {
    let hex = |c: char| c.is_ascii_hexdigit();
    (body.len() >= 4 && body[..4].chars().all(hex)).then_some(4)
}

/// `\{[\da-fA-F]+\}`: the length including both braces.
fn unicode_escape_braced(body: &str) -> Option<usize> {
    let hex = |c: char| c.is_ascii_hexdigit();
    let inner = body.strip_prefix('{')?;
    let n = inner.chars().take_while(|c| hex(*c)).count();
    (n > 0 && inner[n..].starts_with('}')).then_some(1 + n + 1)
}

/// `(\x23?)(?=[$_\p{ID_Start}\\])(?:[$_‌‍\p{ID_Continue}]|\\u…)+`: the end and
/// whether it is private (`#`).
fn identifier(s: &str, i: usize) -> Option<(usize, bool)> {
    let private = at(s, i) == Some('#');
    let begin = if private { i + 1 } else { i };
    match at(s, begin) {
        Some(c) if c == '\\' || is_id_start(c) => {}
        _ => return None,
    }
    let mut j = begin;
    loop {
        match at(s, j) {
            Some(c) if is_id_continue(c) => j += c.len_utf8(),
            Some('\\') => match unicode_escape(s, j) {
                Some(e) => j = e,
                None => break,
            },
            _ => break,
        }
    }
    (j > begin).then_some((j, private))
}

/// `(['"])(?:(?!\1)[^\\\n\r]|\\(?:\r\n|[^]))*(\1)?`
fn string_literal(s: &str, i: usize) -> Option<usize> {
    let q = at(s, i).filter(|c| *c == '\'' || *c == '"')?;
    let mut j = i + 1;
    loop {
        match at(s, j) {
            Some(c) if c == q => return Some(j + 1),
            Some('\\') => {
                if s[j + 1..].starts_with("\r\n") {
                    j += 3;
                } else if at(s, j + 1).is_some() {
                    j = next(s, j + 1);
                } else {
                    return Some(j);
                }
            }
            Some('\n' | '\r') | None => return Some(j),
            Some(c) => j += c.len_utf8(),
        }
    }
}

/// `d(?:_?d)*` at `i` for the digit class `d`: its end, if one digit is there.
fn digits(s: &str, i: usize, d: impl Fn(char) -> bool) -> Option<usize> {
    let b = s.as_bytes();
    let is = |j: usize| j < b.len() && d(b[j] as char);
    if !is(i) {
        return None;
    }
    let mut j = i + 1;
    loop {
        if is(j) {
            j += 1;
        } else if j < b.len() && b[j] == b'_' && is(j + 1) {
            j += 2;
        } else {
            return Some(j);
        }
    }
}

/// js-tokens' `NumericLiteral`, its alternatives in order.
fn numeric_literal(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let ch = |j: usize| b.get(j).map(|x| *x as char);
    let dec = |c: char| c.is_ascii_digit();
    if let Some(end) = numeric_prefixed_or_bigint(s, i) {
        return Some(end);
    }
    // (?:(?:0(?!\d)|0\d*[89]\d*|[1-9](?:_?\d)*)(?:\.(?:\d(?:_?\d)*)?)?|\.\d(?:_?\d)*)(?:[eE]…)?
    let int = numeric_int_head(s, i);
    let mantissa = match int {
        Some(e) if ch(e) == Some('.') => Some(digits(s, e + 1, dec).unwrap_or(e + 1)),
        Some(e) => Some(e),
        None if ch(i) == Some('.') => digits(s, i + 1, dec),
        None => None,
    };
    if let Some(e) = mantissa {
        return Some(numeric_with_exp(s, e).unwrap_or(e));
    }
    // 0[0-7]+ (legacy octal)
    numeric_legacy_octal(b, i)
}

/// `0[xX]…|0[oO]…|0[bB]…` with an optional trailing `n`, or `[1-9](?:_?\d)*n` (plain `BigInt`).
/// `None` when nothing in this alternative matches at `i`.
fn numeric_prefixed_or_bigint(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let ch = |j: usize| b.get(j).map(|x| *x as char);
    let dec = |c: char| c.is_ascii_digit();
    if ch(i) == Some('0') {
        if let Some(end) = numeric_zero_prefix(s, i, ch(i + 1)) {
            return Some(end);
        }
    }
    if ch(i).is_some_and(|c| ('1'..='9').contains(&c)) {
        if let Some(e) = digits(s, i, dec).filter(|e| ch(*e) == Some('n')) {
            return Some(e + 1);
        }
    }
    None
}

/// After `0`: a based-radix run (`0xH`, `0oO`, `0bB`), or a plain `0n` BigInt.
fn numeric_zero_prefix(s: &str, i: usize, next_ch: Option<char>) -> Option<usize> {
    let b = s.as_bytes();
    let ch = |j: usize| b.get(j).map(|x| *x as char);
    let radix: Option<fn(char) -> bool> = match next_ch {
        Some('x' | 'X') => Some(|c: char| c.is_ascii_hexdigit()),
        Some('o' | 'O') => Some(|c: char| ('0'..='7').contains(&c)),
        Some('b' | 'B') => Some(|c: char| c == '0' || c == '1'),
        _ => None,
    };
    if let Some(e) = radix.and_then(|r| digits(s, i + 2, r)) {
        return Some(if ch(e) == Some('n') { e + 1 } else { e });
    }
    if next_ch == Some('n') {
        return Some(i + 2);
    }
    None
}

/// The integer head of a plain-decimal numeric literal (no base prefix, no leading dot): either
/// `0` not followed by a digit, a legacy-octal-falling-through-to-decimal `0\d*[89]\d*` run, or
/// `[1-9](?:_?\d)*`.
fn numeric_int_head(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    let ch = |j: usize| b.get(j).map(|x| *x as char);
    let dec = |c: char| c.is_ascii_digit();
    match ch(i) {
        Some('0') if !ch(i + 1).is_some_and(dec) => Some(i + 1),
        Some('0') => {
            let run = i + 1 + b[i + 1..].iter().take_while(|x| x.is_ascii_digit()).count();
            // backtracking keeps the run up to its last 8 or 9, then digits after it
            b[i + 1..run]
                .iter()
                .rposition(|x| *x == b'8' || *x == b'9')
                .map(|_| run)
        }
        Some('1'..='9') => digits(s, i, dec),
        _ => None,
    }
}

/// Extend `e` through an optional `[eE][+-]?\d(?:_?\d)*` exponent, returning the end of the
/// exponent on a match, `None` when there is no exponent (or it did not complete).
fn numeric_with_exp(s: &str, e: usize) -> Option<usize> {
    let b = s.as_bytes();
    let ch = |j: usize| b.get(j).map(|x| *x as char);
    let dec = |c: char| c.is_ascii_digit();
    if !matches!(ch(e), Some('e' | 'E')) {
        return None;
    }
    let sign = usize::from(matches!(ch(e + 1), Some('+' | '-')));
    digits(s, e + 1 + sign, dec)
}

/// Legacy `0[0-7]+` octal end, or `None` when the bytes at `i` do not start one.
fn numeric_legacy_octal(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i).copied() != Some(b'0') {
        return None;
    }
    let n = b[i + 1..]
        .iter()
        .take_while(|x| (b'0'..=b'7').contains(x))
        .count();
    (n > 0).then_some(i + 1 + n)
}

/// ``[`}](?:[^`\\$]|\\[^]|\$(?!\{))*(`|\$\{)?``: the end and how it ends (`` ` ``, `${` or
/// neither).
fn template(s: &str, i: usize) -> Option<(usize, &'static str)> {
    if !matches!(at(s, i), Some('`' | '}')) {
        return None;
    }
    let mut j = i + 1;
    loop {
        match at(s, j) {
            Some('`') => return Some((j + 1, "`")),
            Some('$') if s[j + 1..].starts_with('{') => return Some((j + 2, "${")),
            Some('\\') => match at(s, j + 1) {
                Some(c) => j = j + 1 + c.len_utf8(),
                None => return Some((j, "")),
            },
            Some(c) => j += c.len_utf8(),
            None => return Some((j, "")),
        }
    }
}

/// `^(?:[\/+-]|\.{3}|\?(?:InterpolationIn(?:JSX|Template)|NoLineTerminatorHere|
/// NonExpressionParenEnd|UnaryIncDec))?$|[{}([,;<>=*%&|^!~?:]$`
fn precedes_expression(t: &str) -> bool {
    matches!(
        t,
        "" | "/"
            | "+"
            | "-"
            | "..."
            | "?InterpolationInJSX"
            | "?InterpolationInTemplate"
            | "?NoLineTerminatorHere"
            | "?NonExpressionParenEnd"
            | "?UnaryIncDec"
    ) || t.ends_with([
        '{', '}', '(', '[', ',', ';', '<', '>', '=', '*', '%', '&', '|', '^', '!', '~', '?', ':',
    ])
}

fn not_preceding_object_literal(t: &str) -> bool {
    matches!(
        t,
        "" | "=>"
            | ";"
            | "]"
            | ")"
            | "{"
            | "}"
            | "else"
            | "?NoLineTerminatorHere"
            | "?NonExpressionParenEnd"
    )
}

fn keyword_with_expression_after(t: &str) -> bool {
    matches!(
        t,
        "await"
            | "case"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "instanceof"
            | "new"
            | "return"
            | "throw"
            | "typeof"
            | "void"
            | "yield"
    )
}

/// js-tokens: the tokens of `input`.
pub fn tokens<'a>(input: &'a str, jsx: bool) -> Vec<Token<'a>> {
    let mut lx = Lexer {
        input,
        jsx,
        out: Vec::new(),
        i: 0,
        last: String::new(),
        stack: vec![Mode {
            tag: Tag::Js,
            nesting: 0,
        }],
        braces: Vec::new(),
        paren_nesting: 0,
        postfix_inc_dec: false,
    };
    while lx.i < input.len() {
        let mode = *lx.stack.last().unwrap();
        let done = match mode.tag {
            Tag::JsxOpen | Tag::JsxClose => lx.jsx_tag(mode),
            Tag::JsxChildren => lx.jsx_children(),
            _ => lx.js(mode),
        };
        if !done && !lx.trivia() {
            lx.invalid(mode);
        }
    }
    lx.out
}

/// The js-tokens state machine over `input`, at `i`.
struct Lexer<'a> {
    input: &'a str,
    jsx: bool,
    out: Vec<Token<'a>>,
    i: usize,
    last: String,
    stack: Vec<Mode>,
    braces: Vec<bool>,
    paren_nesting: i64,
    postfix_inc_dec: bool,
}

impl<'a> Lexer<'a> {
    /// A `kind` token from `i` to `to`, and move there.
    fn push(&mut self, kind: Kind, to: usize) {
        self.out.push(Token {
            kind,
            value: &self.input[self.i..to],
        });
        self.i = to;
    }

    /// The token from `i` to `to` as `last`, then pushed.
    fn push_last(&mut self, kind: Kind, to: usize) {
        self.last = self.input[self.i..to].to_string();
        self.push(kind, to);
    }

    fn expression_next(&self) -> bool {
        precedes_expression(&self.last) || keyword_with_expression_after(&self.last)
    }

    /// A JavaScript-mode token at `i`; whether there was one.
    fn js(&mut self, mode: Mode) -> bool {
        let (input, i) = (self.input, self.i);
        if self.js_try_regex(input, i) {
            return true;
        }
        if let Some(e) = punctuator(input, i) {
            self.js_punctuator(mode, e);
            return true;
        }
        if let Some((e, private)) = identifier(input, i) {
            self.js_identifier(e, private);
            return true;
        }
        self.js_try_literal_or_template(input, i)
    }

    /// Try a regex literal at `i` (only when the cursor is at `/` in an expression context).
    fn js_try_regex(&mut self, input: &'a str, i: usize) -> bool {
        if at(input, i) != Some('/') || !self.expression_next() {
            return false;
        }
        let e = regex_literal(input, i);
        if e <= i {
            return false;
        }
        self.postfix_inc_dec = true;
        self.push_last(Kind::RegularExpressionLiteral, e);
        true
    }

    /// Try the remaining JS token forms: string, numeric, or template.
    fn js_try_literal_or_template(&mut self, input: &'a str, i: usize) -> bool {
        if let Some(e) = string_literal(input, i) {
            self.postfix_inc_dec = true;
            self.push_last(Kind::StringLiteral, e);
            return true;
        }
        if let Some(e) = numeric_literal(input, i) {
            self.postfix_inc_dec = true;
            self.push_last(Kind::NumericLiteral, e);
            return true;
        }
        if let Some((e, end)) = template(input, i) {
            self.js_template(e, end);
            return true;
        }
        false
    }

    /// Record the identifier/keyword `input[i..e]` found by `js`, tracking the lookahead flag used
    /// by `js_punctuator` to tell `for (…)`/`if (…)`/`while (…)`/`with (…)` from an expression
    /// paren. `private` turns a leading `#` into `PrivateIdentifier`.
    fn js_identifier(&mut self, e: usize, private: bool) {
        let word = &self.input[self.i..e];
        let mut next_last = word.to_string();
        if matches!(word, "for" | "if" | "while" | "with") && self.last != "." && self.last != "?."
        {
            next_last = "?NonExpressionParenKeyword".into();
        }
        self.last = next_last;
        self.postfix_inc_dec = !keyword_with_expression_after(word);
        let kind = if private {
            Kind::PrivateIdentifier
        } else {
            Kind::IdentifierName
        };
        self.push(kind, e);
    }

    /// Push the template-literal token `js` found ending at `e`. `end == "${"` opens an
    /// `InterpolationInTemplate` scope and emits `TemplateHead`; otherwise it is a
    /// `NoSubstitutionTemplate`.
    fn js_template(&mut self, e: usize, end: &'static str) {
        if end == "${" {
            self.last = "?InterpolationInTemplate".into();
            self.stack.push(Mode {
                tag: Tag::InterpolationInTemplate,
                nesting: self.braces.len(),
            });
            self.postfix_inc_dec = false;
            self.push(Kind::TemplateHead, e);
        } else {
            self.postfix_inc_dec = true;
            self.push_last(Kind::NoSubstitutionTemplate, e);
        }
    }

    /// The punctuator from `i` to `e`.
    fn js_punctuator(&mut self, mode: Mode, e: usize) {
        let input = self.input;
        let p = &input[self.i..e];
        let mut next_last = p.to_string();
        match p {
            "(" => self.js_punct_open_paren(),
            ")" => self.js_punct_close_paren(mode, &mut next_last),
            "{" => self.js_punct_open_brace(),
            "}" => match self.js_brace_close(mode) {
                Some(n) => next_last = n,
                None => return,
            },
            "]" => self.postfix_inc_dec = true,
            "++" | "--" => {
                next_last = if self.postfix_inc_dec {
                    "?PostfixIncDec".into()
                } else {
                    "?UnaryIncDec".into()
                };
            }
            "<" if self.jsx && self.expression_next() => {
                self.stack.push(Mode {
                    tag: Tag::JsxOpen,
                    nesting: 0,
                });
                self.last = "<".into();
                self.push(Kind::JSXPunctuator, self.i + 1);
                return;
            }
            _ => self.postfix_inc_dec = false,
        }
        self.last = next_last;
        self.push(Kind::Punctuator, e);
    }

    fn js_punct_open_paren(&mut self) {
        if self.last == "?NonExpressionParenKeyword" {
            self.stack.push(Mode {
                tag: Tag::JsNonExpressionParen,
                nesting: self.paren_nesting as usize,
            });
        }
        self.paren_nesting += 1;
        self.postfix_inc_dec = false;
    }

    fn js_punct_close_paren(&mut self, mode: Mode, next_last: &mut String) {
        self.paren_nesting -= 1;
        self.postfix_inc_dec = true;
        if mode.tag == Tag::JsNonExpressionParen && self.paren_nesting == mode.nesting as i64 {
            self.stack.pop();
            *next_last = "?NonExpressionParenEnd".into();
            self.postfix_inc_dec = false;
        }
    }

    fn js_punct_open_brace(&mut self) {
        let is_expression = !not_preceding_object_literal(&self.last) && self.expression_next();
        self.braces.push(is_expression);
        self.postfix_inc_dec = false;
    }

    /// A `}` closing a template interpolation or a JSX one (pushed here: `None`), else a
    /// brace: the `last` it leaves.
    fn js_brace_close(&mut self, mode: Mode) -> Option<String> {
        let i = self.i;
        if mode.tag == Tag::InterpolationInTemplate && self.braces.len() == mode.nesting {
            let (te, end) = template(self.input, i).unwrap();
            if end == "${" {
                self.last = "?InterpolationInTemplate".into();
                self.postfix_inc_dec = false;
                self.push(Kind::TemplateMiddle, te);
            } else {
                self.stack.pop();
                self.postfix_inc_dec = true;
                self.push_last(Kind::TemplateTail, te);
            }
            return None;
        }
        if mode.tag == Tag::InterpolationInJsx && self.braces.len() == mode.nesting {
            self.stack.pop();
            self.last = "}".into();
            self.push(Kind::JSXPunctuator, i + 1);
            return None;
        }
        self.postfix_inc_dec = self.braces.pop().unwrap_or(false);
        Some(if self.postfix_inc_dec {
            "?ExpressionBraceEnd".into()
        } else {
            "}".into()
        })
    }

    /// A JSX-tag token at `i`; whether there was one.
    fn jsx_tag(&mut self, mode: Mode) -> bool {
        let (input, i) = (self.input, self.i);
        let c = at(input, i);
        let punct = match c {
            Some('<' | '>' | '.' | ':' | '=' | '{' | '}') => true,
            Some('/') => !matches!(at(input, i + 1), Some('/' | '*')),
            _ => false,
        };
        if punct {
            self.jsx_tag_punctuator(mode);
            return true;
        }
        if c.is_some_and(is_id_start) {
            let mut e = next(input, i);
            while let Some(d) = at(input, e).filter(|d| *d == '-' || is_id_continue(*d)) {
                e += d.len_utf8();
            }
            self.push_last(Kind::JSXIdentifier, e);
            return true;
        }
        if let Some(q) = c.filter(|c| *c == '\'' || *c == '"') {
            let e = input[i + 1..]
                .find(q)
                .map_or(input.len(), |k| i + 1 + k + 1);
            self.push_last(Kind::JSXString, e);
            return true;
        }
        false
    }

    /// The one-character JSX-tag punctuator at `i`.
    fn jsx_tag_punctuator(&mut self, mode: Mode) {
        let p = &self.input[self.i..self.i + 1];
        let mut next_last = p.to_string();
        match p {
            "<" => self.stack.push(Mode {
                tag: Tag::JsxOpen,
                nesting: 0,
            }),
            ">" => {
                self.stack.pop();
                if self.last == "/" || mode.tag == Tag::JsxClose {
                    next_last = "?JSX".into();
                    self.postfix_inc_dec = true;
                } else {
                    self.stack.push(Mode {
                        tag: Tag::JsxChildren,
                        nesting: 0,
                    });
                }
            }
            "{" => {
                self.stack.push(Mode {
                    tag: Tag::InterpolationInJsx,
                    nesting: self.braces.len(),
                });
                next_last = "?InterpolationInJSX".into();
                self.postfix_inc_dec = false;
            }
            "/" if self.last == "<" => {
                self.stack.pop();
                if self.stack.last().is_some_and(|m| m.tag == Tag::JsxChildren) {
                    self.stack.pop();
                }
                self.stack.push(Mode {
                    tag: Tag::JsxClose,
                    nesting: 0,
                });
            }
            _ => {}
        }
        self.last = next_last;
        self.push(Kind::JSXPunctuator, self.i + 1);
    }

    /// A JSX-children token at `i`; whether there was one.
    fn jsx_children(&mut self) -> bool {
        let (input, i) = (self.input, self.i);
        let e = i + input[i..]
            .find(['<', '>', '{', '}'])
            .unwrap_or(input.len() - i);
        if e > i {
            self.push_last(Kind::JSXText, e);
            return true;
        }
        match at(input, i) {
            Some('<') => {
                self.stack.push(Mode {
                    tag: Tag::JsxOpen,
                    nesting: 0,
                });
                self.last = "<".into();
            }
            Some('{') => {
                self.stack.push(Mode {
                    tag: Tag::InterpolationInJsx,
                    nesting: self.braces.len(),
                });
                self.last = "?InterpolationInJSX".into();
                self.postfix_inc_dec = false;
            }
            _ => return false,
        }
        self.push(Kind::JSXPunctuator, i + 1);
        true
    }

    /// White space, a line terminator or a comment at `i`; whether there was one.
    fn trivia(&mut self) -> bool {
        let (input, i, len) = (self.input, self.i, self.input.len());
        let ws = input[i..]
            .char_indices()
            .find(|(_, c)| !is_white_space(*c))
            .map_or(len, |(k, _)| i + k);
        if ws > i {
            self.push(Kind::WhiteSpace, ws);
            return true;
        }
        let nl = if input[i..].starts_with("\r\n") {
            Some(i + 2)
        } else {
            at(input, i)
                .filter(|c| is_line_terminator(*c))
                .map(|c| i + c.len_utf8())
        };
        if let Some(e) = nl {
            self.line_break();
            self.push(Kind::LineTerminatorSequence, e);
            return true;
        }
        if input[i..].starts_with("/*") {
            let e = input[i + 2..].find("*/").map_or(len, |k| i + 2 + k + 2);
            if input[i..e].contains(is_line_terminator) {
                self.line_break();
            }
            self.push(Kind::MultiLineComment, e);
            return true;
        }
        if input[i..].starts_with("//") {
            let e = input[i..].find(is_line_terminator).map_or(len, |k| i + k);
            self.postfix_inc_dec = false;
            self.push(Kind::SingleLineComment, e);
            return true;
        }
        false
    }

    /// A line break: no postfix `++`/`--` across it, and no expression after `return` & co.
    fn line_break(&mut self) {
        self.postfix_inc_dec = false;
        if matches!(self.last.as_str(), "return" | "throw" | "yield") {
            self.last = "?NoLineTerminatorHere".into();
        }
    }

    /// The character at `i` as an invalid token.
    fn invalid(&mut self, mode: Mode) {
        let e = next(self.input, self.i);
        self.postfix_inc_dec = false;
        let kind = if matches!(mode.tag, Tag::JsxOpen | Tag::JsxClose | Tag::JsxChildren) {
            Kind::JSXInvalid
        } else {
            Kind::Invalid
        };
        self.push_last(kind, e);
    }
}

/// tinyhighlight's token types beyond js-tokens'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type {
    Token(Kind),
    Keyword,
    IdentifierCapitalized,
    Bracket,
    IdentifierCallable,
    PrivateIdentifierCallable,
}

const KEYWORDS: [&str; 36] = [
    "break",
    "case",
    "catch",
    "continue",
    "debugger",
    "default",
    "do",
    "else",
    "finally",
    "for",
    "function",
    "if",
    "return",
    "switch",
    "throw",
    "try",
    "var",
    "const",
    "while",
    "with",
    "new",
    "this",
    "super",
    "class",
    "extends",
    "export",
    "import",
    "null",
    "true",
    "false",
    "in",
    "instanceof",
    "typeof",
    "void",
    "delete",
    "await",
];
const STRICT_AND_SOMETIMES: [&str; 17] = [
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
    "yield",
    "enum",
    "as",
    "async",
    "from",
    "get",
    "of",
    "set",
    "await",
];

fn token_type(t: &Token) -> Type {
    match t.kind {
        Kind::IdentifierName => {
            if KEYWORDS.contains(&t.value) || STRICT_AND_SOMETIMES.contains(&t.value) {
                return Type::Keyword;
            }
            let first = t.value.chars().next().unwrap_or('a');
            if first.to_lowercase().next() != Some(first) {
                return Type::IdentifierCapitalized;
            }
            Type::Token(Kind::IdentifierName)
        }
        Kind::Punctuator if matches!(t.value, "(" | ")" | "[" | "]" | "{" | "}") => Type::Bracket,
        Kind::Invalid if t.value == "@" || t.value == "#" => Type::Token(Kind::Punctuator),
        k => Type::Token(k),
    }
}

/// `v` split `n` bytes before its end.
fn split_end(v: &str, n: usize) -> (&str, &str) {
    v.split_at(v.len() - n.min(v.len()))
}

/// vitest's `getDefs` colours for a token type.
fn colorize(c: &Colors, ty: Type, v: &str) -> String {
    let invalid = |v: &str| c.white(&c.bg_red(&c.bold(v)));
    match ty {
        Type::Keyword => c.magenta(v),
        Type::IdentifierCapitalized | Type::Token(Kind::Punctuator) => c.yellow(v),
        Type::Token(Kind::StringLiteral | Kind::NoSubstitutionTemplate | Kind::JSXString) => {
            c.green(v)
        }
        Type::Token(Kind::MultiLineComment | Kind::SingleLineComment) => c.gray(v),
        Type::Token(Kind::RegularExpressionLiteral) => c.cyan(v),
        Type::Token(Kind::NumericLiteral) | Type::IdentifierCallable => c.blue(v),
        Type::Token(Kind::TemplateHead) => {
            let (a, b) = split_end(v, 2);
            c.green(a) + &c.cyan(b)
        }
        Type::Token(Kind::TemplateTail) => {
            let (a, b) = v.split_at(1.min(v.len()));
            c.cyan(a) + &c.green(b)
        }
        Type::Token(Kind::TemplateMiddle) => {
            let (a, rest) = v.split_at(1.min(v.len()));
            let (m, z) = split_end(rest, 2);
            c.cyan(a) + &c.green(m) + &c.cyan(z)
        }
        Type::PrivateIdentifierCallable => format!("#{}", c.blue(&v[1.min(v.len())..])),
        Type::Token(Kind::Invalid | Kind::JSXInvalid) => invalid(v),
        Type::Token(Kind::JSXIdentifier | Kind::JSXPunctuator) => c.yellow(v),
        _ => v.to_string(),
    }
}

/// tinyhighlight's `highlightTokens`.
pub fn highlight(c: &Colors, code: &str, jsx: bool) -> String {
    let mut out = String::new();
    let mut pending: Option<Token> = None;
    let mut stacked = String::new();
    for t in tokens(code, jsx) {
        let ty = token_type(&t);
        if matches!(
            ty,
            Type::Token(Kind::IdentifierName | Kind::PrivateIdentifier)
        ) {
            if let Some(p) = pending {
                out.push_str(&colorize(c, token_type(&p), p.value));
                out.push_str(&stacked);
                stacked.clear();
            }
            pending = Some(t);
            continue;
        }
        if pending.is_some() && is_pending_passthrough(&t) {
            stacked.push_str(&colorize(c, ty, t.value));
            continue;
        }
        if let Some(p) = pending.take() {
            let pty = pending_type_against(&p, &t);
            out.push_str(&colorize(c, pty, p.value));
            out.push_str(&stacked);
            stacked.clear();
        }
        out.push_str(&colorize(c, ty, t.value));
    }
    out
}

/// A token that `highlight` carries over a pending identifier without flushing it: whitespace, a
/// line terminator, or the `?.` / `!` optional-chain / non-null punctuator.
fn is_pending_passthrough(t: &Token) -> bool {
    matches!(t.kind, Kind::WhiteSpace | Kind::LineTerminatorSequence)
        || t.kind == Kind::Punctuator && (t.value == "?." || t.value == "!")
}

/// The `Type` to color the pending identifier `p` with, now that `t` follows: a `(` after it turns
/// it into a callable; otherwise it keeps its own `token_type`.
fn pending_type_against(p: &Token, t: &Token) -> Type {
    if t.kind == Kind::Punctuator && t.value == "(" {
        if p.kind == Kind::PrivateIdentifier {
            Type::PrivateIdentifierCallable
        } else {
            Type::IdentifierCallable
        }
    } else {
        token_type(p)
    }
}

/// vitest's `highlightCode`: `source` highlighted when `file` is JS / TS (JSX for `…x`).
pub fn highlight_code(c: &Colors, file: &Path, source: &str) -> String {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = ext.strip_prefix(['m', 'c']).unwrap_or(ext);
    if !matches!(lang, "js" | "ts" | "jsx" | "tsx") {
        return source.to_string();
    }
    if source.is_empty() {
        return String::new();
    }
    highlight(c, source, ext.ends_with('x'))
}

#[cfg(test)]
#[path = "highlight_tests.rs"]
mod tests;
