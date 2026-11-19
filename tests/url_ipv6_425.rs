//! WHATWG URL IPv6 host parsing and serialization (turbo-test#425): Node compresses and
//! lower-cases an IPv6 host, rewrites an embedded IPv4 tail as hex and rejects an invalid one,
//! so an SSRF guard comparing `new URL(...).hostname` with `[::1]` sees the canonical form.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const CASES: &str = r#"
test('IPv6 hosts serialize in WHATWG compressed form', () => {
 const cases=[['0:0:0:0:0:0:0:1','[::1]'],['::ffff:127.0.0.1','[::ffff:7f00:1]'],['0:0:0:0:0:ffff:169.254.169.254','[::ffff:a9fe:a9fe]'],
  ['64:ff9b::169.254.169.254','[64:ff9b::a9fe:a9fe]'],['1:0:0:2:0:0:0:3','[1:0:0:2::3]'],['1:0:0:2:0:0:3:4','[1::2:0:0:3:4]'],
  ['ABCD::','[abcd::]'],['::','[::]'],['1:2:3:4:5:6:7::','[1:2:3:4:5:6:7:0]'],['0001:0db8::','[1:db8::]'],['1:2:3:4:5:6:1.2.3.4','[1:2:3:4:5:6:102:304]']];
 for(const [ip,want] of cases){const u=new URL(`http://[${ip}]/`);expect([ip,u.hostname]).toEqual([ip,want]);expect(u.href).toBe(`http://${want}/`);}
 const full=new URL('http://u:p@[0::1]:99/x?q#h');
 expect([full.host,full.href,full.origin]).toEqual(['[::1]:99','http://u:p@[::1]:99/x?q#h','http://[::1]:99']);
 expect(new URL('http://[64:ff9b:0:0:0:0:a9fe:a9fe]:8080/').host).toBe('[64:ff9b::a9fe:a9fe]:8080');
 expect(new URL('foo://[0:0::1]/').host).toBe('[::1]');
 expect(new URL('file://[0:0::1]/p').href).toBe('file://[::1]/p');
});
test('invalid IPv6 hosts are rejected like Node', () => {
 for(const ip of ['1::2::3','1:2:3:4:5:6:7:8:9','::1.2.3','::1.2.3.256','::01.2.3.4',':1','1:','12345::','g::','::1%25eth0','1:2:3:4:5:6:7:1.2.3.4','','::1.2.3.4x']){
  let error;try{new URL(`http://[${ip}]/`)}catch(e){error=e}
  expect([ip,error&&error.name]).toEqual([ip,'TypeError']);
 }
 for(const u of ['foo://[zz]/','foo://[::1]x/','http://[::1]x/']){let error;try{new URL(u)}catch(e){error=e}expect([u,error&&error.name]).toEqual([u,'TypeError']);}
});
test('host setters canonicalize IPv6 and ignore an invalid one', () => {
 const u=new URL('http://a/');
 u.hostname='[0:0::1]';expect(u.href).toBe('http://[::1]/');
 u.host='[1:0::0]:81';expect(u.href).toBe('http://[1::]:81/');
 u.hostname='[zz]';expect(u.href).toBe('http://[1::]:81/');
});
"#;
#[test]
fn url_ipv6_hosts_match_node_serialization() {
    let p = Project::new("url-ipv6-425");
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("url.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 3, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
