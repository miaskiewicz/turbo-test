//! Failed decorated dependency chains must not amplify loader retries (#357).
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
use std::process::Command;

#[test]
fn ordinary_child_failure_is_not_retried_as_metadata_at_every_ancestor() {
    let p = Project::new("loader-retry-amplification-357");
    p.with_esbuild().with_typescript();
    p.file(
        "package.json",
        r#"{"name":"loader-retry-amplification-357"}"#,
    );
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("tsconfig.json", r#"{"compilerOptions":{"module":"CommonJS","target":"ES2021","experimentalDecorators":true,"emitDecoratorMetadata":true}}"#);
    for depth in 0..8 {
        let next = if depth == 7 {
            "leaf".to_owned()
        } else {
            format!("chain{}", depth + 1)
        };
        p.file(&format!("chain{depth}.ts"), &format!("function marker(value:any) {{return value;}}\n@marker\nclass Chain{depth} {{}}\nrequire('./{next}');\n"));
    }
    p.file(
        "leaf.ts",
        "globalThis.__leafAttempts++; throw globalThis.__leafFailure;\n",
    );
    p.file("retry.test.ts", r#"
const assert = require('node:assert/strict');
test('ordinary error retains identity and explicit require retries exactly once', () => {
  globalThis.__leafAttempts = 0;
  globalThis.__leafFailure = Object.assign(new Error('ordinary leaf failure'), { cause: new Error('original cause') });
  const observations = [];
  for (let attempt = 0; attempt < 2; attempt++) {
    let caught;
    try { require('./chain0'); } catch (error) { caught = error; }
    assert.equal(caught, globalThis.__leafFailure);
    assert.equal(caught.cause.message, 'original cause');
    observations.push(globalThis.__leafAttempts);
  }
  assert.deepEqual(observations, [1, 2]);
});
test('the first of multiple caught child errors retains provenance', () => {
  globalThis.__firstAttempts = 0;
  globalThis.__secondAttempts = 0;
  globalThis.__firstError = new Error('first');
  globalThis.__secondError = new Error('second');
  for (let attempt = 1; attempt <= 2; attempt++) {
    let caught;
    try { require('./multiple'); } catch (error) { caught = error; }
    assert.equal(caught, globalThis.__firstError);
    assert.equal(globalThis.__firstAttempts, attempt);
    assert.equal(globalThis.__secondAttempts, attempt);
  }
});
test('caught equal primitive does not suppress local decorator metadata fallback', () => {
  const Own = require('./own').Own;
  assert.equal(new Own().value, 42);
});
"#);
    p.file("primitive.ts", "throw 'local metadata missing';\n");
    p.file(
        "own.ts",
        r#"
try { require('./primitive'); } catch (_) {}
Reflect.metadata = (key, type) => target => { target.__fieldType = type; };
function requireType(target, key) {
  if (target.__fieldType !== Number) throw 'local metadata missing';
}
export class Own {
  @requireType
  value: number = 42;
}
"#,
    );
    p.file(
        "first.ts",
        "globalThis.__firstAttempts++; throw globalThis.__firstError;\n",
    );
    p.file(
        "second.ts",
        "globalThis.__secondAttempts++; throw globalThis.__secondError;\n",
    );
    p.file(
        "multiple.ts",
        r#"
function marker(value) { return value; }
@marker
class Multiple {}
let saved;
try { require('./first'); } catch (error) { saved = error; }
try { require('./second'); } catch (_) {}
throw saved;
"#,
    );
    let oracle = r#"
const fs=require('node:fs'),path=require('node:path'),ts=require(process.argv[1]),root=process.argv[2],out=path.join(root,'oracle');
fs.mkdirSync(out);
for(const name of fs.readdirSync(root).filter(n=>n.endsWith('.ts'))) {
 const result=ts.transpileModule(fs.readFileSync(path.join(root,name),'utf8'),{fileName:name,compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2021,experimentalDecorators:true,emitDecoratorMetadata:true}});
 fs.writeFileSync(path.join(out,name.replace(/\.ts$/,'.js')),result.outputText);
}
global.test=(_name,fn)=>fn(); require(path.join(out,'retry.test.js'));
"#;
    let output = Command::new("node")
        .arg("-e")
        .arg(oracle)
        .arg(p.path("node_modules/typescript"))
        .arg(&p.dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Node control: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = turbo_test(
        &p.dir,
        &["--jobs", "1", "--reporter", "json", "retry.test.ts"],
        &[],
    );
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
