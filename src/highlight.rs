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
            Some('[') => {
                // a class closes at the first unescaped `]` on the line
                let mut j = i + 1;
                let closed = loop {
                    match at(s, j) {
                        Some(']') => break Some(j + 1),
                        Some('\\') if at(s, j + 1).is_some_and(plain) => j = next(s, j + 1),
                        Some(c) if c != '\\' && plain(c) => j = next(s, j),
                        _ => break None,
                    }
                };
                i = closed.unwrap_or(i + 1);
            }
            Some('\\') if at(s, i + 1).is_some_and(plain) => i = next(s, i + 1),
            Some(c) if c != '/' && c != '\\' && plain(c) => i = next(s, i),
            _ => break,
        }
    }
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
    for p in ["--", "++", "=>", "..."] {
        if r.starts_with(p) {
            return Some(i + p.len());
        }
    }
    // \??\.(?!\d)
    let not_digit = |j: usize| !at(s, j).is_some_and(|c| c.is_ascii_digit());
    if r.starts_with("?.") && not_digit(i + 2) {
        return Some(i + 2);
    }
    if r.starts_with('.') && not_digit(i + 1) {
        return Some(i + 1);
    }
    // (?:&&|\|\||\?\?|[+\-%&|^]|\*{1,2}|<{1,2}|>{1,3}|!=?|={1,2}|\/(?![\/*]))=?
    let op = if r.starts_with("&&") || r.starts_with("||") || r.starts_with("??") {
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
    };
    if let Some(n) = op {
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

/// `\\u[\da-fA-F]{4}|\\u\{[\da-fA-F]+\}` at `i`: its end.
fn unicode_escape(s: &str, i: usize) -> Option<usize> {
    let r = s.get(i..)?;
    let hex = |c: char| c.is_ascii_hexdigit();
    if !r.starts_with("\\u") {
        return None;
    }
    let r = &r[2..];
    if r.len() >= 4 && r[..4].chars().all(hex) {
        return Some(i + 6);
    }
    if let Some(body) = r.strip_prefix('{') {
        let n = body.chars().take_while(|c| hex(*c)).count();
        if n > 0 && body[n..].starts_with('}') {
            return Some(i + 3 + n + 1);
        }
    }
    None
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
    // 0[xX]…|0[oO]…|0[bB]… then n?
    if ch(i) == Some('0') {
        let radix: Option<fn(char) -> bool> = match ch(i + 1) {
            Some('x' | 'X') => Some(|c: char| c.is_ascii_hexdigit()),
            Some('o' | 'O') => Some(|c: char| ('0'..='7').contains(&c)),
            Some('b' | 'B') => Some(|c: char| c == '0' || c == '1'),
            _ => None,
        };
        if let Some(e) = radix.and_then(|r| digits(s, i + 2, r)) {
            return Some(if ch(e) == Some('n') { e + 1 } else { e });
        }
        if ch(i + 1) == Some('n') {
            return Some(i + 2);
        }
    }
    // [1-9](?:_?\d)*n
    if ch(i).is_some_and(|c| ('1'..='9').contains(&c)) {
        if let Some(e) = digits(s, i, dec).filter(|e| ch(*e) == Some('n')) {
            return Some(e + 1);
        }
    }
    // (?:(?:0(?!\d)|0\d*[89]\d*|[1-9](?:_?\d)*)(?:\.(?:\d(?:_?\d)*)?)?|\.\d(?:_?\d)*)(?:[eE]…)?
    let int = match ch(i) {
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
    };
    let mantissa = match int {
        Some(e) if ch(e) == Some('.') => Some(digits(s, e + 1, dec).unwrap_or(e + 1)),
        Some(e) => Some(e),
        None if ch(i) == Some('.') => digits(s, i + 1, dec),
        None => None,
    };
    if let Some(e) = mantissa {
        if matches!(ch(e), Some('e' | 'E')) {
            let sign = usize::from(matches!(ch(e + 1), Some('+' | '-')));
            if let Some(x) = digits(s, e + 1 + sign, dec) {
                return Some(x);
            }
        }
        return Some(e);
    }
    // 0[0-7]+
    if ch(i) == Some('0') {
        let n = b[i + 1..]
            .iter()
            .take_while(|x| (b'0'..=b'7').contains(x))
            .count();
        if n > 0 {
            return Some(i + 1 + n);
        }
    }
    None
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
    let mut out = Vec::new();
    let mut i = 0;
    let mut last: String = String::new();
    let mut stack = vec![Mode {
        tag: Tag::Js,
        nesting: 0,
    }];
    let mut braces: Vec<bool> = Vec::new();
    let mut paren_nesting: i64 = 0;
    let mut postfix_inc_dec = false;
    let len = input.len();
    let expression_next =
        |last: &str| precedes_expression(last) || keyword_with_expression_after(last);
    while i < len {
        let mode = *stack.last().unwrap();
        let push = |kind: Kind, from: usize, to: usize, out: &mut Vec<Token<'a>>| {
            out.push(Token {
                kind,
                value: &input[from..to],
            });
        };
        match mode.tag {
            Tag::Js
            | Tag::JsNonExpressionParen
            | Tag::InterpolationInTemplate
            | Tag::InterpolationInJsx => {
                if at(input, i) == Some('/') && expression_next(&last) {
                    let e = regex_literal(input, i);
                    if e > i {
                        last = input[i..e].to_string();
                        postfix_inc_dec = true;
                        push(Kind::RegularExpressionLiteral, i, e, &mut out);
                        i = e;
                        continue;
                    }
                }
                if let Some(e) = punctuator(input, i) {
                    let p = &input[i..e];
                    let mut next_last = p.to_string();
                    match p {
                        "(" => {
                            if last == "?NonExpressionParenKeyword" {
                                stack.push(Mode {
                                    tag: Tag::JsNonExpressionParen,
                                    nesting: paren_nesting as usize,
                                });
                            }
                            paren_nesting += 1;
                            postfix_inc_dec = false;
                        }
                        ")" => {
                            paren_nesting -= 1;
                            postfix_inc_dec = true;
                            if mode.tag == Tag::JsNonExpressionParen
                                && paren_nesting == mode.nesting as i64
                            {
                                stack.pop();
                                next_last = "?NonExpressionParenEnd".into();
                                postfix_inc_dec = false;
                            }
                        }
                        "{" => {
                            let is_expression =
                                !not_preceding_object_literal(&last) && expression_next(&last);
                            braces.push(is_expression);
                            postfix_inc_dec = false;
                        }
                        "}" => {
                            if mode.tag == Tag::InterpolationInTemplate
                                && braces.len() == mode.nesting
                            {
                                let (te, end) = template(input, i).unwrap();
                                last = input[i..te].to_string();
                                if end == "${" {
                                    last = "?InterpolationInTemplate".into();
                                    postfix_inc_dec = false;
                                    push(Kind::TemplateMiddle, i, te, &mut out);
                                } else {
                                    stack.pop();
                                    postfix_inc_dec = true;
                                    push(Kind::TemplateTail, i, te, &mut out);
                                }
                                i = te;
                                continue;
                            }
                            if mode.tag == Tag::InterpolationInJsx && braces.len() == mode.nesting {
                                stack.pop();
                                last = "}".into();
                                push(Kind::JSXPunctuator, i, i + 1, &mut out);
                                i += 1;
                                continue;
                            }
                            postfix_inc_dec = braces.pop().unwrap_or(false);
                            next_last = if postfix_inc_dec {
                                "?ExpressionBraceEnd".into()
                            } else {
                                "}".into()
                            };
                        }
                        "]" => postfix_inc_dec = true,
                        "++" | "--" => {
                            next_last = if postfix_inc_dec {
                                "?PostfixIncDec".into()
                            } else {
                                "?UnaryIncDec".into()
                            };
                        }
                        "<" if jsx && expression_next(&last) => {
                            stack.push(Mode {
                                tag: Tag::JsxOpen,
                                nesting: 0,
                            });
                            last = "<".into();
                            push(Kind::JSXPunctuator, i, i + 1, &mut out);
                            i += 1;
                            continue;
                        }
                        _ => postfix_inc_dec = false,
                    }
                    last = next_last;
                    push(Kind::Punctuator, i, e, &mut out);
                    i = e;
                    continue;
                }
                if let Some((e, private)) = identifier(input, i) {
                    let word = &input[i..e];
                    let mut next_last = word.to_string();
                    if matches!(word, "for" | "if" | "while" | "with")
                        && last != "."
                        && last != "?."
                    {
                        next_last = "?NonExpressionParenKeyword".into();
                    }
                    last = next_last;
                    postfix_inc_dec = !keyword_with_expression_after(word);
                    let kind = if private {
                        Kind::PrivateIdentifier
                    } else {
                        Kind::IdentifierName
                    };
                    push(kind, i, e, &mut out);
                    i = e;
                    continue;
                }
                if let Some(e) = string_literal(input, i) {
                    last = input[i..e].to_string();
                    postfix_inc_dec = true;
                    push(Kind::StringLiteral, i, e, &mut out);
                    i = e;
                    continue;
                }
                if let Some(e) = numeric_literal(input, i) {
                    last = input[i..e].to_string();
                    postfix_inc_dec = true;
                    push(Kind::NumericLiteral, i, e, &mut out);
                    i = e;
                    continue;
                }
                if let Some((e, end)) = template(input, i) {
                    last = input[i..e].to_string();
                    if end == "${" {
                        last = "?InterpolationInTemplate".into();
                        stack.push(Mode {
                            tag: Tag::InterpolationInTemplate,
                            nesting: braces.len(),
                        });
                        postfix_inc_dec = false;
                        push(Kind::TemplateHead, i, e, &mut out);
                    } else {
                        postfix_inc_dec = true;
                        push(Kind::NoSubstitutionTemplate, i, e, &mut out);
                    }
                    i = e;
                    continue;
                }
            }
            Tag::JsxOpen | Tag::JsxClose => {
                let c = at(input, i);
                let punct = match c {
                    Some('<' | '>' | '.' | ':' | '=' | '{' | '}') => true,
                    Some('/') => !matches!(at(input, i + 1), Some('/' | '*')),
                    _ => false,
                };
                if punct {
                    let p = &input[i..i + 1];
                    let mut next_last = p.to_string();
                    match p {
                        "<" => stack.push(Mode {
                            tag: Tag::JsxOpen,
                            nesting: 0,
                        }),
                        ">" => {
                            stack.pop();
                            if last == "/" || mode.tag == Tag::JsxClose {
                                next_last = "?JSX".into();
                                postfix_inc_dec = true;
                            } else {
                                stack.push(Mode {
                                    tag: Tag::JsxChildren,
                                    nesting: 0,
                                });
                            }
                        }
                        "{" => {
                            stack.push(Mode {
                                tag: Tag::InterpolationInJsx,
                                nesting: braces.len(),
                            });
                            next_last = "?InterpolationInJSX".into();
                            postfix_inc_dec = false;
                        }
                        "/" if last == "<" => {
                            stack.pop();
                            if stack.last().is_some_and(|m| m.tag == Tag::JsxChildren) {
                                stack.pop();
                            }
                            stack.push(Mode {
                                tag: Tag::JsxClose,
                                nesting: 0,
                            });
                        }
                        _ => {}
                    }
                    last = next_last;
                    push(Kind::JSXPunctuator, i, i + 1, &mut out);
                    i += 1;
                    continue;
                }
                if c.is_some_and(is_id_start) {
                    let mut e = next(input, i);
                    while let Some(d) = at(input, e).filter(|d| *d == '-' || is_id_continue(*d)) {
                        e += d.len_utf8();
                    }
                    last = input[i..e].to_string();
                    push(Kind::JSXIdentifier, i, e, &mut out);
                    i = e;
                    continue;
                }
                if let Some(q) = c.filter(|c| *c == '\'' || *c == '"') {
                    let e = input[i + 1..].find(q).map_or(len, |k| i + 1 + k + 1);
                    last = input[i..e].to_string();
                    push(Kind::JSXString, i, e, &mut out);
                    i = e;
                    continue;
                }
            }
            Tag::JsxChildren => {
                let e = i + input[i..].find(['<', '>', '{', '}']).unwrap_or(len - i);
                if e > i {
                    last = input[i..e].to_string();
                    push(Kind::JSXText, i, e, &mut out);
                    i = e;
                    continue;
                }
                match at(input, i) {
                    Some('<') => {
                        stack.push(Mode {
                            tag: Tag::JsxOpen,
                            nesting: 0,
                        });
                        last = "<".into();
                        push(Kind::JSXPunctuator, i, i + 1, &mut out);
                        i += 1;
                        continue;
                    }
                    Some('{') => {
                        stack.push(Mode {
                            tag: Tag::InterpolationInJsx,
                            nesting: braces.len(),
                        });
                        last = "?InterpolationInJSX".into();
                        postfix_inc_dec = false;
                        push(Kind::JSXPunctuator, i, i + 1, &mut out);
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
            }
        }
        let ws = input[i..]
            .char_indices()
            .find(|(_, c)| !is_white_space(*c))
            .map_or(len, |(k, _)| i + k);
        if ws > i {
            push(Kind::WhiteSpace, i, ws, &mut out);
            i = ws;
            continue;
        }
        let nl = if input[i..].starts_with("\r\n") {
            Some(i + 2)
        } else {
            at(input, i)
                .filter(|c| is_line_terminator(*c))
                .map(|c| i + c.len_utf8())
        };
        if let Some(e) = nl {
            postfix_inc_dec = false;
            if matches!(last.as_str(), "return" | "throw" | "yield") {
                last = "?NoLineTerminatorHere".into();
            }
            push(Kind::LineTerminatorSequence, i, e, &mut out);
            i = e;
            continue;
        }
        if input[i..].starts_with("/*") {
            let e = input[i + 2..].find("*/").map_or(len, |k| i + 2 + k + 2);
            if input[i..e].contains(is_line_terminator) {
                postfix_inc_dec = false;
                if matches!(last.as_str(), "return" | "throw" | "yield") {
                    last = "?NoLineTerminatorHere".into();
                }
            }
            push(Kind::MultiLineComment, i, e, &mut out);
            i = e;
            continue;
        }
        if input[i..].starts_with("//") {
            let e = input[i..].find(is_line_terminator).map_or(len, |k| i + k);
            postfix_inc_dec = false;
            push(Kind::SingleLineComment, i, e, &mut out);
            i = e;
            continue;
        }
        let e = next(input, i);
        last = input[i..e].to_string();
        postfix_inc_dec = false;
        let kind = if matches!(mode.tag, Tag::JsxOpen | Tag::JsxClose | Tag::JsxChildren) {
            Kind::JSXInvalid
        } else {
            Kind::Invalid
        };
        push(kind, i, e, &mut out);
        i = e;
    }
    out
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
        if pending.is_some()
            && (matches!(t.kind, Kind::WhiteSpace | Kind::LineTerminatorSequence)
                || t.kind == Kind::Punctuator && (t.value == "?." || t.value == "!"))
        {
            stacked.push_str(&colorize(c, ty, t.value));
            continue;
        }
        if let Some(p) = pending.take() {
            let pty = if t.kind == Kind::Punctuator && t.value == "(" {
                if p.kind == Kind::PrivateIdentifier {
                    Type::PrivateIdentifierCallable
                } else {
                    Type::IdentifierCallable
                }
            } else {
                token_type(&p)
            };
            out.push_str(&colorize(c, pty, p.value));
            out.push_str(&stacked);
            stacked.clear();
        }
        out.push_str(&colorize(c, ty, t.value));
    }
    out
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
