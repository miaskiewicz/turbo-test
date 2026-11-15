//! A hashbang is valid at a Node file boundary, including metadata fallback transforms.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
#[test]
fn metadata_fallback_preserves_commonjs_hashbang_semantics_and_script_origins() {
    let p = Project::new("cjs-hashbang-metadata");
    p.with_esbuild().with_typescript();
    p.file("package.json", r#"{"name":"cjs-hashbang-metadata"}"#);
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("tsconfig.json", r#"{"compilerOptions":{"module":"commonjs","target":"ES2022","allowJs":true,"experimentalDecorators":true,"emitDecoratorMetadata":true}}"#);
    p.file("hashbang-comment.js", "#!/usr/bin/env node\n// Convenience decorators (@Controller) are text, as in auth-matrix.\nexports.value = 42;\nexports.frame = function sourceFrame() { const old=Error.prepareStackTrace; try { Error.prepareStackTrace=(_,frames)=>frames; return new Error().stack[0].getFileName(); } finally { Error.prepareStackTrace=old; } };\n");
    p.file(
        "hashbang-only.js",
        "#!/usr/bin/env node\nexports.value = 42;\n",
    );
    p.file(
        "comment-only.js",
        "// Convenience decorators (@Controller) are text.\nexports.value = 42;\n",
    );
    p.file("hashbang.spec.cjs", r#"const testCase=globalThis.test||require('node:test'); const assert=require('node:assert/strict');
const path=require('node:path');
for(const name of ['hashbang-comment','hashbang-only','comment-only']) {
 testCase(name+' loads unchanged',()=>assert.equal(require('./'+name+'.js').value,42));
}
testCase('metadata module retains its real script origin',()=>{
 const filename=require('./hashbang-comment.js').frame();
 assert.equal(filename,path.join(__dirname,'hashbang-comment.js'));
});
"#);
    let oracle = std::process::Command::new("node")
        .args(["--test", "hashbang.spec.cjs"])
        .current_dir(&p.dir)
        .output()
        .unwrap();
    assert!(
        oracle.status.success(),
        "{}",
        String::from_utf8_lossy(&oracle.stderr)
    );
    for _ in 0..2 {
        let run = turbo_test(
            &p.dir,
            &["--jobs", "1", "--reporter", "json", "hashbang.spec.cjs"],
            &[],
        );
        let report = json_report(&run);
        assert_eq!(report["numPassedTests"], 4, "{}", run.all());
        assert_eq!(report["numFailedTests"], 0, "{}", run.all());
    }
}
