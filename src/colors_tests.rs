use super::*;

const C: Colors = Colors { on: true };

#[test]
fn colours_wrap_in_their_codes() {
    assert_eq!(C.red("x"), "\x1b[31mx\x1b[39m");
    assert_eq!(C.gray(""), "\x1b[90m\x1b[39m");
    assert_eq!(C.bold("b"), "\x1b[1mb\x1b[22m");
    assert_eq!(C.bg_cyan(" RUN "), "\x1b[46m RUN \x1b[49m");
    // off: the string itself
    let off = Colors { on: false };
    assert_eq!(off.red("x"), "x");
    assert_eq!(off.dim(&C.red("x")), C.red("x"));
}

#[test]
fn a_nested_close_resumes_the_outer_colour() {
    // vitest's failed test line: c.red(` ${pad}${c.red('×')} name`)
    assert_eq!(
        C.red(&format!("   {} fails", C.red("×"))),
        "\x1b[31m   \x1b[31m×\x1b[31m fails\x1b[39m"
    );
    // bold / dim share their close: it resumes with `\x1b[22m` + open
    assert_eq!(
        C.dim(&format!(" | a{}b", C.dim(" > "))),
        "\x1b[2m | a\x1b[2m > \x1b[22m\x1b[2mb\x1b[22m"
    );
    // every close after the first is replaced too
    assert_eq!(
        C.green(&format!("{}{}", C.red("ab"), C.red("c"))),
        "\x1b[32m\x1b[31mab\x1b[32m\x1b[31mc\x1b[32m\x1b[39m"
    );
    // a close within the first open.len() units is left alone (tinyrainbow searches from there)
    assert_eq!(C.red("\x1b[39m"), "\x1b[31m\x1b[39m\x1b[39m");
    assert_eq!(C.red("ab\x1b[39m"), "\x1b[31mab\x1b[39m\x1b[39m");
    assert_eq!(C.red("abcde\x1b[39m"), "\x1b[31mabcde\x1b[31m\x1b[39m");
    // the units are UTF-16 ones: `✓` is one unit (3 bytes), `𝒳` two (4 bytes)
    assert_eq!(C.red("✓✓✓✓✓\x1b[39m"), "\x1b[31m✓✓✓✓✓\x1b[31m\x1b[39m");
    assert_eq!(C.red("𝒳𝒳\x1b[39m"), "\x1b[31m𝒳𝒳\x1b[39m\x1b[39m");
    assert_eq!(C.red("𝒳𝒳𝒳\x1b[39m"), "\x1b[31m𝒳𝒳𝒳\x1b[31m\x1b[39m");
}

#[test]
fn colours_are_tinyrainbows_is_supported() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    };
    let none: &[String] = &[];
    // no TERM at all still counts as not `dumb`
    assert!(supported(env(&[]), none));
    assert!(supported(env(&[("TERM", "xterm")]), none));
    assert!(!supported(env(&[("TERM", "dumb")]), none));
    assert!(!supported(env(&[("FORCE_TTY", "false")]), none));
    assert!(supported(env(&[("FORCE_TTY", "false"), ("CI", "")]), none));
    assert!(supported(
        env(&[("TERM", "dumb"), ("FORCE_COLOR", "0")]),
        none
    ));
    assert!(supported(
        env(&[("TERM", "dumb")]),
        &["--color".to_string()]
    ));
    // NO_COLOR (even empty) and --no-color win
    assert!(!supported(
        env(&[("NO_COLOR", ""), ("FORCE_COLOR", "1")]),
        none
    ));
    assert!(!supported(
        env(&[("FORCE_COLOR", "1")]),
        &["--no-color".to_string()]
    ));
}

#[test]
fn strip_drops_the_codes() {
    assert_eq!(strip(&C.red(&format!("a{}b", C.dim("c")))), "acb");
    assert_eq!(strip("plain ✓"), "plain ✓");
    // an ESC not starting a sequence stays
    assert_eq!(strip("a\x1bb"), "a\x1bb");
}
