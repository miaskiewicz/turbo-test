#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn buffer_module_limits_match_the_linked_v8_engine_and_node_descriptors() {
    let p = Project::new("buffer-constants-396");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"buffer-constants-396"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    let mut source = include_str!("data/compat-084/buffer-constants.cjs").to_owned();
    source.push_str(&format!("\ntest('limits derive from linked V8',()=>{{assert.equal(buffer.constants.MAX_LENGTH,{});assert.equal(buffer.constants.MAX_STRING_LENGTH,{});}});",v8::Uint8Array::MAX_LENGTH,v8::String::MAX_LENGTH));
    p.file("constants.test.cjs", &source);
    let run = turbo_test(&p.dir, &["--jobs", "1", "--reporter", "json"], &[]);
    let result = json_report(&run);
    assert_eq!(result["numTotalTests"], 4, "{}", run.all());
    assert_eq!(result["numPassedTests"], 4, "{}", run.all());
    assert_eq!(result["numFailedTests"], 0, "{}", run.all());
}
