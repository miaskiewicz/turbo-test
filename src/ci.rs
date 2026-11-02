//! CI detection, as vitest decides it: std-env's `isCI`.
//!
//! vitest 4.1.12 takes `isCI` from std-env 4.3.0 (snapshot update mode `none` on CI, `new`
//! elsewhere). There the run is on CI when `CI` is set to any non-empty value (`false` and `0`
//! included), or when the first provider row whose variable is non-empty marks a CI provider.
//! A few rows name local or preview environments (`NETLIFY_LOCAL`, `VERCEL`, `CODESANDBOX_*`)
//! and do not count; test environments (`TEST`, `NODE_ENV=test`) play no part.
//!
//! The provider table is turbo-test data, `ci_providers.json`, restated from std-env (see its
//! `source` note) and checked row by row against an installed std-env by
//! `scripts/check-ci-detection.mjs`.

use std::sync::OnceLock;

/// One row of the provider table: a provider's environment variable and whether that provider
/// counts as CI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub provider: String,
    pub env: String,
    pub ci: bool,
}

/// The provider table, in std-env's order.
pub fn providers() -> &'static [Provider] {
    static TABLE: OnceLock<Vec<Provider>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("ci_providers.json")).expect("ci_providers.json");
        let field = |row: &serde_json::Value, k: &str| row[k].as_str().expect(k).to_string();
        data["providers"]
            .as_array()
            .expect("providers")
            .iter()
            .map(|row| Provider {
                provider: field(row, "provider"),
                env: field(row, "env"),
                ci: row["ci"].as_bool().expect("ci"),
            })
            .collect()
    })
}

/// Every environment variable the decision reads: `CI`, then each provider's.
pub fn env_vars() -> impl Iterator<Item = &'static str> {
    std::iter::once("CI").chain(providers().iter().map(|p| p.env.as_str()))
}

/// Whether the run is on CI, as std-env's `isCI`, with `var` looking up the environment.
pub fn is_ci(var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    set("CI")
        || providers()
            .iter()
            .find(|p| set(&p.env))
            .is_some_and(|p| p.ci)
}

/// [`is_ci`] over the process environment.
pub fn is_ci_env() -> bool {
    is_ci(|k| std::env::var_os(k).map(|v| v.to_string_lossy().into_owned()))
}

#[cfg(test)]
#[path = "ci_tests.rs"]
mod tests;
