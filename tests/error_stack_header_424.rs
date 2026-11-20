//! error.stack's header is formatted when the stack is first read (turbo-test#424): Node's
//! PrepareStackTraceCallback uses the error's name and message at that moment, so a message set
//! after super() (Nest's HttpException) appears in the stack; the frames are V8's own.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const CASES: &str = r#"
class LateMsg extends Error { constructor(m) { super(); this.message = m; this.name = 'LateMsg'; } }
// the shape of Nest's HttpException: message and name are assigned after super()
class HttpException extends Error {
 constructor(response, status) { super(); this.response = response; this.status = status; this.initMessage(); this.initName(); }
 initMessage() { this.message = typeof this.response === 'string' ? this.response : 'Http Exception'; }
 initName() { this.name = this.constructor.name; }
}
class NotFoundException extends HttpException { constructor(m) { super(m, 404); } }
test('the stack header reflects a message and name set after super()', () => {
 const e = new LateMsg('outbox pruned');
 expect(e.stack.split('\n')[0]).toBe('LateMsg: outbox pruned');
 const n = new NotFoundException('outbox pruned');
 expect(n.stack.split('\n')[0]).toBe('NotFoundException: outbox pruned');
 const f = new Error('a'); f.message = 'b';
 expect(f.stack.split('\n')[0]).toBe('Error: b');
});
test('captureStackTrace formats the header when the stack is first read', () => {
 const o = { message: 'obj' }; Error.captureStackTrace(o); o.message = 'later';
 expect(o.stack.split('\n')[0]).toBe('Error: later');
 const p = { name: 'Custom', message: 'm' }; Error.captureStackTrace(p); p.name = 'Renamed';
 expect(p.stack.split('\n')[0]).toBe('Renamed: m');
});
test('the first read fixes the stack, later changes do not rewrite it', () => {
 const e = new Error('first');
 expect(e.stack.split('\n')[0]).toBe('Error: first');
 e.message = 'second';
 expect(e.stack.split('\n')[0]).toBe('Error: first');
 e.stack = 'custom';
 expect(e.stack).toBe('custom');
 expect(Object.getOwnPropertyDescriptor(new Error('x'), 'stack').enumerable).toBe(false);
});
test('frames still point at this file', () => {
 const e = new LateMsg('where');
 const frames = e.stack.split('\n').slice(1);
 expect(frames.length).toBeGreaterThan(0);
 expect(frames[0]).toMatch(/^    at /);
 expect(frames[0]).toContain('case.test.js:');
});
test('the header follows Error.prototype.toString rules and getter errors propagate', () => {
 const e = new Error('x'); Object.defineProperty(e, 'message', { get() { throw new Error('boom'); } });
 let error; try { e.stack; } catch (err) { error = err; }
 expect(error && error.message).toBe('boom');
 const n = { name: 5, message: { toString() { return 'obj'; } } }; Error.captureStackTrace(n);
 expect(n.stack.split('\n')[0]).toBe('5: obj');
});
test('a user Error.prepareStackTrace formats the stack lazily', () => {
 const saved = Error.prepareStackTrace;
 Error.prepareStackTrace = (err, sites) => 'P:' + err.message + ':' + (sites.length > 0) + ':' + typeof sites[0].getFileName;
 try {
  const h = new Error('a'); h.message = 'b';
  expect(h.stack).toBe('P:b:true:function');
 } finally { Error.prepareStackTrace = saved; }
 expect(new Error('c').stack.split('\n')[0]).toBe('Error: c');
});
"#;
#[test]
fn error_stack_header_is_formatted_on_first_read() {
    let p = Project::new("error-stack-424");
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("case.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 6, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}

/// Node's own header rules (checked against `node` 26; Jest's environment formats these two
/// differently): an empty name leaves the message, an empty or missing message the name.
const NODE_RULES: &str = r#"
import { expect, test } from 'vitest';
test('empty parts of the header are left out as in Node', () => {
 const f = new Error('m'); f.name = '';
 expect(f.stack.split('\n')[0]).toBe('m');
 expect(new Error('').stack.split('\n')[0]).toBe('Error');
 const o = {}; Error.captureStackTrace(o);
 expect(o.stack.split('\n')[0]).toBe('Error');
});
"#;
#[test]
fn error_stack_header_drops_empty_parts_like_node() {
    let p = Project::new("error-stack-424-node");
    p.file("package.json", "{}");
    p.file("header.test.js", NODE_RULES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 1, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
