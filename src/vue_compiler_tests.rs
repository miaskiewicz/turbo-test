use super::*;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::process::Child;

struct CompilerChild(Child);
impl Drop for CompilerChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn compiler_child(script: &str, piped: bool) -> CompilerChild {
    let child = Command::new("/bin/sh")
        .args(["-c", script])
        .env_clear()
        .stdin(if piped { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    CompilerChild(child)
}

fn assert_already_reaped(child: &Child) {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let status = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    assert_eq!(status, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[test]
fn missing_compiler_input_pipe_terminates_and_reaps_the_owned_child() {
    let mut child = compiler_child("exec /bin/sleep 60", false);
    let error = request_or_terminate(&mut child.0, b"compile input").unwrap_err();
    assert_eq!(error, "compiler stdin unavailable");
    assert_already_reaped(&child.0);
    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
}

#[test]
fn broken_compiler_input_pipe_preserves_error_and_reaps_the_owned_child() {
    let mut child = compiler_child("exec 0<&-; printf ready; exec /bin/sleep 60", true);
    let mut ready = [0; 5];
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_exact(&mut ready)
        .unwrap();
    assert_eq!(&ready, b"ready");
    // Exceed pipe capacity so inherited read aliases cannot accept the entire request.
    let source = vec![b'x'; 1_048_576];
    let error = request_or_terminate(&mut child.0, &source).unwrap_err();
    assert_eq!(
        error,
        std::io::Error::from_raw_os_error(libc::EPIPE).to_string()
    );
    assert_already_reaped(&child.0);
    assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
}

#[test]
fn compiler_input_request_writes_exact_bytes_and_closes_its_pipe() {
    let mut child = compiler_child("exec /bin/cat", true);
    let source = "<template>héllo</template>".as_bytes();
    request_or_terminate(&mut child.0, source).unwrap();
    let mut output = Vec::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, source);
    assert!(child.0.wait().unwrap().success());
}

#[test]
fn compiled_cache_reclaims_old_results_at_its_limit_and_keeps_the_new_result() {
    COMPILED.with(|cache| cache.borrow_mut().clear());
    for index in 0..256_u64 {
        cache_compiled(
            (PathBuf::from(format!("cached-{index}.vue")), index),
            "export default 1;",
        );
    }
    COMPILED.with(|cache| assert_eq!(cache.borrow().len(), 256));
    let fresh = (PathBuf::from("fresh.vue"), 999);
    cache_compiled(fresh.clone(), "export default 42;");
    COMPILED.with(|cache| {
        let cache = cache.borrow();
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.get(&fresh).map(String::as_str),
            Some("export default 42;")
        );
    });
    COMPILED.with(|cache| cache.borrow_mut().clear());
}
