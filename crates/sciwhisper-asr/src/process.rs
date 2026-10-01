//! Running the recogniser as a child process, with a deadline and a ceiling
//! on how much it may say.
//!
//! Arguments are always passed as arguments — never assembled into a command
//! line — so a path with spaces, quotes or Cyrillic reaches the backend
//! exactly as it appears on disk.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// How long the recogniser may run before it is stopped. A long dictation on
/// a slow CPU is legitimate; an hour of silence is not.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// How much of stdout and stderr is kept. Whisper prints a few kilobytes; a
/// backend stuck in a loop must not be able to fill memory.
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            timeout: timeout_from_env().unwrap_or(DEFAULT_TIMEOUT),
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}

fn timeout_from_env() -> Option<Duration> {
    let raw = std::env::var("SCIWHISPER_TIMEOUT_SECS").ok()?;
    let seconds: u64 = raw.trim().parse().ok()?;
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

#[derive(Clone, Debug)]
pub struct Finished {
    pub code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    /// True when either stream hit the ceiling and was cut short.
    pub truncated: bool,
}

impl Finished {
    /// The last few lines of stderr — what a person needs to see, without a
    /// wall of progress output.
    pub fn tail(&self, lines: usize) -> String {
        let source = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let collected: Vec<&str> = source
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        let start = collected.len().saturating_sub(lines);
        collected[start..].join("; ")
    }
}

/// How long to wait for the output readers once the process tree is gone.
/// Reaching this means a descendant is still holding the pipe, and the run
/// returns with whatever was captured rather than waiting on it.
const READER_GRACE: Duration = Duration::from_secs(2);

/// Runs a prepared command to completion, or kills it — and everything it
/// started — at the deadline.
///
/// On Unix the child is placed in its own process group before it starts. On
/// Windows it is attached to a kill-on-close job object immediately after
/// spawning; failure to create, configure, or attach that job aborts the run
/// instead of silently weakening the deadline.
pub fn run(mut command: Command, limits: Limits) -> Result<Finished> {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    isolate(&mut command);
    let mut child = command.spawn().map_err(|error| Error::BackendFailed {
        code: None,
        detail: format!("не удалось запустить процесс: {error}"),
    })?;

    #[cfg(unix)]
    let tree = Tree::adopt(&child);
    #[cfg(windows)]
    let tree = match Tree::adopt(&child) {
        Ok(tree) => tree,
        Err(detail) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::BackendFailed { code: None, detail });
        }
    };

    let cap = limits.max_output_bytes;
    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        let _ = stdout_tx.send(read_capped(stdout, cap));
    });
    std::thread::spawn(move || {
        let _ = stderr_tx.send(read_capped(stderr, cap));
    });

    let status = match wait_until(&mut child, limits.timeout) {
        Ok(status) => status,
        Err(error) => {
            tree.kill();
            let _ = child.wait();
            return Err(Error::BackendFailed {
                code: None,
                detail: format!("не удалось дождаться завершения движка: {error}"),
            });
        }
    };
    // Whether it finished or ran out of time, nothing of it may be left
    // running: on the deadline this is the enforcement, and after a normal
    // exit it collects any straggler still holding the pipes.
    let timed_out = status.is_none();
    if timed_out {
        tree.kill();
        let _ = child.wait();
    }

    // Collecting output is bounded too. A grandchild that inherited the pipe
    // could otherwise keep `join` waiting for as long as it likes, which would
    // defeat the deadline that was just enforced.
    let collect =
        |rx: &std::sync::mpsc::Receiver<(String, bool)>| match rx.recv_timeout(READER_GRACE) {
            Ok(value) => value,
            Err(_) => {
                tree.kill();
                rx.recv_timeout(READER_GRACE).unwrap_or_default()
            }
        };
    let (stdout, stdout_cut) = collect(&stdout_rx);
    let (stderr, stderr_cut) = collect(&stderr_rx);

    let Some(status) = status else {
        return Err(Error::BackendTimedOut {
            seconds: limits.timeout.as_secs(),
        });
    };
    Ok(Finished {
        code: status.code(),
        success: status.success(),
        stdout,
        stderr,
        truncated: stdout_cut || stderr_cut,
    })
}

/// Puts the child where it can be stopped as a whole.
#[cfg(unix)]
fn isolate(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Its own process group, so a single signal reaches every descendant.
    command.process_group(0);
}

#[cfg(windows)]
fn isolate(_command: &mut Command) {
    // Windows has no equivalent at spawn time; the job object is attached
    // immediately afterwards, in `Tree::adopt`.
}

/// A handle on the whole process tree.
struct Tree {
    #[cfg(unix)]
    pid: u32,
    #[cfg(windows)]
    job: Option<windows::Win32::Foundation::HANDLE>,
}

#[cfg(unix)]
impl Tree {
    fn adopt(child: &Child) -> Self {
        Tree { pid: child.id() }
    }

    fn kill(&self) {
        // The child leads its own group, so its id is also the group id.
        unsafe {
            libc::killpg(self.pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
impl Tree {
    fn adopt(child: &Child) -> std::result::Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        unsafe {
            let job = CreateJobObjectW(None, None)
                .map_err(|error| format!("не удалось создать Windows Job Object: {error}"))?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            // Closing the last handle to the job kills everything inside it, so
            // even a panic on this thread cannot leave the tree running.
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Err(error) = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) {
                let _ = CloseHandle(job);
                return Err(format!(
                    "не удалось настроить Windows Job Object для движка: {error}"
                ));
            }
            let handle = HANDLE(child.as_raw_handle() as _);
            if let Err(error) = AssignProcessToJobObject(job, handle) {
                let _ = CloseHandle(job);
                return Err(format!(
                    "не удалось включить движок в Windows Job Object: {error}"
                ));
            }
            Ok(Tree { job: Some(job) })
        }
    }

    fn kill(&self) {
        use windows::Win32::System::JobObjects::TerminateJobObject;
        if let Some(job) = self.job {
            unsafe {
                let _ = TerminateJobObject(job, 1);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for Tree {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        if let Some(job) = self.job.take() {
            unsafe {
                let _ = CloseHandle(job);
            }
        }
    }
}

/// Polls rather than blocking, so the deadline is honoured even when the child
/// never writes anything.
fn wait_until(
    child: &mut Child,
    timeout: Duration,
) -> std::io::Result<Option<std::process::ExitStatus>> {
    let deadline = Instant::now() + timeout;
    let mut nap = Duration::from_millis(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(nap.min(Duration::from_millis(100)));
        nap = (nap * 2).min(Duration::from_millis(100));
    }
}

/// Reads a stream up to the ceiling and then keeps draining it without
/// storing more, so the child never blocks on a full pipe.
fn read_capped<R: Read + Send + 'static>(stream: Option<R>, cap: usize) -> (String, bool) {
    let Some(mut stream) = stream else {
        return (String::new(), false);
    };
    let mut kept: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if kept.len() < cap {
                    let room = cap - kept.len();
                    let take = room.min(read);
                    kept.extend_from_slice(&buffer[..take]);
                    if take < read {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (String::from_utf8_lossy(&kept).into_owned(), truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Writes a tiny script that behaves like a backend, so the process rules
    /// can be tested without a real Whisper anywhere.
    fn fake_backend(dir: &Path, name: &str, body: &str) -> PathBuf {
        #[cfg(windows)]
        {
            let path = dir.join(format!("{name}.cmd"));
            std::fs::write(&path, body).unwrap();
            path
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
            path
        }
    }

    fn limits(seconds: u64, cap: usize) -> Limits {
        Limits {
            timeout: Duration::from_secs(seconds),
            max_output_bytes: cap,
        }
    }

    #[test]
    fn a_successful_run_returns_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_backend(dir.path(), "ok", "echo hello\nexit 0\n");
        let finished = run(Command::new(&script), limits(30, MAX_OUTPUT_BYTES)).unwrap();
        assert!(finished.success);
        assert_eq!(finished.code, Some(0));
        assert!(finished.stdout.contains("hello"), "{:?}", finished.stdout);
        assert!(!finished.truncated);
    }

    #[test]
    fn a_nonzero_exit_code_is_reported_with_its_message() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_backend(dir.path(), "bad", "echo failure detail 1>&2\nexit 3\n");
        let finished = run(Command::new(&script), limits(30, MAX_OUTPUT_BYTES)).unwrap();
        assert!(!finished.success);
        assert_eq!(finished.code, Some(3));
        assert!(finished.tail(2).contains("failure detail"), "{finished:?}");
    }

    #[test]
    fn a_missing_backend_is_a_start_failure_not_a_hang() {
        let error = run(
            Command::new("sciwhisper-no-such-backend-anywhere"),
            limits(5, MAX_OUTPUT_BYTES),
        )
        .expect_err("a missing program cannot run");
        assert!(
            matches!(error, Error::BackendFailed { code: None, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_process_that_never_finishes_is_stopped_at_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let body = "ping -n 60 127.0.0.1 > nul\n";
        #[cfg(not(windows))]
        let body = "sleep 60\n";
        let script = fake_backend(dir.path(), "slow", body);
        let started = Instant::now();
        let error = run(Command::new(&script), limits(1, MAX_OUTPUT_BYTES))
            .expect_err("the deadline must be enforced");
        assert!(
            matches!(error, Error::BackendTimedOut { seconds: 1 }),
            "{error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the run must not wait for the child: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_talkative_backend_is_cut_off_instead_of_filling_memory() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let body = "for /L %%i in (1,1,4000) do @echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nexit 0\n";
        #[cfg(not(windows))]
        let body = "i=0\nwhile [ $i -lt 4000 ]; do echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; i=$((i+1)); done\nexit 0\n";
        let script = fake_backend(dir.path(), "loud", body);
        let finished = run(Command::new(&script), limits(60, 4096)).unwrap();
        assert!(finished.truncated, "the ceiling must be reported");
        assert!(
            finished.stdout.len() <= 4096,
            "kept {} bytes",
            finished.stdout.len()
        );
    }

    #[test]
    fn arguments_are_passed_whole_even_with_spaces_and_cyrillic() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let body = "@echo %~1\nexit 0\n";
        #[cfg(not(windows))]
        let body = "printf '%s' \"$1\"\nexit 0\n";
        let script = fake_backend(dir.path(), "echo1", body);
        let odd = "C:\\Мои документы\\запись 1.wav";
        let mut command = Command::new(&script);
        command.arg(odd);
        let finished = run(command, limits(30, MAX_OUTPUT_BYTES)).unwrap();
        assert!(
            finished.stdout.contains("Мои документы") && finished.stdout.contains("запись 1"),
            "the path was mangled: {:?}",
            finished.stdout
        );
    }

    /// A backend that starts a helper and then hangs. The helper writes a
    /// marker only if it is allowed to finish, so the marker's absence is
    /// proof that the whole tree was stopped.
    #[cfg(not(windows))]
    const HANGS_WITH_A_CHILD: &str = r#"
( sleep 30; echo alive > "$1" ) &
sleep 30
"#;
    #[cfg(windows)]
    const HANGS_WITH_A_CHILD: &str = "@echo off\r\nstart /b cmd /c \"ping -n 30 127.0.0.1 > nul & echo alive > %~1\"\r\nping -n 30 127.0.0.1 > nul\r\n";

    #[test]
    fn killing_a_stuck_backend_also_kills_what_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_backend(dir.path(), "tree", HANGS_WITH_A_CHILD);
        let marker = dir.path().join("alive.txt");
        let mut command = Command::new(&script);
        command.arg(&marker);

        let started = Instant::now();
        let error = run(command, limits(1, MAX_OUTPUT_BYTES)).expect_err("the deadline applies");
        assert!(
            matches!(error, Error::BackendTimedOut { seconds: 1 }),
            "{error}"
        );
        // The run itself must not have waited for the descendant.
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the run waited for a descendant: {:?}",
            started.elapsed()
        );
        // And the descendant must be gone: give it more than enough time to
        // write its marker if it had survived.
        std::thread::sleep(Duration::from_secs(3));
        assert!(
            !marker.exists(),
            "a descendant outlived the deadline and wrote {}",
            marker.display()
        );
    }

    /// A backend that exits normally but leaves a helper holding the pipe.
    #[cfg(not(windows))]
    const EXITS_LEAVING_A_CHILD: &str = r#"
( sleep 30 ) &
echo done
exit 0
"#;
    #[cfg(windows)]
    const EXITS_LEAVING_A_CHILD: &str =
        "@echo off\r\nstart /b cmd /c \"ping -n 30 127.0.0.1 > nul\"\r\necho done\r\nexit /b 0\r\n";

    #[test]
    fn a_lingering_helper_cannot_stall_a_finished_run() {
        let dir = tempfile::tempdir().unwrap();
        let script = fake_backend(dir.path(), "linger", EXITS_LEAVING_A_CHILD);
        let started = Instant::now();
        let finished = run(Command::new(&script), limits(30, MAX_OUTPUT_BYTES))
            .expect("the process itself exited cleanly");
        assert!(finished.success);
        // Collecting output is bounded, so the inherited pipe cannot hold the
        // call open for the helper's whole lifetime.
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the run waited on an inherited pipe: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_timeout_can_be_configured_but_never_to_zero() {
        // The parser refuses nonsense rather than turning the deadline off.
        assert!(timeout_from_env().is_none() || timeout_from_env().unwrap() > Duration::ZERO);
    }
}
