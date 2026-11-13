//! Actual loopback HTTP bytes and resource lifecycle (#321), shared with Node controls.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};

#[test]
fn loopback_http_server_client_and_lifecycle_batch() {
    let p = Project::new("http-084");
    p.with_esbuild();
    p.file("package.json", r#"{"name":"http-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p.file("http.test.cjs", include_str!("data/compat-084/http.cjs"));
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 8, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
struct NodeOracle(Child);
impl Drop for NodeOracle {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn node_server() -> (NodeOracle, u16) {
    let source = r#"
const http = require('node:http');
const server = http.createServer((request, response) => {
  let body = ''; request.setEncoding('utf8'); request.on('data', chunk => body += chunk);
  request.on('end', () => {
    response.writeHead(201, {'X-Oracle':'node'}); response.write('from-node:');
    setTimeout(() => { response.end(body); server.close(); }, 5);
  });
});
server.listen(0, '127.0.0.1', () => process.stdout.write(String(server.address().port) + '\n'));
"#;
    let mut child = Command::new("node")
        .arg("-e")
        .arg(source)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port = line
        .trim()
        .parse()
        .expect("Node HTTP oracle did not report a real port");
    (NodeOracle(child), port)
}
fn http_project(name: &str) -> Project {
    let p = Project::new(name);
    p.with_esbuild();
    p.file("package.json", r#"{"name":"http-wire-084"}"#);
    p.file(
        "jest.config.js",
        "module.exports = { testEnvironment: 'node' };\n",
    );
    p
}
fn one_pass(run: &common::Run) {
    let report = json_report(run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
#[test]
fn native_client_speaks_real_node_http_wire_protocol() {
    let (_oracle, port) = node_server();
    let p = http_project("http-node-server-084");
    p.file("wire.test.cjs", &format!(r#"
const assert = require('node:assert/strict'); const http = require('node:http');
test('native client sends valid chunked bytes to real Node server', async () => {{
  const body = await new Promise((resolve, reject) => {{
    const request = http.request({{hostname:'127.0.0.1',port:{port},method:'POST',path:'/oracle'}}, response => {{
      assert.equal(response.statusCode,201); assert.equal(response.headers['x-oracle'],'node');
      let data=''; response.setEncoding('utf8'); response.on('data',chunk => data+=chunk);
      response.on('end',()=>resolve(data)); response.on('error',reject);
    }}); request.on('error',reject); request.end('native-http');
  }}); assert.equal(body,'from-node:native-http');
}});
"#));
    one_pass(&turbo_test(&p.dir, &["--reporter", "json"], &[]));
}
fn wait_port(path: &std::path::Path) -> u16 {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(port) = text.parse() {
                return port;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("native HTTP server did not report a port");
}
fn node_client(path: &std::path::Path) -> Output {
    let port = wait_port(path);
    let source = r#"
const assert=require('node:assert/strict'); const http=require('node:http');
const request=http.request({hostname:'127.0.0.1',port:Number(process.argv[1]),path:'/from-node',method:'POST',headers:{'Content-Length':11}},response=>{
  assert.equal(response.statusCode,202); assert.equal(response.headers['x-native'],'yes');
  let body='';response.setEncoding('utf8');response.on('data',chunk=>body+=chunk);
  response.on('end',()=>assert.equal(body,'from-turbo:oracle-body'));
});request.on('error',error=>{console.error(error);process.exitCode=1;});request.end('oracle-body');
"#;
    Command::new("node")
        .arg("-e")
        .arg(source)
        .arg(port.to_string())
        .output()
        .unwrap()
}
#[test]
fn native_server_speaks_real_node_http_wire_protocol() {
    let p = http_project("http-node-client-084");
    let port_file = p.path("port.txt");
    p.file("wire.test.cjs", &format!(r#"
const assert=require('node:assert/strict');const http=require('node:http');const fs=require('node:fs');
test('real Node client exchanges bytes with native server',async()=>{{
  await new Promise((resolve,reject)=>{{
    const server=http.createServer((request,response)=>{{
      assert.equal(request.url,'/from-node'); let body='';request.setEncoding('utf8');
      request.on('data',chunk=>body+=chunk);request.on('end',()=>{{
        response.writeHead(202,{{'X-Native':'yes'}});response.end('from-turbo:'+body);server.close(resolve);
      }});
    }});server.on('error',reject);server.listen(0,'127.0.0.1',()=>fs.writeFileSync({port_file:?},String(server.address().port)));
  }});
}});
"#));
    let client = std::thread::spawn(move || node_client(&port_file));
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let output = client.join().unwrap();
    assert!(
        output.status.success(),
        "Node client: {}\nNative runner: {}",
        String::from_utf8_lossy(&output.stderr),
        run.all()
    );
    one_pass(&run);
}
