//! Persistent node worker that keeps `require("typescript")` loaded across many
//! decorator-metadata transforms. One cold `require("typescript")` + `transpileModule`
//! takes ~300 ms (TypeScript ~8 MiB of JS parsed and evaluated per call), so a spec
//! that imports hundreds of decorated models paid that cost per file (turbo-test#431:
//! `mexico-history-field-semantics.spec.ts` loads ~860 decorated modules → ~300 s cold).
//!
//! The worker speaks a tiny length-prefixed JSON protocol over stdio: `<decimal-len>\n`
//! then that many bytes of `{"file","moduleKind","src"}` → `{"code"}` or `{"error"}`.
//! On any I/O failure the worker is dropped and the caller falls back to a one-shot
//! node spawn (the pre-0.8.6 behaviour) so correctness never depends on reuse.
//!
//! One worker per (project root, `node_modules/typescript` dir) — in practice one per
//! run. Module loading is single-threaded in V8, so a single outer `Mutex` suffices.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const WORKER_SCRIPT: &str = include_str!("tsc_worker.js");

static SPAWN_COUNT: AtomicU64 = AtomicU64::new(0);

/// How many persistent workers this process has spawned since start. Test hook: a
/// run that calls `transform` for many files from the same project must stay at 1.
#[cfg(test)]
pub fn spawn_count() -> u64 {
    SPAWN_COUNT.load(Ordering::Relaxed)
}

/// Transform one TS file with the project's TypeScript compiler through the persistent
/// worker. Returns `None` on any worker / protocol failure so the caller can fall back
/// to the one-shot `node -e <inline script>` path.
pub fn transform(
    root: &Path,
    ts_dir: &Path,
    file: &Path,
    module_kind: &str,
    src: &str,
) -> Option<String> {
    let body = encode_request(file, module_kind, src);
    try_once(root, ts_dir, &body).or_else(|| try_once(root, ts_dir, &body))
}

fn try_once(root: &Path, ts_dir: &Path, body: &[u8]) -> Option<String> {
    let mut map = workers().lock().ok()?;
    let key = (root.to_path_buf(), ts_dir.to_path_buf());
    ensure_worker(&mut map, &key, root, ts_dir)?;
    let worker = map.get_mut(&key).expect("just ensured");
    match worker.call(body) {
        Ok(resp) => decode_response(&resp),
        Err(_) => {
            map.remove(&key);
            None
        }
    }
}

fn ensure_worker(
    map: &mut HashMap<(PathBuf, PathBuf), Worker>,
    key: &(PathBuf, PathBuf),
    root: &Path,
    ts_dir: &Path,
) -> Option<()> {
    if map.contains_key(key) {
        return Some(());
    }
    let worker = Worker::spawn(root, ts_dir)?;
    map.insert(key.clone(), worker);
    Some(())
}

fn workers() -> &'static Mutex<HashMap<(PathBuf, PathBuf), Worker>> {
    static WORKERS: OnceLock<Mutex<HashMap<(PathBuf, PathBuf), Worker>>> = OnceLock::new();
    WORKERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn encode_request(file: &Path, module_kind: &str, src: &str) -> Vec<u8> {
    // The TypeScript side decodes one JSON object; we hand-build it (no serde pull-in).
    let mut out = String::from("{\"file\":");
    json_quote(&mut out, &file.to_string_lossy());
    out.push_str(",\"moduleKind\":");
    json_quote(&mut out, module_kind);
    out.push_str(",\"src\":");
    json_quote(&mut out, src);
    out.push('}');
    out.into_bytes()
}

fn json_quote(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Pull `{"code":"..."}` out of the worker's response. Any other shape → None.
fn decode_response(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    let marker = "\"code\":\"";
    let start = text.find(marker)? + marker.len();
    let mut out = String::new();
    let mut iter = text[start..].chars();
    loop {
        match iter.next()? {
            '"' => return Some(out),
            '\\' => out.push(decode_escape(&mut iter)?),
            c => out.push(c),
        }
    }
}

fn decode_escape(iter: &mut std::str::Chars<'_>) -> Option<char> {
    match iter.next()? {
        '"' => Some('"'),
        '\\' => Some('\\'),
        '/' => Some('/'),
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'b' => Some('\u{08}'),
        'f' => Some('\u{0c}'),
        'u' => decode_unicode_escape(iter),
        _ => None,
    }
}

fn decode_unicode_escape(iter: &mut std::str::Chars<'_>) -> Option<char> {
    let mut hex = String::new();
    for _ in 0..4 {
        hex.push(iter.next()?);
    }
    let n = u32::from_str_radix(&hex, 16).ok()?;
    char::from_u32(n)
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn(root: &Path, ts_dir: &Path) -> Option<Self> {
        let mut child = Command::new("node")
            .current_dir(root)
            .args(["-e", WORKER_SCRIPT, &ts_dir.to_string_lossy()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        SPAWN_COUNT.fetch_add(1, Ordering::Relaxed);
        Some(Worker {
            child,
            stdin,
            stdout,
        })
    }

    fn call(&mut self, body: &[u8]) -> std::io::Result<Vec<u8>> {
        write_frame(&mut self.stdin, body)?;
        self.stdin.flush()?;
        read_frame(&mut self.stdout)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_frame(w: &mut ChildStdin, body: &[u8]) -> std::io::Result<()> {
    let header = format!("{}\n", body.len());
    w.write_all(header.as_bytes())?;
    w.write_all(body)
}

fn read_frame(r: &mut BufReader<ChildStdout>) -> std::io::Result<Vec<u8>> {
    let mut header = String::new();
    let read = r.read_line(&mut header)?;
    if read == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "worker closed stdout",
        ));
    }
    let n: usize = header
        .trim_end()
        .parse()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad frame header"))?;
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    Ok(body)
}

#[cfg(test)]
#[path = "tsc_worker_tests.rs"]
mod tests;
