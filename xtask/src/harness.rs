//! Subprocess, temp-directory and assertion helpers shared by every xtask gate.
//!
//! Each child runs in its own process group so a timeout reaps the debugger *and*
//! its inferior; the previous Python runners each carried their own copy of this.

use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

pub type Result<T = ()> = std::result::Result<T, Failure>;

/// A gate failure. Carries the child output so the caller never has to re-run.
#[derive(Debug)]
pub struct Failure(pub String);

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `panic!`-free assertion that reports like the Python `assert (..., output)` tuples.
#[macro_export]
macro_rules! ensure {
    ($condition:expr, $($arg:tt)*) => {
        if !$condition {
            return Err($crate::harness::Failure(format!($($arg)*)));
        }
    };
}

pub fn root() -> PathBuf {
    // xtask/src/harness.rs -> xtask -> repo root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent")
        .to_path_buf()
}

/// A child process specification. Mirrors the keyword arguments the Python
/// runners had drifted apart on (`success` / `expect_success` / `ok`).
pub struct Run {
    command: Command,
    expect_success: bool,
    marker: Option<String>,
    timeout: Duration,
}

impl Run {
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        let mut command = Command::new(program);
        command.current_dir(root());
        // `cargo xtask` injects its own build context. Children must see the same
        // environment the previous `python3 tests/...` entry points gave them, or a
        // temp project would silently inherit this workspace's toolchain and target dir.
        for key in [
            "CARGO",
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_TARGET_DIR",
            "CARGO_PKG_NAME",
            "CARGO_PKG_VERSION",
            "CARGO_CRATE_NAME",
            "RUSTC",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
            "RUSTDOC",
            "RUSTUP_TOOLCHAIN",
        ] {
            command.env_remove(key);
        }
        Self {
            command,
            expect_success: true,
            marker: None,
            timeout: Duration::from_secs(120),
        }
    }

    pub fn arg<S: AsRef<OsStr>>(mut self, value: S) -> Self {
        self.command.arg(value);
        self
    }

    pub fn args<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(values);
        self
    }

    pub fn cwd<P: AsRef<Path>>(mut self, directory: P) -> Self {
        self.command.current_dir(directory);
        self
    }

    pub fn env<K: AsRef<OsStr>, V: AsRef<OsStr>>(mut self, key: K, value: V) -> Self {
        self.command.env(key, value);
        self
    }

    /// Expect a non-zero exit instead of success.
    pub fn expect_failure(mut self) -> Self {
        self.expect_success = false;
        self
    }

    /// Require this string in the combined output.
    pub fn marker(mut self, marker: &str) -> Self {
        self.marker = Some(marker.to_owned());
        self
    }

    pub fn timeout(mut self, seconds: u64) -> Self {
        self.timeout = Duration::from_secs(seconds);
        self
    }

    /// Run to completion, returning the combined stdout+stderr.
    pub fn output(mut self) -> Result<String> {
        let described = describe(&self.command);
        self.command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = self
            .command
            .spawn()
            .map_err(|error| Failure(format!("spawn {described}: {error}")))?;
        let group = child.id();
        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");

        // Drain both pipes from threads so a chatty child cannot fill a pipe
        // buffer and deadlock before we ever reach the timeout.
        let (out_tx, out_rx) = mpsc::channel();
        let err_tx = out_tx.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stdout.read_to_string(&mut text);
            let _ = out_tx.send((0, text));
        });
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            let _ = err_tx.send((1, text));
        });

        let (status_tx, status_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = status_tx.send(child.wait());
        });

        let status = match status_rx.recv_timeout(self.timeout) {
            Ok(status) => status.map_err(|error| Failure(format!("wait {described}: {error}")))?,
            Err(_) => {
                kill_group(group);
                let text = collect(&out_rx);
                return Err(Failure(format!("TIMEOUT {described}\n{text}")));
            }
        };

        let text = collect(&out_rx);
        print!("{text}");
        ensure!(
            status.success() == self.expect_success,
            "{described} exited with {status}, expected {}\n{text}",
            if self.expect_success {
                "success"
            } else {
                "failure"
            }
        );
        if let Some(marker) = &self.marker {
            ensure!(
                text.contains(marker.as_str()),
                "{described} did not print {marker}\n{text}"
            );
        }
        Ok(text)
    }

    /// Run to completion, discarding the output on success.
    pub fn check(self) -> Result {
        self.output().map(|_| ())
    }
}

/// Collect both pipe readers, ordering stdout before stderr for stable reports.
fn collect(receiver: &mpsc::Receiver<(u8, String)>) -> String {
    let mut parts = [String::new(), String::new()];
    for _ in 0..2 {
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok((stream, text)) => parts[stream as usize] = text,
            Err(_) => break,
        }
    }
    parts.concat()
}

fn kill_group(group: u32) {
    // SAFETY: killpg with a process group this xtask created via process_group(0).
    unsafe { libc::killpg(group as libc::pid_t, libc::SIGKILL) };
}

fn describe(command: &Command) -> String {
    let mut text = command.get_program().to_string_lossy().into_owned();
    for argument in command.get_args() {
        text.push(' ');
        text.push_str(&argument.to_string_lossy());
    }
    text
}

/// A self-deleting scratch directory. Replaces Python's `TemporaryDirectory`.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(prefix: &str) -> Result<Self> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)
            .map_err(|error| Failure(format!("create {}: {error}", path.display())))?;
        // cargo canonicalizes path dependencies; resolve now so generated manifests
        // match what cargo reports back (macOS /var -> /private/var).
        let path = path
            .canonicalize()
            .map_err(|error| Failure(format!("canonicalize {}: {error}", path.display())))?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write `contents` to `path`, creating parent directories as needed.
pub fn write<P: AsRef<Path>>(path: P, contents: &str) -> Result {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| Failure(format!("mkdir {}: {error}", parent.display())))?;
    }
    std::fs::write(path, contents)
        .map_err(|error| Failure(format!("write {}: {error}", path.display())))
}

/// Whether `haystack` contains `needle` delimited by non-identifier characters.
/// Replaces the `\b<type>\b` regexes in the visibility matrix.
pub fn contains_word(haystack: &str, needle: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut offset = 0;
    while let Some(found) = haystack[offset..].find(needle) {
        let start = offset + found;
        let end = start + needle.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident(c));
        let after_ok = haystack[end..].chars().next().is_none_or(|c| !is_ident(c));
        if before_ok && after_ok {
            return true;
        }
        offset = start + 1;
    }
    false
}
