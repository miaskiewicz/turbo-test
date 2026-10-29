use super::*;

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| {
        pairs
            .iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| v.to_string())
    }
}

#[test]
fn agents_are_std_envs() {
    assert!(!is_agent(env(&[]), false));
    for var in AGENT_VARS {
        assert!(is_agent(env(&[(var, "1")]), false), "{var}");
        // an empty value does not count
        assert!(!is_agent(env(&[(var, "")]), false), "{var}");
    }
    assert!(is_agent(
        env(&[("PATH", "/usr/bin:/home/u/.pi/agent/bin")]),
        true
    ));
    assert!(is_agent(env(&[("PATH", "C:\\u\\.pi\\agent")]), true));
    assert!(!is_agent(
        env(&[("PATH", "/usr/bin:/home/u/.pi/bin")]),
        false
    ));
    assert!(is_agent(env(&[("EDITOR", "/opt/devin/edit")]), true));
    assert!(!is_agent(env(&[("EDITOR", "vim")]), false));
    // Kiro counts only when stdout is not a terminal
    assert!(is_agent(env(&[("TERM_PROGRAM", "kiro")]), false));
    assert!(!is_agent(env(&[("TERM_PROGRAM", "kiro")]), true));
    assert!(!is_agent(env(&[("TERM_PROGRAM", "iTerm.app")]), false));
}
