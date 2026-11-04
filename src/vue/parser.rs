//! SFC block parser (<script setup>, <script>, <template>, <style>).

/// Extracted blocks of a Vue SFC.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct SfcDescriptor<'a> {
    pub script_setup: Option<SfcBlock<'a>>,
    pub script: Option<SfcBlock<'a>>,
    pub template: Option<SfcBlock<'a>>,
    pub has_multiple_script_setup: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct SfcBlock<'a> {
    pub content: &'a str,
    pub lang: Option<&'a str>,
    pub is_setup: bool,
}

/// Parse raw `.vue` file content into SFC blocks.
pub fn parse_sfc(src: &str) -> SfcDescriptor<'_> {
    let mut desc = SfcDescriptor::default();
    let mut rest = src;

    while let Some(tag_start) = rest.find('<') {
        rest = &rest[tag_start..];
        if rest.starts_with("<!--") {
            if let Some(comment_end) = rest.find("-->") {
                rest = &rest[comment_end + 3..];
                continue;
            } else {
                break;
            }
        }

        if rest.starts_with("<script") {
            if let Some((block, remainder)) = extract_block(rest, "script") {
                if block.is_setup {
                    if desc.script_setup.is_some() {
                        desc.has_multiple_script_setup = true;
                    }
                    desc.script_setup = Some(block);
                } else {
                    desc.script = Some(block);
                }
                rest = remainder;
                continue;
            }
        } else if rest.starts_with("<template") {
            if let Some((block, remainder)) = extract_template_block(rest) {
                desc.template = Some(block);
                rest = remainder;
                continue;
            }
        } else if rest.starts_with("<style") {
            if let Some((_block, remainder)) = extract_block(rest, "style") {
                rest = remainder;
                continue;
            }
        }

        // Advance by at least 1 byte if nothing matched
        if !rest.is_empty() {
            rest = &rest[1..];
        }
    }

    desc
}

/// Extract `<tag ...>content</tag>`
fn extract_block<'a>(src: &'a str, tag_name: &str) -> Option<(SfcBlock<'a>, &'a str)> {
    let open_end = src.find('>')?;
    let open_tag = &src[..open_end];

    let close_tag = format!("</{tag_name}>");
    let content_start = open_end + 1;
    let content_end = src[content_start..].find(&close_tag)?;
    let content = &src[content_start..content_start + content_end];
    let remainder = &src[content_start + content_end + close_tag.len()..];

    let is_setup = tag_name == "script"
        && (open_tag.contains(" setup ")
            || open_tag.ends_with(" setup")
            || open_tag.contains(" setup="));
    let lang = extract_attr_value(open_tag, "lang");

    Some((
        SfcBlock {
            content,
            lang,
            is_setup,
        },
        remainder,
    ))
}

/// Extract `<template>...</template>`, properly handling nested `<template>` tags (e.g. slots).
fn extract_template_block<'a>(src: &'a str) -> Option<(SfcBlock<'a>, &'a str)> {
    let open_end = src.find('>')?;
    let open_tag = &src[..open_end];
    let lang = extract_attr_value(open_tag, "lang");

    let content_start = open_end + 1;
    let mut depth = 1;
    let mut cursor = content_start;

    while depth > 0 && cursor < src.len() {
        if let Some(next_lt) = src[cursor..].find('<') {
            let tag_idx = cursor + next_lt;
            let slice = &src[tag_idx..];

            if slice.starts_with("<!--") {
                if let Some(comment_end) = slice.find("-->") {
                    cursor = tag_idx + comment_end + 3;
                    continue;
                }
            }

            if slice.starts_with("<template")
                && (slice[9..].starts_with('>')
                    || slice[9..].starts_with(' ')
                    || slice[9..].starts_with('\n')
                    || slice[9..].starts_with('\t'))
            {
                depth += 1;
                cursor = tag_idx + 9;
            } else if slice.starts_with("</template>") {
                depth -= 1;
                if depth == 0 {
                    let content = &src[content_start..tag_idx];
                    let remainder = &src[tag_idx + "</template>".len()..];
                    return Some((
                        SfcBlock {
                            content,
                            lang,
                            is_setup: false,
                        },
                        remainder,
                    ));
                }
                cursor = tag_idx + "</template>".len();
            } else {
                cursor = tag_idx + 1;
            }
        } else {
            break;
        }
    }

    None
}

/// Extract attribute value from tag string (e.g. `lang="ts"` -> `"ts"`).
pub fn extract_attr_value<'a>(tag_str: &'a str, attr_name: &str) -> Option<&'a str> {
    let pattern = format!("{attr_name}=");
    let idx = tag_str.find(&pattern)?;
    let rest = &tag_str[idx + pattern.len()..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let after_quote = &rest[1..];
        let end_quote = after_quote.find(quote)?;
        Some(&after_quote[..end_quote])
    } else {
        let end_space = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(&rest[..end_space])
    }
}

