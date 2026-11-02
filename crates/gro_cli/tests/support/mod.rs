//! What the end-to-end tests share: the fixtures, the `fgro` command, a
//! spawn bounded in time and memory, a child killed on drop, and scratch
//! directories removed on drop.

#![allow(dead_code, reason = "each test binary uses only part of this module")]
// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]
#![expect(clippy::panic, reason = "a hung child fails its test by panicking")]

use std::io::{BufRead as _, BufReader, Read, Write as _};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The binary under test.
pub const FGRO: &str = env!("CARGO_BIN_EXE_fgro");

/// How long a child may run before it's killed and its test fails, generous
/// for a debug build on a loaded machine.
pub const DEADLINE: Duration = Duration::from_secs(60);

/// The most resident memory a child may reach, in KiB, before it's killed
/// and its test fails: 1.5 GiB, twice the most an eval heap test's child
/// reaches, so a cap that stops holding fails its test rather than taking
/// the machine's memory.
pub const MAX_RSS_KIB: u64 = 3 << 19;

/// The variable naming the defaults a priced build turns off,
/// `gro_runtime::prices`'s, read only with its feature.
pub const PRICES_OFF_ENV: &str = "GRO_PRICES_OFF";

/// The variables `fgro run` decides color from, all of them, so a test sets
/// what it needs and the harness's own environment (Claude Code's
/// `CLAUDECODE`, a CI's `FORCE_COLOR`) doesn't leak in; eval reads none.
pub const COLOR_VARS: [&str; 5] = [
    "FORCE_COLOR",
    "NO_COLOR",
    "NODE_DISABLE_COLORS",
    "CLAUDECODE",
    "TERM",
];

/// The fixtures directory `tests/fixtures/<kind>`.
pub fn fixtures(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(kind)
}

/// The `kind` fixtures directory as a `file:` URL prefix, as error positions
/// name it, percent-encoded as the runtime's URLs are.
pub fn fixtures_url(kind: &str) -> String {
    url::Url::from_file_path(fixtures(kind).canonicalize().unwrap())
        .unwrap()
        .into()
}

/// `fgro` with `args`, as [`fgro_command_in`] sets it up.
pub fn fgro_command(args: &[&str]) -> Command {
    fgro_command_in(Command::new(FGRO), args)
}

/// `fgro` with `args`, run by `wrapper` (`prlimit` with its limits, a shell
/// with its script), which gets `fgro`'s path and `args` after its own
/// arguments, as [`fgro_command_in`] sets it up.
pub fn wrapped_fgro_command(mut wrapper: Command, args: &[&str]) -> Command {
    wrapper.arg(FGRO);
    fgro_command_in(wrapper, args)
}

/// Adds `args` to `command`, clears every color variable, and sets `TZ=UTC`
/// when the subcommand is `eval`, which refuses any other time zone, since
/// ICU reads the host's.
fn fgro_command_in(mut command: Command, args: &[&str]) -> Command {
    for name in COLOR_VARS {
        command.env_remove(name);
    }
    command.args(args);
    if args.first() == Some(&"eval") {
        command.env("TZ", "UTC");
    }
    command
}

/// Runs `command` with `stdin` piped to it and stdout and stderr read on
/// threads of their own while it runs, so output larger than a pipe's buffer
/// can't block it, killing it at the [`DEADLINE`] or past [`MAX_RSS_KIB`] so
/// a run that never ends or grows without bound fails its test instead of
/// hanging the suite or taking the machine's memory.
pub fn spawn(mut command: Command, stdin: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        // a test's stdin is far smaller than a pipe's buffer, so this can't
        // block
        let mut pipe = child.stdin.take().unwrap();
        pipe.write_all(stdin).unwrap();
    }
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        let rss = peak_rss_kib(child.id());
        if start.elapsed() > DEADLINE || rss > MAX_RSS_KIB {
            child.kill().unwrap();
            child.wait().unwrap();
            let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
            panic!(
                "killed after {:?} at {rss} KiB resident: {command:?}\n{}\n{}",
                start.elapsed(),
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

/// The most resident memory the process `pid` has reached, in KiB, as
/// Linux reports it (`VmHWM`), or 0 when it can't be read.
fn peak_rss_kib(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
            line.split_whitespace().nth(1)?.parse().ok()
        })
        .unwrap_or(0)
}

/// A child killed and reaped when dropped, so a test that fails while it
/// runs leaves no process behind.
#[derive(Debug)]
pub struct KillOnDrop(pub Child);

impl Deref for KillOnDrop {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for KillOnDrop {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // an exited child is only reaped
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The lines `pipe` prints, each with its newline, read on a thread of its
/// own, so a test waits for one with `recv_timeout` and a child that never
/// prints can't hang it. The channel closes at the pipe's end.
pub fn lines(pipe: impl Read + Send + 'static) -> Receiver<String> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        loop {
            let mut line = String::new();
            // the test may have stopped waiting
            if !matches!(reader.read_line(&mut line), Ok(1..)) || send.send(line).is_err() {
                break;
            }
        }
    });
    receive
}

/// Reads `pipe` to its end on a thread of its own.
pub fn drain(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).unwrap();
        bytes
    })
}

/// The system's temporary directory, canonical, checked to be
/// world-writable with no `package.json` at or above it, so every walk up
/// from a file under it stops there, having found none.
///
/// # Panics
///
/// Panics, naming `TMPDIR`, when it isn't, which the tests that expect the
/// walks to stop there would fail on misleadingly.
pub fn system_temp() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let temp = std::env::temp_dir().canonicalize().unwrap();
    let mode = std::fs::metadata(&temp).unwrap().permissions().mode();
    assert!(
        mode & 0o002 != 0,
        "the temp dir {} (TMPDIR) isn't world-writable (mode {mode:o}), and the tests expect \
         every walk up from their files to stop there. Set TMPDIR to one that is, like /tmp",
        temp.display()
    );
    let package_json = temp
        .ancestors()
        .map(|dir| dir.join("package.json"))
        .find(|package_json| package_json.exists());
    if let Some(package_json) = package_json {
        panic!(
            "the temp dir {} (TMPDIR) is under a project: {} is at or above it, and the walks \
             from the tests' files would read it. Set TMPDIR outside one",
            temp.display(),
            package_json.display()
        );
    }
    temp
}

/// A fresh directory of files for one test, its path canonical, removed on
/// drop, so a test that panics leaves nothing behind. It's named for the
/// test binary, the process, and the test's `name`; one of the same name a
/// killed run left behind, under a pid since reused, is removed first. Every
/// directory it makes has the mode `0o755`, whatever the umask, so none is
/// world-writable, which would end every walk up from the files at it,
/// unless a test makes it so.
#[derive(Debug)]
pub struct Scratch(pub PathBuf);

impl Scratch {
    /// `files`, each at its relative path, under the target's temporary
    /// directory, which no world-writable directory holds.
    pub fn new(name: &str, files: &[(&str, &str)]) -> Self {
        Self::under(Path::new(env!("CARGO_TARGET_TMPDIR")), name, files)
    }

    /// `files`, each at its relative path, under the system's temporary
    /// directory, which is world-writable, so every walk up from the files
    /// stops there ([`system_temp`]).
    ///
    /// # Panics
    ///
    /// Panics, naming `TMPDIR`, when the temp dir isn't world-writable or a
    /// `package.json` is at or above it.
    pub fn in_system_temp(name: &str, files: &[(&str, &str)]) -> Self {
        Self::under(&system_temp(), name, files)
    }

    fn under(base: &Path, name: &str, files: &[(&str, &str)]) -> Self {
        let dir = base.join(format!(
            "gro_cli_{}_{}_{name}",
            env!("CARGO_CRATE_NAME"),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_mode(&dir, 0o755);
        for (file, content) in files {
            let path = dir.join(file);
            for parent in path.ancestors().skip(1) {
                if parent == dir {
                    break;
                }
                std::fs::create_dir_all(parent).unwrap();
                set_mode(parent, 0o755);
            }
            std::fs::write(path, content).unwrap();
        }
        Self(dir.canonicalize().unwrap())
    }

    /// The path of `file`, relative to the directory, as a string.
    pub fn path(&self, file: &str) -> String {
        self.0.join(file).display().to_string()
    }

    /// Sets the mode of `path`, relative to the directory.
    pub fn set_mode(&self, path: &str, mode: u32) {
        set_mode(&self.0.join(path), mode);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}
