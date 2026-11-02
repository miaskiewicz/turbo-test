//! AI-agent detection, as vitest decides it: std-env's `isAgent` (std-env 4.3.0). Under an agent
//! vitest 4.1.12 makes its `agent` reporter the default and turns colours off (turbo-test#94).
//!
//! std-env: `AI_AGENT` set to a non-empty value names the agent; else the first agent whose
//! variable is set (or matches): Claude (`CLAUDECODE`, `CLAUDE_CODE`), Replit (`REPL_ID`), Gemini
//! (`GEMINI_CLI`), Codex (`CODEX_SANDBOX`, `CODEX_THREAD_ID`), OpenCode (`OPENCODE`), Pi (`PATH`
//! containing `.pi/agent`), Auggie (`AUGMENT_AGENT`), Goose (`GOOSE_PROVIDER`), Junie
//! (`JUNIE_DATA`, `JUNIE_SHIM_PATH`), Copilot (`COPILOT_AGENT`, `COPILOT_CLI`), Devin (`EDITOR`
//! containing `devin`), Cursor (`CURSOR_AGENT`), Kiro (`TERM_PROGRAM` containing `kiro`, when
//! stdout is not a terminal).

/// The variables that name an agent when set to a non-empty value.
pub const AGENT_VARS: [&str; 15] = [
    "AI_AGENT",
    "CLAUDECODE",
    "CLAUDE_CODE",
    "REPL_ID",
    "GEMINI_CLI",
    "CODEX_SANDBOX",
    "CODEX_THREAD_ID",
    "OPENCODE",
    "AUGMENT_AGENT",
    "GOOSE_PROVIDER",
    "JUNIE_DATA",
    "JUNIE_SHIM_PATH",
    "COPILOT_AGENT",
    "COPILOT_CLI",
    "CURSOR_AGENT",
];

/// Whether an agent runs the tests, as std-env's `isAgent`, with `var` looking up the
/// environment and `tty` whether stdout is a terminal.
pub fn is_agent(var: impl Fn(&str) -> Option<String>, tty: bool) -> bool {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    let contains = |k: &str, needle: &str| var(k).is_some_and(|v| v.contains(needle));
    AGENT_VARS.iter().any(|k| set(k))
        || contains("PATH", ".pi/agent")
        || contains("PATH", ".pi\\agent")
        || contains("EDITOR", "devin")
        || (!tty && contains("TERM_PROGRAM", "kiro"))
}

/// [`is_agent`] over the process environment.
pub fn is_agent_env() -> bool {
    use std::io::IsTerminal;
    is_agent(|k| std::env::var(k).ok(), std::io::stdout().is_terminal())
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
