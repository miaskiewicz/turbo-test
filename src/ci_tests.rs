//! Unit tests for ci.rs: the provider table as data.

use super::*;

#[test]
fn provider_table_is_std_env_order() {
    let t = providers();
    assert_eq!(t.len(), 58);
    assert_eq!(
        t[0],
        Provider {
            provider: "appveyor".into(),
            env: "APPVEYOR".into(),
            ci: true
        }
    );
    assert_eq!(t[57].env, "EO_PAGES_CI");
    // the rows that do not count as CI, each after the provider's CI row where it has one
    let not_ci: Vec<&str> = t.iter().filter(|p| !p.ci).map(|p| p.env.as_str()).collect();
    assert_eq!(
        not_ci,
        [
            "NETLIFY_LOCAL",
            "VERCEL",
            "VERCEL_ENV",
            "CODESANDBOX_SSE",
            "CODESANDBOX_HOST"
        ]
    );
    let pos = |k: &str| t.iter().position(|p| p.env == k).unwrap();
    assert!(pos("NETLIFY") < pos("NETLIFY_LOCAL"));
    assert!(pos("NOW_BUILDER") < pos("VERCEL"));
}

#[test]
fn env_vars_are_ci_then_every_row() {
    let vars: Vec<&str> = env_vars().collect();
    assert_eq!(vars.len(), 59);
    assert_eq!(vars[..3], ["CI", "APPVEYOR", "AWS_APP_ID"]);
    assert!(vars.contains(&"bamboo_planKey"));
}
