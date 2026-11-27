//! A command of Gro's in the run's kernel box, end to end: `fgro gen` on a
//! project whose genfile changes a file's mode and times through the fs host
//! module and runs a native program outside the system's directories,
//! which only its pin's rule lets the box run, then runs a probe under
//! `python3`, a program the grant flags grant, which attacks each wall from
//! inside the box and prints what it
//! could and couldn't do: write `.git/hooks`, `node_modules`, a new
//! top-level file, and above the root; read a file in the home directory
//! beside git's config there; reach a listener of the test's on the host's
//! loopback, and bind one of its own; signal a process outside the box;
//! read the environment of `fgro`, its parent; and its own seccomp and
//! `no_new_privs` marks, and the environment the box set (the temporary
//! and `XDG_*` directories pointed into the run's private directory, the
//! vendored `deno_core`'s variables removed). The genfile's own write of a
//! new top-level file, which `gen`'s grants allow and the box refuses,
//! names where the box lets it write; and a run that fails after a tool it
//! started failed names the box's walls.
//!
//! The same probe runs under a seccomp filter refusing `unshare`, which
//! `python3` installs before it execs `fgro`, so the box takes the host's
//! network with every internet socket refused, and says so. Another filter,
//! refusing `landlock_create_ruleset` as a kernel without Landlock does,
//! forces the box's floor below, so the command falls back to the project's
//! Node Gro (a `node` of the test's own that records how it was run) with
//! the process as it started, a descriptor it inherited and its network
//! unchanged, or is refused under `GRO_FALLBACK=command`. Then the refusals
//! before the box: `-A`, and a socket on stdin; and `fgro run`, which has no
//! box.
//!
//! A command of Gro's runs in the box only from a snapshot build, so these
//! run from `cargo xtask build --verify` and are ignored in a plain `cargo
//! test`. They need Landlock ABI 6, `python3`, and the checkout's
//! `node_modules`, which the projects link whole, where Gro's embedded
//! library finds its peers; and a `node` on `PATH` that is the runtime
//! itself, not a version manager's shim, which the box refuses.

#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use gro_runtime::BINARY_NAME;
use support::{KillOnDrop, Scratch, fgro_command, spawn, version};

/// The variables the verb and the box read, and those the probe reports,
/// cleared so the harness's own don't leak in.
const CLEARED: [&str; 13] = [
    "GRO_FALLBACK",
    "NODE_ENV",
    "TMP",
    "TEMP",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_CONFIG_HOME",
    "GIT_CONFIG_GLOBAL",
    "DENO_SNAPSHOT_IMPORT_GRAPH",
    "DENO_LOG_LAZY_LOAD",
    "DENO_STARTUP_PHASES",
    "FAKE_NODE_RECORD",
];

/// The variables the genfile passes the probe, each forwarded by the flags.
const FORWARDED: &str = "TMP,TEMP,XDG_DATA_HOME,XDG_CACHE_HOME,XDG_STATE_HOME,\
                         DENO_SNAPSHOT_IMPORT_GRAPH,DENO_LOG_LAZY_LOAD,DENO_STARTUP_PHASES";

/// `unshare`'s and `landlock_create_ruleset`'s call numbers on `x86_64`.
const UNSHARE: u32 = 272;
const LANDLOCK_CREATE_RULESET: u32 = 444;

/// `EPERM` and `ENOSYS`, what a filter answers.
const EPERM: u32 = 1;
const ENOSYS: u32 = 38;

/// The genfile: the fs host module's mode and time changes on a file the
/// box writes, which go through `/proc/self/fd`, then the probe.
const GENFILE: &str = "import {fs} from '@fuzdev/gro/fs.ts';
import {spawn} from '@fuzdev/gro/spawn.ts';
export const gen = async () => {
\tconst ops = [
\t\t['set_mode', () => fs.project.set_mode('src/m.txt', 0o600)],
\t\t['set_times', () => fs.project.set_times('src/m.txt', {mtime_ms: 1e12})],
\t];
\tfor (const [name, op] of ops) {
\t\ttry {
\t\t\tawait op();
\t\t\tconsole.error(`probe: ${name} ok`);
\t\t} catch (error) {
\t\t\tconsole.error(`probe: ${name} ${error}`);
\t\t}
\t}
\tconst ran = await spawn.program('probe_true').run([]);
\tconsole.error(`probe: program ${ran.kind} ${ran.code}`);
\tconst writes = [
\t\t['realm_top', () => fs.project.write('realm_top.txt', 'x')],
\t\t['read_only_mode', () => fs.project.write('src/ro.txt', 'x')],
\t\t['renamed_into_git', () => fs.project.rename('src/mv.txt', '.git/mv.txt')],
\t\t['copied_into_git', () => fs.project.copy('src/m.txt', '.git/cp.txt')],
\t\t['through_dir_link', () => fs.project.write('src/dlink/x', 'x')],
\t\t['through_file_link', () => fs.project.write('src/flink', 'x')],
\t\t['renamed_out_of_git', () => fs.project.rename('.git/out.txt', 'src/out.txt')],
\t\t['removed_in_locked_dir', () => fs.project.remove('src/sub/l')],
\t\t['removed_top_level', () => fs.project.remove('top.txt')],
\t\t['moded_in_locked_dir', () => fs.project.set_mode('.git/locked/f', 0o644)],
\t];
\tfor (const [name, op] of writes) {
\t\ttry {
\t\t\tawait op();
\t\t\tconsole.error(`probe: ${name} ok`);
\t\t} catch (error) {
\t\t\tconsole.error(`probe: ${name} ${error.kind} ${error.cause?.message}`);
\t\t}
\t}
\t// a read through the project's node_modules, a symlink into another
\t// checkout, as gro's own checks of a sync read
\tconst linked = await fs.real_path(`${fs.project.path}/node_modules/esm-env/package.json`);
\tconst holder = await fs.open_dir(linked.slice(0, linked.lastIndexOf('/')));
\tconsole.error(`probe: linked_read ${await holder.exists('package.json')}`);
\tconst names = 'FORWARDED'.split(',');
\tawait spawn.program('python3', {pass_env: names}).run(['-I', 'probe.py']);
\treturn '// probed\\n';
};
";

/// The probe, run in the project: one line per attempt, `probe: <name> ok`
/// or the error's name, then its marks and the environment it was given.
const PROBE: &str = r#"
import errno, os, signal, socket
def attempt(name, act):
    try:
        act()
        print("probe:", name, "ok", flush=True)
    except OSError as error:
        print("probe:", name, errno.errorcode.get(error.errno, error.errno), flush=True)
def write(path):
    with open(path, "w") as file:
        file.write("x")
def read(path):
    with open(path, "rb") as file:
        file.read()
def connect():
    with socket.socket() as sock:
        sock.settimeout(5)
        sock.connect(("127.0.0.1", int(open("port.txt").read())))
def bind():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        sock.listen()
home = os.environ["HOME"]
attempt("git_hooks", lambda: write(".git/hooks/x"))
attempt("node_modules", lambda: write("node_modules/x"))
attempt("top_level", lambda: write("new.txt"))
attempt("above_root", lambda: write("../above.txt"))
attempt("src", lambda: write("src/x.txt"))
attempt("temp", lambda: write(os.environ["TMPDIR"] + "/x"))
attempt("run_private", lambda: os.listdir(os.path.dirname(os.environ["TMPDIR"])))
attempt("home_secret", lambda: read(home + "/secret.txt"))
attempt("home_gitconfig", lambda: read(home + "/.gitconfig"))
attempt("connect_outside", connect)
attempt("bind_loopback", bind)
attempt("signal_outside", lambda: os.kill(int(open("pid.txt").read()), signal.SIGCONT))
attempt("parent_environ", lambda: read("/proc/%d/environ" % os.getppid()))
for line in open("/proc/self/status").read().splitlines():
    name = line.split(":")[0]
    if name in ("Seccomp", "Seccomp_filters", "NoNewPrivs"):
        print("probe:", name, line.split()[1], flush=True)
for name in ("TMPDIR", "TMP", "TEMP", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME",
             "DENO_SNAPSHOT_IMPORT_GRAPH", "DENO_LOG_LAZY_LOAD", "DENO_STARTUP_PHASES"):
    print("probe:", name, os.environ.get(name, "<unset>"), flush=True)
"#;

/// A script `python3 -I -c` runs: installs a seccomp filter answering the
/// call numbered by its first argument with the error its second numbers,
/// allowing every other call; with a file named by the third (`-` for
/// none), opens it as descriptor 5, inherited; then execs the rest.
const FILTER: &str = r#"
import ctypes, os, struct, sys
number, error, held = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
code = struct.pack("HBBI", 0x20, 0, 0, 0)
code += struct.pack("HBBI", 0x15, 0, 1, number)
code += struct.pack("HBBI", 0x06, 0, 0, 0x00050000 | error)
code += struct.pack("HBBI", 0x06, 0, 0, 0x7fff0000)
buffer = ctypes.create_string_buffer(code)
class Program(ctypes.Structure):
    _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.c_void_p)]
program = Program(4, ctypes.cast(buffer, ctypes.c_void_p))
libc = ctypes.CDLL(None, use_errno=True)
ulong = ctypes.c_ulong
assert libc.prctl(38, ulong(1), ulong(0), ulong(0), ulong(0)) == 0
assert libc.prctl(22, ulong(2), ctypes.byref(program), ulong(0), ulong(0)) == 0
if held != "-":
    fd = os.open(held, os.O_RDONLY)
    os.dup2(fd, 5)
    os.set_inheritable(5, True)
os.execv(sys.argv[4], sys.argv[4:])
"#;

/// The checkout's `node_modules`, where Gro's peers are installed.
fn checkout_node_modules() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules")
        .canonicalize()
        .unwrap()
}

/// The scratch tree the probe runs in: a home with git's config and a
/// secret beside it, a temporary directory of its own, and the project,
/// its own `.git`, its `node_modules` the checkout's, linked whole.
fn probe_tree(name: &str) -> Scratch {
    let genfile = GENFILE.replace("FORWARDED", FORWARDED);
    let scratch = Scratch::new(
        name,
        &[
            ("home/.gitconfig", "[user]\n\tname = t\n"),
            ("home/secret.txt", "secret"),
            ("tmp/.keep", ""),
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/.git/hooks/.keep", ""),
            ("project/src/m.txt", "m"),
            ("project/src/ro.txt", "ro"),
            ("project/src/mv.txt", "mv"),
            ("project/.git/x", "x"),
            ("project/.git/out.txt", "out"),
            ("project/src/sub/.keep", ""),
            ("project/top.txt", "top"),
            ("project/.git/locked/f", "f"),
            ("project/src/probe.gen.ts", &genfile),
            ("project/probe.py", PROBE),
        ],
    );
    std::os::unix::fs::symlink(
        checkout_node_modules(),
        scratch.0.join("project/node_modules"),
    )
    .unwrap();
    // a file whose own mode refuses a write the box allows
    scratch.set_mode("project/src/ro.txt", 0o444);
    // symlinks inside the project into its `.git`, which the box keeps
    // read-only
    std::os::unix::fs::symlink("../.git/hooks", scratch.0.join("project/src/dlink")).unwrap();
    std::os::unix::fs::symlink("../.git/x", scratch.0.join("project/src/flink")).unwrap();
    // a link to `.git` in a directory whose own mode refuses its removal,
    // which the box allows; the mode is set only for the run (`probe`)
    std::os::unix::fs::symlink("../../.git/x", scratch.0.join("project/src/sub/l")).unwrap();
    // a native program outside the system's directories, which only its
    // pin's rule lets the box run
    std::fs::create_dir(scratch.0.join("bin")).unwrap();
    std::fs::copy("/bin/true", scratch.0.join("bin/probe_true")).unwrap();
    scratch
}

/// `command` given the scratch tree's home and temporary directory and the
/// variables cleared, in the project.
fn in_tree(mut command: Command, scratch: &Scratch) -> Command {
    for name in CLEARED {
        command.env_remove(name);
    }
    let path = std::env::var_os("PATH").unwrap();
    let path = std::env::join_paths(
        std::iter::once(scratch.0.join("bin")).chain(std::env::split_paths(&path)),
    )
    .unwrap();
    command
        .current_dir(scratch.0.join("project"))
        .env("PATH", path)
        .env("HOME", scratch.0.join("home"))
        .env("TMPDIR", scratch.0.join("tmp"))
        .env("TMP", "/host-tmp")
        .env(
            "DENO_SNAPSHOT_IMPORT_GRAPH",
            scratch.0.join("import_graph.txt"),
        )
        .env("DENO_LOG_LAZY_LOAD", "1");
    command
}

/// `fgro gen` granting the probe, as [`in_tree`] sets it up.
fn probe_args() -> Vec<String> {
    [
        "gen",
        "--allow-run=python3,probe_true",
        &format!("--allow-run-env={FORWARDED}"),
    ]
    .map(str::to_owned)
    .to_vec()
}

/// `command` wrapped by `python3`, which installs [`FILTER`] answering the
/// call `number` with `error`, holding `held` as descriptor 5 when given.
fn filtered(number: u32, error: u32, held: Option<&Path>, args: &[String]) -> Command {
    let mut command = Command::new(python3());
    command.args(["-I", "-c", FILTER]).args([
        number.to_string(),
        error.to_string(),
        held.map_or_else(|| "-".to_owned(), |held| held.display().to_string()),
    ]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    support::wrapped_fgro_command(command, &args)
}

/// `python3` on this process's `PATH`, by its path, so a command whose
/// `PATH` a test sets still finds it.
fn python3() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("python3"))
        .find(|path| path.is_file())
        .unwrap()
}

/// The run of `command`: its code, stdout, and stderr.
fn run(command: Command) -> (Option<i32>, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = spawn(command, b"");
    (
        status.code(),
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    )
}

/// What the probe printed, by name, from either stream.
fn probed(stdout: &str, stderr: &str) -> BTreeMap<String, String> {
    stdout
        .lines()
        .chain(stderr.lines())
        .filter_map(|line| line.strip_prefix("probe: "))
        .filter_map(|line| line.split_once(' '))
        .map(|(name, what)| (name.to_owned(), what.to_owned()))
        .collect()
}

/// Runs the probe in a fresh tree, by `command` made from the tree and
/// the probe's args, with a listener on the host's loopback and a process
/// outside the box, returning the run, what the probe printed, and the
/// tree.
fn probe(
    name: &str,
    command: impl FnOnce(&Scratch, &[String]) -> Command,
) -> (Option<i32>, String, BTreeMap<String, String>, Scratch) {
    let scratch = probe_tree(name);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let outside = KillOnDrop(Command::new("sleep").arg("60").spawn().unwrap());
    let project = scratch.0.join("project");
    std::fs::write(
        project.join("port.txt"),
        listener.local_addr().unwrap().port().to_string(),
    )
    .unwrap();
    std::fs::write(project.join("pid.txt"), outside.id().to_string()).unwrap();
    // `src/sub` refuses removals by its own mode for the run alone, so the
    // tree is removable however the run ends
    scratch.set_mode("project/src/sub", 0o555);
    // and `.git/locked` refuses a path through it, for the run alone too
    scratch.set_mode("project/.git/locked", 0o000);
    let (code, stdout, stderr) = run(in_tree(command(&scratch, &probe_args()), &scratch));
    scratch.set_mode("project/src/sub", 0o755);
    scratch.set_mode("project/.git/locked", 0o755);
    let probed = probed(&stdout, &stderr);
    // nothing reached the host's listener
    assert!(listener.accept().is_err(), "{stdout}{stderr}");
    (code, format!("{stdout}{stderr}"), probed, scratch)
}

/// The seccomp filters this process holds, which a process it starts
/// inherits.
fn own_filters() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("Seccomp_filters:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// What every probe in the box shows, whatever its network: the walls on
/// the files, the home directory, signals, and the parent's environment,
/// the marks, one more filter than its launcher's, `launched` (the box's
/// denylist), and the environment the box set.
fn assert_walls(output: &str, probed: &BTreeMap<String, String>, scratch: &Scratch, launched: u32) {
    assert_eq!(
        probed.get("Seccomp_filters"),
        Some(&(launched + 1).to_string()),
        "{output}"
    );
    let expected = [
        ("set_mode", "ok"),
        ("set_times", "ok"),
        ("program", "exited 0"),
        ("git_hooks", "EACCES"),
        ("node_modules", "EACCES"),
        ("top_level", "EACCES"),
        ("above_root", "EACCES"),
        ("src", "EACCES"),
        ("temp", "ok"),
        ("run_private", "EACCES"),
        ("home_secret", "EACCES"),
        ("home_gitconfig", "ok"),
        ("signal_outside", "EPERM"),
        ("parent_environ", "EACCES"),
        ("Seccomp", "2"),
        ("NoNewPrivs", "1"),
        ("TEMP", "<unset>"),
        ("DENO_SNAPSHOT_IMPORT_GRAPH", "<unset>"),
        ("DENO_LOG_LAZY_LOAD", "<unset>"),
        ("DENO_STARTUP_PHASES", "<unset>"),
    ];
    for (name, what) in expected {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    // the box's write set, the project's own node_modules linked, which
    // gets no tool directory
    assert_eq!(
        probed["realm_top"],
        format!(
            "permission_denied Permission denied (os error 13): the run's kernel box lets \
             {BINARY_NAME} gen write only to the top-level entries the project had as it started \
             (but .git, node_modules, the dot-entries, symlinks) and a temporary directory of \
             the run's; .git, node_modules, the other dot-entries, and a new top-level entry \
             stay read-only"
        ),
        "{output}"
    );
    // a path the box writes, refused by its own mode, gains no cause
    assert_eq!(
        probed["read_only_mode"], "permission_denied Permission denied (os error 13)",
        "{output}"
    );
    // a write whose real destination is `.git`, through a rename, a copy, or
    // a symlink inside the project, names the box, as does removing a
    // top-level entry, which the kernel checks by the project's own directory
    for name in [
        "renamed_into_git",
        "copied_into_git",
        "through_dir_link",
        "through_file_link",
        "renamed_out_of_git",
        "removed_top_level",
    ] {
        assert!(
            probed[name].starts_with(
                "permission_denied Permission denied (os error 13): the run's kernel box lets"
            ),
            "{name}: {output}"
        );
    }
    // an op on the entry is checked by its parent, as the kernel checks it:
    // removing a link to `.git` from a directory the box writes, refused by
    // that directory's own mode, names no box
    assert_eq!(
        probed["removed_in_locked_dir"], "permission_denied Permission denied (os error 13)",
        "{output}"
    );
    // Landlock gates no mode, so a mode refused in `.git`, by a directory's
    // own mode, names no box
    assert_eq!(
        probed["moded_in_locked_dir"], "permission_denied Permission denied (os error 13)",
        "{output}"
    );
    // the realm reads through the linked node_modules, as the box does
    assert_eq!(probed["linked_read"], "true", "{output}");
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("project/.git/x")).unwrap(),
        "x",
        "{output}"
    );
    let project = scratch.0.join("project");
    for written in [
        "new.txt",
        "realm_top.txt",
        ".git/hooks/x",
        "src/probe.ts",
        "src/x.txt",
    ] {
        let made = project.join(written).exists();
        assert_eq!(made, written == "src/probe.ts", "{written}: {output}");
    }
    assert!(!scratch.0.join("above.txt").exists(), "{output}");
    assert!(!scratch.0.join("import_graph.txt").exists(), "{output}");
    // the temporary directories point into the program's own, in the
    // run's, made in the temporary directory the process was given, named
    // by its place in the run's table of programs
    let temp = PathBuf::from(&probed["TMPDIR"]);
    let name = temp.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("program_"), "{output}");
    let private = temp.parent().unwrap();
    assert_eq!(
        private.parent(),
        Some(scratch.0.join("tmp").as_path()),
        "{output}"
    );
    let name = private.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("gro-"), "{output}");
    assert_eq!(probed["TMP"], probed["TMPDIR"], "{output}");
    for (variable, beneath) in [
        ("XDG_DATA_HOME", "xdg_data"),
        ("XDG_CACHE_HOME", "xdg_cache"),
        ("XDG_STATE_HOME", "xdg_state"),
    ] {
        assert_eq!(
            PathBuf::from(&probed[variable]),
            temp.join(beneath),
            "{output}"
        );
    }
}

/// Every wall holds from inside the box, in the run's own network, where a
/// test binds its loopback and reaches nothing outside; where the host
/// refuses the namespace, the box says so and refuses every internet
/// socket instead.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn each_wall_holds_from_inside_the_box() {
    let (code, output, probed, scratch) = probe("verb_box_walls", |_, args| {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        fgro_command(&args)
    });
    assert_eq!(code, Some(0), "{output}");
    assert_walls(&output, &probed, &scratch, own_filters());
    assert_ne!(probed["connect_outside"], "ok", "{output}");
    if output.contains("note: no network namespace here") {
        assert_ne!(probed["bind_loopback"], "ok", "{output}");
    } else {
        assert_eq!(probed["connect_outside"], "ECONNREFUSED", "{output}");
        assert_eq!(probed["bind_loopback"], "ok", "{output}");
    }
}

/// Where a seccomp filter refuses `unshare`, the box takes the host's
/// network with every internet socket refused, and says why, and every
/// other wall holds as in the run's own network.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn the_box_refuses_internet_sockets_where_the_namespace_is_refused() {
    let (code, output, probed, scratch) = probe("verb_box_inet", |_, args| {
        filtered(UNSHARE, EPERM, None, args)
    });
    assert_eq!(code, Some(0), "{output}");
    // where AppArmor restricts user namespaces, the box tells it before
    // `unshare`, which the filter never sees, and names it
    let restricted =
        std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
            .is_ok_and(|value| value.trim() == "1");
    let cause = if restricted {
        "AppArmor restricts unprivileged user namespaces \
         (kernel.apparmor_restrict_unprivileged_userns is 1)"
    } else {
        "a seccomp filter on the process refuses unshare"
    };
    assert!(
        output.contains(&format!(
            "note: no network namespace here ({cause}), so the box refuses every internet socket \
             in its place: tests that bind 127.0.0.1 fail\n"
        )),
        "{output}"
    );
    // the filter `python3` installed, then the box's
    assert_walls(&output, &probed, &scratch, own_filters() + 1);
    assert_ne!(probed["connect_outside"], "ok", "{output}");
    assert_ne!(probed["bind_loopback"], "ok", "{output}");
}

/// A project that installs `@fuzdev/gro` at the binary's version, built,
/// with a `node` in `bin/` that records its args, whether it holds
/// descriptor 5, and its network namespace, then exits 7.
fn planted_project(name: &str) -> Scratch {
    let package = format!(r#"{{"name": "@fuzdev/gro", "version": "{}"}}"#, version());
    let project = Scratch::new(
        name,
        &[
            ("package.json", r#"{"name": "p", "type": "module"}"#),
            ("node_modules/@fuzdev/gro/package.json", package.as_str()),
            ("node_modules/@fuzdev/gro/dist/invoke.js", "throw 1;\n"),
            ("held.txt", "held"),
            (
                "bin/node",
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$FAKE_NODE_RECORD\"\n\
                 if [ -e /proc/$$/fd/5 ]; then echo held >> \"$FAKE_NODE_RECORD\"; fi\n\
                 readlink /proc/$$/ns/net >> \"$FAKE_NODE_RECORD\"\nexit 7\n",
            ),
        ],
    );
    project.set_mode("bin/node", 0o755);
    project
}

/// Below the box's Landlock floor, the host decides the fallback: the
/// command runs on the project's Node Gro, saying why, with the process as
/// it started, its inherited descriptor open and its network the host's;
/// `GRO_FALLBACK=command` refuses it, naming the Node Gro's command.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn below_the_floor_the_host_decides_the_fallback() {
    let project = planted_project("verb_box_floor");
    let record = project.0.join("record.txt");
    let below = |strict: bool| {
        let args = ["check".to_owned()];
        let mut command = filtered(
            LANDLOCK_CREATE_RULESET,
            ENOSYS,
            Some(&project.0.join("held.txt")),
            &args,
        );
        for name in CLEARED {
            command.env_remove(name);
        }
        command
            .current_dir(&project.0)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", project.0.join("bin").display()),
            )
            .env("FAKE_NODE_RECORD", &record);
        if strict {
            command.env("GRO_FALLBACK", "command");
        }
        run(command)
    };
    let (code, stdout, stderr) = below(true);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert_eq!(
        stderr,
        "error: GRO_FALLBACK=command refuses running the project's own gro, which confines \
         nothing, in the kernel box's place: Landlock isn't available here (the kernel has none, \
         or a seccomp filter refuses its calls), which the kernel box needs at ABI 6 (Linux \
         6.12) or later. Run node_modules/.bin/gro check for the project's, unconfined\n"
    );
    assert!(!record.exists());
    let (code, stdout, stderr) = below(false);
    assert_eq!(code, Some(7), "{stdout}{stderr}");
    assert_eq!(
        stderr,
        format!(
            "note: {BINARY_NAME} check runs on the Node gro in node_modules/@fuzdev/gro {}, \
             which confines nothing, because Landlock isn't available here (the kernel has none, \
             or a seccomp filter refuses its calls), which the kernel box needs at ABI 6 (Linux \
             6.12) or later\n",
            version()
        )
    );
    let recorded = std::fs::read_to_string(&record).unwrap();
    let lines: Vec<&str> = recorded.lines().collect();
    let host_net = std::fs::read_link("/proc/self/ns/net").unwrap();
    assert_eq!(
        lines[lines.len() - 3..],
        ["check", "held", host_net.to_str().unwrap()],
        "{recorded}"
    );
}

/// `-A` after a command word is refused before anything runs, naming the
/// Node Gro's command, the unconfined spelling.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_command_s_minus_a_is_refused() {
    let scratch = probe_tree("verb_box_all");
    let (code, stdout, stderr) = run(in_tree(
        fgro_command(&["format", "-A", "--check"]),
        &scratch,
    ));
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert_eq!(
        stderr,
        "error: -A would run a command of Gro's unconfined, and the binary runs each in a kernel \
         box: the project's Node gro is the unconfined spelling (node_modules/.bin/gro format \
         --check)\n"
    );
}

/// A socket on stdin that isn't a connected unix stream is refused before
/// anything changes: the run, which grants a program, makes no private
/// directory.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_socket_on_stdin_is_refused() {
    let scratch = probe_tree("verb_box_stdin");
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let args = probe_args();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = in_tree(fgro_command(&args), &scratch);
    let output = command
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(socket)))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert_eq!(
        stderr,
        "error: stdin is a socket that isn't a connected unix stream, which reaches past the \
         box's walls: the run's box takes only a connected unix stream on stdio, which reaches \
         its peer alone; give it a file, a pipe, or /dev/null\n"
    );
    let made: Vec<_> = std::fs::read_dir(scratch.0.join("tmp"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with("gro-"))
        .collect();
    assert!(made.is_empty(), "{made:?}");
}

/// `fgro run` has no box: a program it starts holds no seccomp filter but
/// its launcher's.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn fgro_run_has_no_box() {
    let scratch = Scratch::new(
        "verb_box_run",
        &[(
            "a.mjs",
            "import {spawn} from '@fuzdev/gro/spawn.ts';\n\
             await spawn.program('python3').run(['-I', '-c', \
             'print([l for l in open(\"/proc/self/status\") if l.startswith(\"Seccomp_filters:\")][0])']);\n",
        )],
    );
    let mut command = fgro_command(&["run", "--allow-run=python3", "a.mjs"]);
    command.current_dir(&scratch.0);
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert_eq!(
        stdout.split_whitespace().collect::<Vec<_>>(),
        ["Seccomp_filters:", &own_filters().to_string()]
    );
}

/// A run that fails after a tool it started failed names the box's walls
/// on its last line, one that left a process holding its pipes, which the
/// run then killed, included; one that fails with every tool exiting 0
/// doesn't, nor does one whose tool the run itself stopped (a timeout).
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_failed_run_whose_tool_failed_names_the_walls() {
    let genfile = |python: &str, options: &str| {
        let (call, options) = match options {
            "out" => ("out", ""),
            options => ("run", options),
        };
        format!(
            "import {{spawn}} from '@fuzdev/gro/spawn.ts';\n\
             export const gen = async () => {{\n\
             \tawait spawn.program('python3').{call}(['-I', '-c', '{python}']{options});\n\
             \tthrow new Error('the genfile failed');\n\
             }};\n"
        )
    };
    for (name, python, options, noted) in [
        ("failed", "import sys; sys.exit(3)", "", true),
        // it exits 3 leaving a process that holds its pipes, which the run
        // then kills: the failure is the tool's own
        (
            "held",
            "import subprocess, sys; subprocess.Popen([\"sleep\", \"5\"]); sys.exit(3)",
            "out",
            true,
        ),
        ("passed", "import sys; sys.exit(0)", "", false),
        (
            "stopped",
            "import time; time.sleep(30)",
            ", {timeout_ms: 200}",
            false,
        ),
    ] {
        let source = genfile(python, options);
        let project = Scratch::new(
            &format!("verb_box_note_{name}"),
            &[
                ("package.json", r#"{"name": "p", "type": "module"}"#),
                ("src/n.gen.ts", &source),
            ],
        );
        std::os::unix::fs::symlink(checkout_node_modules(), project.0.join("node_modules"))
            .unwrap();
        let mut command = fgro_command(&["gen", "--allow-run=python3"]);
        for name in CLEARED {
            command.env_remove(name);
        }
        command.current_dir(&project.0);
        let (exit, stdout, stderr) = run(command);
        assert_eq!(exit, Some(1), "{name}: {stdout}{stderr}");
        let note = stderr
            .lines()
            .last()
            .filter(|line| line.starts_with("note: "))
            .unwrap_or_default();
        if !noted {
            assert_eq!(note, "", "{name}: {stderr}");
            continue;
        }
        assert!(
            note.starts_with(&format!(
                "note: {BINARY_NAME} gen's tools each ran in a kernel box of their own inside the \
                 run's, which lets them reach no network"
            )),
            "{stderr}"
        );
        // the project's node_modules linked, so no tool directory
        assert!(
            note.contains(
                " and write only to a temporary directory of their own: a tool that hits a wall \
                 fails with EACCES, EPERM, or ENOSYS; --allow-run-write=<program>:<dirs> lets one \
                 of them ("
            ),
            "{stderr}"
        );
        assert!(note.contains("python3"), "{stderr}");
        assert!(
            note.ends_with(
                ") write more, --allow-read= and --allow-run-in= widen every box, and \
                 --allow-write= widens only what gro's own code writes"
            ),
            "{stderr}"
        );
    }
}

/// A command in check mode writes nothing of the project: its box gives no
/// top-level entry a write rule, so a tool it starts can't write `src`,
/// though the same command's write mode can, while its temporary directory
/// stays its own.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_check_mode_run_writes_nothing_of_the_project() {
    let scratch = probe_tree("verb_box_check");
    let mut args = probe_args();
    args.push("--check".to_owned());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (_, stdout, stderr) = run(in_tree(fgro_command(&args), &scratch));
    let output = format!("{stdout}{stderr}");
    let probed = probed(&stdout, &stderr);
    for (name, what) in [("src", "EACCES"), ("top_level", "EACCES"), ("temp", "ok")] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    assert!(!scratch.0.join("project/src/x.txt").exists(), "{output}");
}

/// The genfile of [`each_tool_runs_in_a_box_of_its_own`], `PYDIR` the
/// directory `python3` is found in: each program's writes, temporary
/// variables, and working directory, a sibling's temporary directory read,
/// a `.bin` script run by its `node`, a program found by its pin beneath an
/// exec root, then gro's own writes.
const OWN_BOX_GENFILE: &str = "import {fs} from '@fuzdev/gro/fs.ts';
import {spawn} from '@fuzdev/gro/spawn.ts';
const say = (name, what) => console.error(`probe: ${name} ${what}`);
const lines = (text) => {
\tfor (const line of text.split('\\n')) if (line.startsWith('probe: ')) console.error(line);
};
export const gen = async () => {
\tconst python = spawn.program('python3', {pass_env: ['TMP']});
\tconst py = async (...args) => lines((await python.out(['-I', 'own.py', ...args])).stdout);
\tawait py('own_dir', 'write', 'out/x');
\tawait py('write_flag_dir', 'write', 'wout/x');
\tawait py('run_private', 'private');
\tawait py('py_temp', 'temp');
\tawait py('py', 'env');
\tconst src = await fs.open_dir(`${fs.project.path}/src`);
\tconst sh = await spawn.program('sh').out(
\t\t['-c', 'echo \"probe: sh_tmp ${TMP-unset}\"; echo \"probe: sh_tmpdir $TMPDIR\"; echo x > \"$TMPDIR/sib\" && echo \"probe: sh_own ok\"; echo \"probe: sh_pwd $(pwd -P)\"'],
\t\t{cwd: src},
\t);
\tlines(sh.stdout);
\tconst sh_tmpdir = sh.stdout.match(/probe: sh_tmpdir (.*)/)[1];
\tawait py('sibling_read', 'read', `${sh_tmpdir}/sib`);
\tlines((await spawn.program('node_modules/.bin/tool').out([])).stdout);
\tconst found = await spawn.find_in(await fs.open_dir('PYDIR'), 'python3');
\tlines((await found.out(['-I', 'own.py', 'found_write', 'write', 'out/found'])).stdout);
\tfor (const [name, path] of [['realm_own_dir', 'out/z'], ['realm_write_flag_dir', 'wout/z']]) {
\t\ttry {
\t\t\tawait fs.project.write(path, 'z');
\t\t\tsay(name, 'ok');
\t\t} catch (error) {
\t\t\tsay(name, error.kind);
\t\t}
\t}
\treturn '// probed\\n';
};
";

/// The script [`OWN_BOX_GENFILE`] runs under `python3`: its first argument
/// names the line it prints, the second what it does.
const OWN_BOX_PROBE: &str = r#"
import errno, os, sys
name, act, args = sys.argv[1], sys.argv[2], sys.argv[3:]
def attempt(do):
    try:
        do()
        print("probe:", name, "ok", flush=True)
    except OSError as error:
        print("probe:", name, errno.errorcode.get(error.errno, error.errno), flush=True)
def write(path):
    with open(path, "w") as file:
        file.write("x")
def read(path):
    with open(path, "rb") as file:
        file.read()
def temp():
    write(os.environ["TMPDIR"] + "/x")
    os.makedirs(os.environ["XDG_DATA_HOME"] + "/vitest")
    write(os.environ["XDG_DATA_HOME"] + "/vitest/.vitest-secret-token")
if act == "write":
    attempt(lambda: write(args[0]))
elif act == "read":
    attempt(lambda: read(args[0]))
elif act == "private":
    attempt(lambda: os.listdir(os.path.dirname(os.environ["TMPDIR"])))
elif act == "temp":
    attempt(temp)
else:
    for variable in ("TMPDIR", "TMP", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"):
        print("probe:", name + "_" + variable, os.environ.get(variable, "<unset>"), flush=True)
"#;

/// Each program a command starts runs in a box of its own inside the
/// run's: `--allow-run-write=` lets the program it names write its
/// directory and no other program, `--allow-write=` lets gro's own code
/// write and no program, and a program found by its pin beneath an exec
/// root takes nothing of the box of the granted program pinning the same
/// file; each program's temporary variables name its own directory, `TMP`
/// only where its environment has it, which no other program reads, nor
/// the run's; a native program runs by its pin's descriptor, in the
/// working directory a call names, and a `.bin` script by its `node`. Gro's
/// own code writes the program's directory only where its grants let it,
/// under `gen` and not under `gen --check`, which drops every write of the
/// realm's, its refusal failing the run.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn each_tool_runs_in_a_box_of_its_own() {
    let python_dir = python3().parent().unwrap().to_owned();
    let scratch = Scratch::new("verb_box_own_box", &[("tmp/.keep", "")]);
    let temp = scratch.0.join("tmp");
    let genfile = OWN_BOX_GENFILE.replace("PYDIR", &python_dir.display().to_string());
    let project = scratch.0.join("project");
    for (path, text) in [
        ("package.json", r#"{"name": "p", "type": "module"}"#),
        ("src/probe.gen.ts", genfile.as_str()),
        ("src/probe.ts", "// probed\n"),
        ("own.py", OWN_BOX_PROBE),
        ("out/.keep", ""),
        ("wout/.keep", ""),
        (
            "node_modules/.bin/tool",
            "#!/usr/bin/env node\nconst temp = process.env.TMPDIR;\n\
             console.log(`probe: node_script ${/\\/program_\\d+$/.test(temp) ? 'own' : temp}`);\n",
        ),
    ] {
        let path = project.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    scratch.set_mode("project/node_modules/.bin/tool", 0o755);
    for package in ["@fuzdev/fuz_util", "@fuzdev/fuz_schema", "esm-env"] {
        copy_tree(
            &checkout_node_modules().join(package),
            &project.join("node_modules").join(package),
        );
    }
    std::fs::create_dir(scratch.0.join("bin")).unwrap();
    std::fs::copy("/bin/true", scratch.0.join("bin/probe_true")).unwrap();
    let run_in = format!("--allow-run-in={}", python_dir.display());
    let read = format!("--allow-read={}", python_dir.display());
    let path = std::env::var_os("PATH").unwrap();
    let path = std::env::join_paths(
        std::iter::once(scratch.0.join("bin")).chain(std::env::split_paths(&path)),
    )
    .unwrap();
    let run_gen = |check: bool| {
        let mut args = vec![
            "gen",
            "--allow-run=python3,sh,probe_true,node_modules/.bin/tool",
            "--allow-run-env=TMP",
            "--allow-run-write=python3:out",
            "--allow-write=wout",
            &run_in,
            &read,
        ];
        if check {
            args.push("--check");
        }
        let mut command = fgro_command(&args);
        for name in CLEARED {
            command.env_remove(name);
        }
        command
            .current_dir(&project)
            .env("PATH", &path)
            .env("TMPDIR", &temp)
            .env("TMP", "/host-tmp");
        let (code, stdout, stderr) = run(command);
        let output = format!("{stdout}{stderr}");
        // a refusal of gro's own code fails the run, caught or not
        assert_eq!(code, Some(i32::from(check)), "{output}");
        (output, probed(&stdout, &stderr))
    };
    // gro's own code holds no write under the check mode, though a program
    // writes its own directory
    let (output, probed) = run_gen(true);
    for (name, what) in [("own_dir", "ok"), ("realm_own_dir", "not_granted")] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    std::fs::remove_file(project.join("out/x")).unwrap();
    let (output, probed) = run_gen(false);
    let src = project.join("src").canonicalize().unwrap();
    let src = src.display().to_string();
    for (name, what) in [
        ("own_dir", "ok"),
        ("write_flag_dir", "EACCES"),
        ("run_private", "EACCES"),
        ("py_temp", "ok"),
        ("sh_tmp", "unset"),
        ("sh_own", "ok"),
        ("sh_pwd", src.as_str()),
        ("sibling_read", "EACCES"),
        ("node_script", "own"),
        ("found_write", "EACCES"),
        ("realm_own_dir", "ok"),
        ("realm_write_flag_dir", "ok"),
    ] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    assert!(project.join("out/x").exists(), "{output}");
    assert!(!project.join("wout/x").exists(), "{output}");
    assert!(!project.join("out/found").exists(), "{output}");
    // python3's variables name its own directory in the run's, TMP among
    // them as its environment has it; sh's is another
    let own = PathBuf::from(&probed["py_TMPDIR"]);
    let place = own.file_name().unwrap().to_str().unwrap();
    assert!(place.starts_with("program_"), "{output}");
    assert_eq!(
        own.parent().unwrap().parent(),
        Some(temp.as_path()),
        "{output}"
    );
    assert_eq!(probed["py_TMP"], probed["py_TMPDIR"], "{output}");
    for (variable, beneath) in [
        ("py_XDG_DATA_HOME", "xdg_data"),
        ("py_XDG_CACHE_HOME", "xdg_cache"),
        ("py_XDG_STATE_HOME", "xdg_state"),
    ] {
        assert_eq!(
            PathBuf::from(&probed[variable]),
            own.join(beneath),
            "{output}"
        );
    }
    assert_ne!(probed["sh_tmpdir"], probed["py_TMPDIR"], "{output}");
    assert_eq!(
        Path::new(&probed["sh_tmpdir"]).parent(),
        own.parent(),
        "{output}"
    );
}

/// The genfile of [`a_program_whose_box_can_t_be_built_fails_each_launch_alike`],
/// `TEMP` the OS temporary directory the run's private directory is in,
/// which `--allow-write=` grants gro's own code: it reads the private
/// directory's name from a program's `TMPDIR`, plants an entry at each
/// name a program's temporary directory could take, then launches `sh`
/// twice, removing the entries between.
const PLANT_GENFILE: &str = "import {fs} from '@fuzdev/gro/fs.ts';
import {spawn} from '@fuzdev/gro/spawn.ts';
const say = (name, what) => console.error(`probe: ${name} ${what}`);
export const gen = async () => {
\tconst own = (await spawn.program('python3').out(['-I', '-c', 'import os; print(os.environ[\"TMPDIR\"])'])).stdout.trim();
\tconst run_dir = own.slice('TEMP'.length + 1, own.lastIndexOf('/'));
\tconst temp = await fs.open_dir('TEMP');
\tconst planted = [];
\tconst own_name = own.slice(own.lastIndexOf('/') + 1);
\tfor (let place = 0; place < 16; place++) {
\t\tif (`program_${place}` === own_name) continue;
\t\tconst path = `${run_dir}/program_${place}`;
\t\tawait temp.mkdir(path);
\t\tplanted.push(path);
\t}
\tconst sh = spawn.program('sh');
\tconst started = async () => {
\t\ttry {
\t\t\tawait sh.run(['-c', 'true']);
\t\t\treturn 'ran';
\t\t} catch (error) {
\t\t\tconst chain = [error.kind];
\t\t\tfor (let cause = error.cause; cause; cause = cause.cause) chain.push(cause.message);
\t\t\treturn chain.join(' | ');
\t\t}
\t};
\tconst first = await started();
\tfor (const path of planted) await temp.remove(path);
\tconst again = await started();
\tsay('box_failed', first);
\tsay('box_failed_again', again === first ? 'same' : again);
\treturn '// probed\\n';
};
";

/// A program whose box can't be built, its temporary directory's name
/// taken already (here by gro's own code, which `--allow-write=` on the OS
/// temporary directory lets write the run's private directory), fails its
/// launch with `launch_failed` naming the box, and fails each later launch
/// the same way, its build never tried again, though the entry is gone.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_program_whose_box_can_t_be_built_fails_each_launch_alike() {
    let scratch = Scratch::new("verb_box_unbuilt", &[("tmp/.keep", "")]);
    // by its real path, as the run reads the OS temporary directory
    let temp = scratch.0.join("tmp").canonicalize().unwrap();
    let genfile = PLANT_GENFILE.replace("TEMP", &temp.display().to_string());
    let project = scratch.0.join("project");
    for (path, text) in [
        ("package.json", r#"{"name": "p", "type": "module"}"#),
        ("src/probe.gen.ts", genfile.as_str()),
        ("src/probe.ts", "// probed\n"),
    ] {
        let path = project.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    std::os::unix::fs::symlink(checkout_node_modules(), project.join("node_modules")).unwrap();
    let write_temp = format!("--allow-write={}", temp.display());
    let mut command = fgro_command(&["gen", "--allow-run=python3,sh", &write_temp]);
    for name in CLEARED {
        command.env_remove(name);
    }
    command.current_dir(&project).env("TMPDIR", &temp);
    let (code, stdout, stderr) = run(command);
    let output = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0), "{output}");
    let probed = probed(&stdout, &stderr);
    let failed: Vec<&str> = probed["box_failed"].split(" | ").collect();
    let [kind, launch, cause] = failed.as_slice() else {
        panic!("{output}");
    };
    assert_eq!(*kind, "launch_failed", "{output}");
    assert!(
        launch.starts_with("sh at ") && launch.ends_with(" didn't start"),
        "{output}"
    );
    assert!(
        cause.starts_with(&format!(
            "its box couldn't be built: failed to make the program's temporary directory {}/gro-",
            temp.display()
        )) && cause.ends_with(": File exists (os error 17)"),
        "{output}"
    );
    assert_eq!(probed["box_failed_again"], "same", "{output}");
}

/// A project for a write flag's refusals before the box: its own `.git`,
/// a genfile that writes its output, a directory outside the project and a
/// top-level symlink leading to it, a temporary directory of its own, and
/// the checkout's `node_modules`, linked whole.
fn write_flag_project(name: &str) -> Scratch {
    let scratch = Scratch::new(
        name,
        &[
            ("tmp/.keep", ""),
            ("outside/.keep", ""),
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/.git/hooks/.keep", ""),
            ("project/src/inner/.keep", ""),
            (
                "project/src/x.gen.ts",
                "export const gen = () => '// generated\\n';\n",
            ),
        ],
    );
    let project = scratch.0.join("project");
    std::os::unix::fs::symlink(checkout_node_modules(), project.join("node_modules")).unwrap();
    std::os::unix::fs::symlink("../outside", project.join("out")).unwrap();
    std::os::unix::fs::symlink("src/inner", project.join("in")).unwrap();
    scratch
}

/// `fgro <args>` in `scratch`'s project, its temporary directory the
/// scratch tree's, the variables the box reads cleared.
fn in_write_flag_project(scratch: &Scratch, args: &[&str]) -> (Option<i32>, String, String) {
    let mut command = fgro_command(args);
    for name in CLEARED {
        command.env_remove(name);
    }
    command
        .current_dir(scratch.0.join("project"))
        .env("TMPDIR", scratch.0.join("tmp"));
    run(command)
}

/// The run's private directories in `scratch`'s temporary directory.
fn private_dirs(scratch: &Scratch) -> Vec<String> {
    std::fs::read_dir(scratch.0.join("tmp"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("gro-"))
        .collect()
}

/// Where gro's own code writes the whole project, a program's directory
/// under `.git` is refused, exit 2, naming the command, before the box and
/// before anything is made; `--allow-write=` naming it too is the way
/// through, which runs; and a command whose code writes no root honors it.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_program_s_directory_gro_s_code_would_write_is_refused() {
    let scratch = write_flag_project("verb_box_run_write_corner");
    let (code, stdout, stderr) = in_write_flag_project(
        &scratch,
        &["gen", "--allow-run-write=svelte-kit:.git/hooks"],
    );
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.ends_with(&format!(
            "error: --allow-run-write=svelte-kit:.git/hooks would let gro's own code write \
             .git/hooks as well, since {BINARY_NAME} gen writes the project and only the kernel \
             box keeps it out of .git, node_modules, and the dot-entries: add \
             --allow-write=.git/hooks to grant it to both\n"
        )),
        "{stderr}"
    );
    assert!(!scratch.0.join("project/src/x.ts").exists());
    assert_eq!(private_dirs(&scratch), Vec::<String>::new());
    // a top-level symlink to `.git`, which the flag doesn't name, is
    // refused before the plan reads its real path
    std::os::unix::fs::symlink(".git", scratch.0.join("project/g")).unwrap();
    let (code, stdout, stderr) =
        in_write_flag_project(&scratch, &["gen", "--allow-run-write=svelte-kit:g"]);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.contains(
            "--allow-run-write=svelte-kit:g names g, which leads through a symlink to .git"
        ),
        "{stderr}"
    );
    std::fs::remove_file(scratch.0.join("project/g")).unwrap();
    let (code, stdout, stderr) = in_write_flag_project(
        &scratch,
        &[
            "gen",
            "--allow-write=.git/hooks",
            "--allow-run-write=svelte-kit:.git/hooks",
        ],
    );
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(scratch.0.join("project/src/x.ts").exists(), "{stderr}");
    // gen's check mode writes nothing, so the program's directory is
    // honored
    let (code, stdout, stderr) = in_write_flag_project(
        &scratch,
        &["gen", "--allow-run-write=svelte-kit:.git/hooks", "--check"],
    );
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(!stderr.contains("would let gro's own code"), "{stderr}");
}

/// A directory a command's grant flag lists inside the project by its text
/// that leads out of it through a symlink is refused, exit 2, before the
/// box and before anything is made, for each directory flag, and so is one
/// leading through a symlink into `.git`, which it doesn't name; one leading
/// through a symlink that stays inside is noted and runs; and `fgro run`
/// notes the one leading out, the path being the person's.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_command_s_directory_leading_out_of_the_project_is_refused() {
    let scratch = write_flag_project("verb_box_leaves_project");
    let outside = scratch.0.join("outside").canonicalize().unwrap();
    for flag in [
        "--allow-read=out",
        "--allow-write=out",
        "--allow-run-in=out",
        "--allow-run-write=svelte-kit:out",
    ] {
        let (code, stdout, stderr) = in_write_flag_project(&scratch, &["gen", flag, "--check"]);
        assert_eq!(code, Some(2), "{flag}: {stdout}{stderr}");
        assert!(
            stderr.ends_with(&format!(
                "error: {flag} names out, which leads out of the project to {} through a \
                 symlink: a directory typed inside the project must be inside it, so name {} \
                 itself to grant it\n",
                outside.display(),
                outside.display()
            )),
            "{flag}: {stderr}"
        );
        assert_eq!(private_dirs(&scratch), Vec::<String>::new(), "{flag}");
    }
    // a link inside that lands in `.git`, which the flag doesn't name
    std::os::unix::fs::symlink("../.git", scratch.0.join("project/src/fixtures")).unwrap();
    for flag in [
        "--allow-write=src/fixtures",
        "--allow-run-write=svelte-kit:src/fixtures",
    ] {
        let (code, stdout, stderr) = in_write_flag_project(&scratch, &["gen", flag, "--check"]);
        assert_eq!(code, Some(2), "{flag}: {stdout}{stderr}");
        assert!(
            stderr.ends_with(&format!(
                "error: {flag} names src/fixtures, which leads through a symlink to .git: name \
                 .git itself to grant it\n"
            )),
            "{flag}: {stderr}"
        );
        assert_eq!(private_dirs(&scratch), Vec::<String>::new(), "{flag}");
    }
    std::fs::remove_file(scratch.0.join("project/src/fixtures")).unwrap();
    let inner = scratch.0.join("project/src/inner").canonicalize().unwrap();
    let (code, stdout, stderr) = in_write_flag_project(&scratch, &["gen", "--allow-read=in"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!(
            "note: --allow-read=in grants {}\n",
            inner.display()
        )),
        "{stderr}"
    );
    std::fs::write(scratch.0.join("project/a.mjs"), "").unwrap();
    let (code, stdout, stderr) =
        in_write_flag_project(&scratch, &["run", "--allow-read=out", "a.mjs"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert_eq!(
        stderr,
        format!("note: --allow-read=out grants {}\n", outside.display())
    );
}

/// A grant flag holding the temporary directory, which would give a
/// program the run starts all of its private directory, is refused for a
/// run that starts a program, before anything is made: a flag on its
/// parent, on itself, and on `/`, each naming the flag and the fix, a
/// program's write directory on it, and a read flag on it under a `TMPDIR`
/// that's a symlink to it, refused by its real path; a write flag on it,
/// which gives programs nothing, runs, as does a flag beside it, and a run
/// that starts no program under the same flag.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_flag_holding_the_temporary_directory_is_refused() {
    let scratch = probe_tree("verb_box_temp_held");
    let root = std::fs::canonicalize(&scratch.0).unwrap();
    let temp = root.join("tmp");
    for (flag, rule) in [
        (format!("--allow-read={}", root.display()), root.clone()),
        (format!("--allow-run-in={}", temp.display()), temp.clone()),
        ("--allow-read=/".to_owned(), PathBuf::from("/")),
    ] {
        let mut args = probe_args();
        args.push(flag.clone());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, stdout, stderr) = run(in_tree(fgro_command(&args), &scratch));
        assert_eq!(code, Some(2), "{flag}: {stdout}{stderr}");
        let spelled = flag.split_once('=').unwrap().0;
        assert!(
            stderr.ends_with(&format!(
                "error: the kernel box makes the run's private directory in the OS's temporary \
                 directory, {}, and {spelled}={} holds it, so every program the run starts could \
                 read the others' temporary directories there: narrow {spelled} to a directory \
                 that doesn't hold {}, or point TMPDIR elsewhere\n",
                temp.display(),
                rule.display(),
                temp.display()
            )),
            "{flag}: {stderr}"
        );
        let made: Vec<_> = std::fs::read_dir(&temp)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.starts_with("gro-"))
            .collect();
        assert!(made.is_empty(), "{flag}: {made:?}");
    }
    // a program's write directory holds it for that program; a write flag,
    // which gives programs nothing, read included, holds nothing of theirs
    let mut args = probe_args();
    args.push(format!("--allow-run-write=python3:{}", temp.display()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = run(in_tree(fgro_command(&args), &scratch));
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.ends_with(&format!(
            "error: the kernel box makes the run's private directory in the OS's temporary \
             directory, {}, and --allow-run-write=python3:{} holds it, so python3 could read \
             every other program's temporary directory there: narrow python3's directories to \
             ones that don't hold {}, or point TMPDIR elsewhere\n",
            temp.display(),
            temp.display(),
            temp.display()
        )),
        "{stderr}"
    );
    let mut args = probe_args();
    args.push(format!("--allow-write={}", temp.display()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = run(in_tree(fgro_command(&args), &scratch));
    assert_ne!(code, Some(2), "{stdout}{stderr}");
    assert!(!stderr.contains("holds it"), "{stderr}");
    // a `TMPDIR` that's a symlink to it is held by its real path, which the
    // refusal names
    let linked = root.join("tmp_link");
    std::os::unix::fs::symlink(&temp, &linked).unwrap();
    let mut args = probe_args();
    args.push(format!("--allow-read={}", temp.display()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = in_tree(fgro_command(&args), &scratch);
    command.env("TMPDIR", &linked);
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!(
            "temporary directory, {}, and --allow-read={} holds it",
            temp.display(),
            temp.display()
        )),
        "{stderr}"
    );
    // beside it, not holding it
    std::fs::create_dir(root.join("tmpx")).unwrap();
    let mut args = probe_args();
    args.push(format!("--allow-read={}", root.join("tmpx").display()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = run(in_tree(fgro_command(&args), &scratch));
    assert_ne!(code, Some(2), "{stdout}{stderr}");
    assert!(!stderr.contains("holds it"), "{stderr}");
    // `format --check` starts no program, so makes no private directory
    let flag = format!("--allow-read={}", root.display());
    let (code, stdout, stderr) = run(in_tree(
        fgro_command(&["format", &flag, "--check"]),
        &scratch,
    ));
    assert_ne!(code, Some(2), "{stdout}{stderr}");
    assert!(!stderr.contains("holds it"), "{stderr}");
    assert!(!stderr.contains("expected no unknown keys"), "{stderr}");
}

/// A temporary directory inside the project is refused for a run that
/// makes its private directory there, before anything is made, and isn't
/// swept; a run that makes none runs.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_temporary_directory_in_the_project_is_refused() {
    let scratch = probe_tree("verb_box_temp");
    let project = scratch.0.join("project");
    // a leftover the sweep would remove, were it swept
    std::fs::create_dir(project.join("src/gro-999999999-1")).unwrap();
    let args = probe_args();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = in_tree(fgro_command(&args), &scratch);
    command.env("TMPDIR", project.join("src"));
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.ends_with(&format!(
            "error: the kernel box makes the run's private directory in the OS's temporary \
             directory, and {} is inside the project, where the run's tools could write what \
             later commands read: point TMPDIR outside the project\n",
            project.join("src").display()
        )),
        "{stderr}"
    );
    let entries: Vec<_> = std::fs::read_dir(project.join("src"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("gro-"))
        .collect();
    assert_eq!(entries, ["gro-999999999-1"], "{stderr}");
    // the project by another path, `/proc/self/root`'s, which resolves to
    // it, is the project
    let other_name = Path::new("/proc/self/root").join(project.strip_prefix("/").unwrap());
    let mut command = in_tree(fgro_command(&args), &scratch);
    command.env("TMPDIR", other_name.join("src"));
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.contains("point TMPDIR outside the project"),
        "{stderr}"
    );
    // `format --check` starts no program, so makes no private directory
    let mut command = in_tree(fgro_command(&["format", "--check"]), &scratch);
    command.env("TMPDIR", project.join("src"));
    let (code, stdout, stderr) = run(command);
    assert_ne!(code, Some(2), "{stdout}{stderr}");
    assert!(!stderr.contains("point TMPDIR"), "{stderr}");
}

/// A project whose `node_modules` links another checkout's reads through it
/// in the realm, as the box does, with a write flag on that checkout's
/// `node_modules` too; a read through any other symlink leaving the root is
/// still refused, failing the run.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn the_realm_reads_a_linked_node_modules_and_no_other_link_out() {
    let genfile = |path: &str| {
        format!(
            "import {{fs}} from '@fuzdev/gro/fs.ts';\n\
             export const gen = async () => {{\n\
             \tconst real = await fs.real_path(`${{fs.project.path}}/{path}`);\n\
             \tconst dir = await fs.open_dir(real.slice(0, real.lastIndexOf('/')));\n\
             \tconsole.error(`probe: read ${{await dir.exists(real.slice(real.lastIndexOf('/') + 1))}}`);\n\
             \treturn '// read\\n';\n\
             }};\n"
        )
    };
    let linked = genfile("node_modules/esm-env/package.json");
    let out = genfile("src/out/secret.txt");
    let scratch = Scratch::new(
        "verb_box_realm_link",
        &[
            ("outside/secret.txt", "secret"),
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/src/linked.gen.ts", &linked),
        ],
    );
    let project = scratch.0.join("project");
    std::os::unix::fs::symlink(checkout_node_modules(), project.join("node_modules")).unwrap();
    std::os::unix::fs::symlink(scratch.0.join("outside"), project.join("src/out")).unwrap();
    let generate = |flags: &[String]| {
        let mut args = vec!["gen".to_owned()];
        args.extend_from_slice(flags);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut command = fgro_command(&args);
        for name in CLEARED {
            command.env_remove(name);
        }
        command.current_dir(&project);
        run(command)
    };
    let write_flag = format!("--allow-write={}", checkout_node_modules().display());
    for flags in [Vec::new(), vec![write_flag]] {
        let (code, stdout, stderr) = generate(&flags);
        assert_eq!(code, Some(0), "{flags:?}: {stdout}{stderr}");
        assert!(stderr.contains("probe: read true"), "{flags:?}: {stderr}");
    }
    std::fs::remove_file(project.join("src/linked.gen.ts")).unwrap();
    std::fs::write(project.join("src/out.gen.ts"), out).unwrap();
    let (code, stdout, stderr) = generate(&[]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("fs refused"), "{stderr}");
    assert!(!stderr.contains("probe: read"), "{stderr}");
}

/// The realm's root on a linked `node_modules` reads only: with no write
/// flag on its target, a write through the link is the realm's refusal,
/// `not_granted`, before the box would refuse it.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn the_realm_root_on_a_linked_node_modules_reads_only() {
    let genfile = "import {fs} from '@fuzdev/gro/fs.ts';\n\
         export const gen = async () => {\n\
         \tconst dir = await fs.open_dir(`${fs.project.path}/node_modules`);\n\
         \ttry { await dir.write('gro_probe_never_written.txt', 'x', {create_new: true}); console.error('probe: wrote'); }\n\
         \tcatch (e) { console.error(`probe: ${e.kind}`); }\n\
         \treturn '// read\\n';\n\
         };\n";
    let scratch = Scratch::new(
        "verb_box_realm_link_read_only",
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/src/linked.gen.ts", genfile),
        ],
    );
    let project = scratch.0.join("project");
    std::os::unix::fs::symlink(checkout_node_modules(), project.join("node_modules")).unwrap();
    let mut command = fgro_command(&["gen"]);
    for name in CLEARED {
        command.env_remove(name);
    }
    command.current_dir(&project);
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("probe: not_granted"), "{stderr}");
    assert!(
        !checkout_node_modules()
            .join("gro_probe_never_written.txt")
            .exists()
    );
}

/// Copies the directory `from` to `to`, its files and directories, symlinks
/// followed.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if std::fs::metadata(entry.path()).unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// With `--allow-write=` naming where a linked `node_modules` leads, the
/// realm writes through the link's own spelling, the write flag's root and
/// the read root naming one directory; the write lands in the other
/// directory, a copy of the packages Gro's library reads, never the
/// checkout's.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_write_flag_on_a_linked_node_modules_writes_through_the_link() {
    let genfile = "import {fs} from '@fuzdev/gro/fs.ts';\n\
         export const gen = async () => {\n\
         \tconst dir = await fs.open_dir(`${fs.project.path}/node_modules`);\n\
         \tawait dir.write('written_through.txt', 'x');\n\
         \treturn '// wrote\\n';\n\
         };\n";
    let scratch = Scratch::new(
        "verb_box_realm_link_write",
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/src/write.gen.ts", genfile),
        ],
    );
    let other = scratch.0.join("other/node_modules");
    for package in ["@fuzdev/fuz_util", "@fuzdev/fuz_schema", "esm-env"] {
        copy_tree(&checkout_node_modules().join(package), &other.join(package));
    }
    let project = scratch.0.join("project");
    std::os::unix::fs::symlink(&other, project.join("node_modules")).unwrap();
    let flag = format!("--allow-write={}", other.display());
    let mut command = fgro_command(&["gen", &flag]);
    for name in CLEARED {
        command.env_remove(name);
    }
    command.current_dir(&project);
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(other.join("written_through.txt").exists(), "{stderr}");
}

/// A `node_modules` linked from another checkout gets no tool directory,
/// and a tool writes there only what `--allow-run-write=` names for it:
/// `--allow-write=` naming where it leads makes nothing there and lets no
/// tool write it, and `--allow-run-write=vitest:` naming it lets vitest
/// write there and no other tool, the run's note saying which. The other
/// directory holds the fake table tools and a copy of the packages Gro's
/// library reads, never the checkout's.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_linked_node_modules_is_written_only_by_a_program_s_flag() {
    let package_json = r#"{"name": "p", "type": "module", "devDependencies": {"vitest": "*", "@sveltejs/kit": "*"}}"#;
    let scratch = Scratch::new(
        "verb_box_linked_writes",
        &[
            ("tmp/.keep", ""),
            ("project/package.json", package_json),
            ("project/src/probe.ts", "// probed\n"),
            ("other/node_modules/.bin/vitest", FAKE_TOOL),
            ("other/node_modules/.bin/svelte-kit", FAKE_TOOL),
            ("other/node_modules/$app/.keep", ""),
            (
                "other/node_modules/vitest/package.json",
                r#"{"name": "vitest", "version": "1.0.0"}"#,
            ),
            (
                "other/node_modules/@sveltejs/kit/package.json",
                r#"{"name": "@sveltejs/kit", "version": "1.0.0"}"#,
            ),
        ],
    );
    scratch.set_mode("other/node_modules/.bin/vitest", 0o755);
    scratch.set_mode("other/node_modules/.bin/svelte-kit", 0o755);
    let other = scratch.0.join("other/node_modules").canonicalize().unwrap();
    for package in ["@fuzdev/fuz_util", "@fuzdev/fuz_schema", "esm-env"] {
        copy_tree(&checkout_node_modules().join(package), &other.join(package));
    }
    let project = scratch.0.join("project");
    std::os::unix::fs::symlink(&other, project.join("node_modules")).unwrap();
    let run_check = |flag: &str| {
        let mut command = fgro_command(&[
            "check",
            flag,
            "--no-typecheck",
            "--no-lint",
            "--no-format",
            "--no-package_json",
        ]);
        for name in CLEARED {
            command.env_remove(name);
        }
        command
            .current_dir(&project)
            .env("TMPDIR", scratch.0.join("tmp"));
        let (code, stdout, stderr) = run(command);
        let output = format!("{stdout}{stderr}");
        assert_eq!(code, Some(0), "{flag}: {output}");
        let note = stderr
            .lines()
            .find(|line| line.starts_with("note: node_modules is a symlink"))
            .unwrap_or_default()
            .to_owned();
        (output, probed(&stdout, &stderr), note)
    };
    let link = format!(
        "note: node_modules is a symlink to {}, which the box follows to read and execute, and ",
        other.display()
    );
    // gro's own code's flag: no tool directory made there, no tool writing
    let (output, probed, note) = run_check(&format!("--allow-write={}", other.display()));
    for (name, what) in [
        ("vitest_nm", "EACCES"),
        ("vitest_vite", "ENOENT"),
        ("svelte-kit_nm", "EACCES"),
        ("svelte-kit_vite", "ENOENT"),
    ] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    for dir in [".vite", ".vite-temp"] {
        assert!(!other.join(dir).exists(), "{dir}: {output}");
    }
    // the checkout's own `$app`, which no tool wrote
    assert_eq!(
        std::fs::read_dir(other.join("$app")).unwrap().count(),
        1,
        "{output}"
    );
    assert!(!other.join("vitest_w").exists(), "{output}");
    assert_eq!(
        note,
        format!(
            "{link}no tool writes in it: --allow-run-write=<program>:<dirs> naming where it leads \
             lets that tool write there"
        ),
        "{output}"
    );
    // vitest's own flag: vitest writes there, and no other tool
    let (output, probed, note) =
        run_check(&format!("--allow-run-write=vitest:{}", other.display()));
    for (name, what) in [
        ("vitest_nm", "ok"),
        ("vitest_vite", "ENOENT"),
        ("svelte-kit_nm", "EACCES"),
    ] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    assert!(other.join("vitest_w").exists(), "{output}");
    assert!(!other.join("svelte-kit_w").exists(), "{output}");
    assert_eq!(
        note,
        format!("{link}of the tools only vitest writes in it, where --allow-run-write= names"),
        "{output}"
    );
    // svelte-kit's flag on `$app` there, as the linked-worktree spelling
    // names it: its files written, and no symlink made there
    let (output, probed, _) = run_check(&format!(
        "--allow-run-write=svelte-kit:{}",
        other.join("$app").display()
    ));
    for (name, what) in [("svelte-kit_app", "ok"), ("svelte-kit_link_app", "EACCES")] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
}

/// Why a private user and mount namespace can't bind-mount here, by trying
/// it with `unshare`, none when it can: the `AppArmor` policy restricting user
/// namespaces refuses one, as does a sandbox that refuses nested ones.
fn bind_mount_unavailable_reason(scratch: &Scratch) -> Option<String> {
    let (from, to) = (
        scratch.0.join("bind_probe_from"),
        scratch.0.join("bind_probe_to"),
    );
    std::fs::create_dir_all(&from).unwrap();
    std::fs::create_dir_all(&to).unwrap();
    let output = Command::new("unshare")
        .args(["-rm", "mount", "--bind"])
        .args([&from, &to])
        .output();
    match output {
        Ok(output) if output.status.success() => None,
        Ok(output) => Some(format!(
            "this case needs a private user and mount namespace that can bind-mount, and \
             `unshare -rm mount --bind` failed ({}: {})",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Some(format!(
            "this case needs `unshare` to make a private user and mount namespace, and it \
             didn't start ({error})"
        )),
    }
}

/// The project bind-mounted at another path is the project: a `TMPDIR`
/// beneath the mount, which the run's text check of the path misses, is
/// refused by the directory's identity, before anything is made. It runs
/// in a private user and mount namespace, and says why it skips where one
/// can't bind-mount (user namespaces refused).
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_temporary_directory_in_the_project_s_bind_mount_is_refused() {
    let scratch = probe_tree("verb_box_bind");
    if let Some(reason) = bind_mount_unavailable_reason(&scratch) {
        eprintln!("skipped: {reason}");
        return;
    }
    let project = scratch.0.join("project");
    let bind = scratch.0.join("bind");
    std::fs::create_dir(&bind).unwrap();
    let mut wrapper = Command::new("unshare");
    wrapper.args([
        "-rm",
        "sh",
        "-c",
        "mount --bind \"$1\" \"$2\" && shift 2 && exec \"$@\"",
        "sh",
    ]);
    wrapper.args([&project, &bind]);
    let args = probe_args();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = in_tree(support::wrapped_fgro_command(wrapper, &args), &scratch);
    command.env("TMPDIR", bind.join("src"));
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.ends_with(&format!(
            "error: the kernel box makes the run's private directory in the OS's temporary \
             directory, and {} is inside the project, where the run's tools could write what \
             later commands read: point TMPDIR outside the project\n",
            bind.join("src").display()
        )),
        "{stderr}"
    );
    let made: Vec<_> = std::fs::read_dir(project.join("src"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("gro-"))
        .collect();
    assert!(made.is_empty(), "{made:?}");
}

/// A fake tool of the grants table, a `node` script at every table tool's
/// `.bin` name: it writes the paths a program's box might, its own
/// temporary directory among them, and prints what it could.
const FAKE_TOOL: &str = "#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
const me = path.basename(process.argv[1]);
const attempt = (name, act) => {
\ttry {
\t\tact();
\t\tconsole.log(`probe: ${me}_${name} ok`);
\t} catch (error) {
\t\tconsole.log(`probe: ${me}_${name} ${error.code}`);
\t}
};
for (const [name, at] of [
	['out', 'out/w'],
	['vite', 'node_modules/.vite/w'],
	['nm', `node_modules/${me}_w`],
	['src', 'src/w'],
	['kit', `.svelte-kit/${me}_w`],
	['dev', `.svelte-kit/generated/dev/${me}_w`],
	['types', `.svelte-kit/types/${me}_w`],
	['app', `node_modules/$app/${me}_w`],
	['temp', `node_modules/.vite-temp/${me}_w`],
	['cache', `node_modules/.vite/vitest/${me}_w`],
]) {
\tattempt(name, () => fs.writeFileSync(at, me));
}
const own = `${me}_${process.pid}`;
attempt('link_dev', () => fs.symlinkSync('/nowhere', `.svelte-kit/generated/dev/${own}_l`));
attempt('link_out', () => fs.symlinkSync('/nowhere', `out/${own}_l`));
attempt('link_app', () => fs.symlinkSync('/nowhere', `node_modules/$app/${own}_l`));
attempt('link_cache', () => fs.symlinkSync('/nowhere', `node_modules/.vite/vitest/${own}_l`));
attempt('move_out', () => {
\tfs.mkdirSync(`out/${own}_m`);
\tfs.symlinkSync('/nowhere', `out/${own}_m/l`);
\tfs.renameSync(`out/${own}_m`, `.svelte-kit/generated/dev/${own}_m`);
});
attempt('move_tmp', () => {
\tconst made = `${process.env.TMPDIR}/${own}_t`;
\tfs.mkdirSync(made);
\tfs.symlinkSync('/nowhere', `${made}/l`);
\tfs.renameSync(made, `.svelte-kit/generated/dev/${own}_t`);
});
attempt('fifo_dev', () => {
\tconst made = spawnSync('mkfifo', [`.svelte-kit/generated/dev/${own}_f`], { encoding: 'utf8' });
\tif (made.status !== 0) {
\t\tthrow Object.assign(new Error(made.stderr), {
\t\t\tcode: made.stderr.includes('Permission denied') ? 'EACCES' : `status ${made.status}`
\t\t});
\t}
});
console.log(`probe: ${me}_tmpdir ${process.env.TMPDIR}`);
";

/// The genfile of [`each_program_writes_by_its_grant_beside_the_table_s`],
/// `PYDIR` the directory `python3` is found in: a program a flag grants and
/// one found beneath an exec root, each made twice and launched by each
/// handle, write no tool directory, and each keeps one temporary directory
/// across its handles and launches.
const GRANT_GENFILE: &str = "import {fs} from '@fuzdev/gro/fs.ts';
import {spawn} from '@fuzdev/gro/spawn.ts';
const lines = (text) => {
\tfor (const line of text.split('\\n')) if (line.startsWith('probe: ')) console.error(line);
};
const script = 'import os, sys\\n' +
\t'try:\\n    open(\"node_modules/.vite/\" + sys.argv[1], \"w\").write(\"x\")\\n    r = \"ok\"\\n' +
\t'except OSError as e:\\n    r = e.strerror\\n' +
\t'print(\"probe:\", sys.argv[1] + \"_vite\", r)\\nprint(\"probe:\", sys.argv[1] + \"_tmpdir\", os.environ[\"TMPDIR\"])\\n';
export const gen = async () => {
\tconst python = spawn.program('python3');
\tconst found = await spawn.find_in(await fs.open_dir('PYDIR'), 'python3');
\tconst python_again = spawn.program('python3');
\tconst found_again = await spawn.find_in(await fs.open_dir('PYDIR'), 'python3');
\tfor (const [name, program] of [['flag', python], ['flag_again', python_again], ['found', found], ['found_again', found_again]]) {
\t\tlines((await program.out(['-I', '-c', script, name])).stdout);
\t}
\treturn '// probed\\n';
};
";

/// Each program writes by its own grant beside the grants table's tools:
/// `vitest`, named by its plain name by `--allow-run-write=`, writes its
/// directory and its own tool directories, `svelte-kit`, a table tool no
/// flag names, its own and not that directory, eslint, one that names
/// none, no tool directory, none writes another's or `src`, no tool
/// writes `node_modules/.vite` beside vitest's own cache, and `$app` is
/// `svelte-kit`'s alone; no tool makes a symlink or a FIFO in a tool
/// directory, even its own, nor moves a directory in from the one its flag
/// names or its own temporary directory, where vitest still makes a symlink
/// in the directory its flag names, unless that directory is in a tool
/// directory;
/// a program a flag grants and one found beneath an exec root write no
/// tool directory, and each program keeps one temporary directory across
/// its handles and launches, apart from every other program's.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn each_program_writes_by_its_grant_beside_the_table_s() {
    let python_dir = python3().parent().unwrap().to_owned();
    let genfile = GRANT_GENFILE.replace("PYDIR", &python_dir.display().to_string());
    let package_json = r#"{"name": "p", "type": "module", "devDependencies": {"vitest": "*", "@sveltejs/kit": "*", "eslint": "*"}}"#;
    let scratch = Scratch::new(
        "verb_box_grant_writes",
        &[
            ("tmp/.keep", ""),
            ("project/package.json", package_json),
            ("project/src/probe.gen.ts", &genfile),
            ("project/src/probe.ts", "// probed\n"),
            ("project/out/.keep", ""),
            ("project/node_modules/.bin/vitest", FAKE_TOOL),
            ("project/node_modules/.bin/svelte-kit", FAKE_TOOL),
            ("project/node_modules/.bin/eslint", FAKE_TOOL),
            (
                "project/node_modules/vitest/package.json",
                r#"{"name": "vitest", "version": "1.0.0"}"#,
            ),
            (
                "project/node_modules/@sveltejs/kit/package.json",
                r#"{"name": "@sveltejs/kit", "version": "1.0.0"}"#,
            ),
        ],
    );
    scratch.set_mode("project/node_modules/.bin/vitest", 0o755);
    scratch.set_mode("project/node_modules/.bin/svelte-kit", 0o755);
    scratch.set_mode("project/node_modules/.bin/eslint", 0o755);
    let project = scratch.0.join("project");
    for package in ["@fuzdev/fuz_util", "@fuzdev/fuz_schema", "esm-env"] {
        copy_tree(
            &checkout_node_modules().join(package),
            &project.join("node_modules").join(package),
        );
    }
    let run_in = format!("--allow-run-in={}", python_dir.display());
    let read = format!("--allow-read={}", python_dir.display());
    let mut command = fgro_command(&[
        "check",
        "--allow-run=python3",
        "--allow-run-write=vitest:out",
        &run_in,
        &read,
        "--no-typecheck",
        "--no-format",
        "--no-package_json",
        "--lint",
    ]);
    for name in CLEARED {
        command.env_remove(name);
    }
    command
        .current_dir(&project)
        .env("TMPDIR", scratch.0.join("tmp"));
    let (code, stdout, stderr) = run(command);
    let output = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0), "{output}");
    let probed = probed(&stdout, &stderr);
    for (name, what) in [
        ("vitest_out", "ok"),
        ("vitest_vite", "EACCES"),
        ("vitest_src", "EACCES"),
        ("vitest_kit", "EACCES"),
        ("vitest_dev", "ok"),
        ("vitest_types", "ok"),
        ("vitest_app", "EACCES"),
        ("vitest_link_dev", "EACCES"),
        ("vitest_link_out", "ok"),
        ("vitest_move_out", "EXDEV"),
        ("vitest_move_tmp", "EXDEV"),
        ("svelte-kit_move_tmp", "EXDEV"),
        ("vitest_fifo_dev", "EACCES"),
        ("svelte-kit_fifo_dev", "EACCES"),
        ("svelte-kit_link_dev", "EACCES"),
        ("eslint_link_dev", "EACCES"),
        ("vitest_temp", "ok"),
        ("vitest_cache", "ok"),
        ("svelte-kit_out", "EACCES"),
        ("svelte-kit_vite", "EACCES"),
        ("svelte-kit_src", "EACCES"),
        ("svelte-kit_kit", "ok"),
        ("svelte-kit_app", "ok"),
        ("svelte-kit_temp", "ok"),
        ("svelte-kit_cache", "EACCES"),
        ("eslint_out", "EACCES"),
        ("eslint_kit", "EACCES"),
        ("eslint_dev", "EACCES"),
        ("eslint_app", "EACCES"),
        ("eslint_temp", "EACCES"),
        ("eslint_cache", "EACCES"),
        ("flag_vite", "Permission denied"),
        ("found_vite", "Permission denied"),
    ] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    assert_eq!(
        probed["flag_tmpdir"], probed["flag_again_tmpdir"],
        "{output}"
    );
    assert_eq!(
        probed["found_tmpdir"], probed["found_again_tmpdir"],
        "{output}"
    );
    let temps: BTreeSet<&str> = [
        "flag_tmpdir",
        "found_tmpdir",
        "vitest_tmpdir",
        "svelte-kit_tmpdir",
        "eslint_tmpdir",
    ]
    .iter()
    .map(|name| probed[*name].as_str())
    .collect();
    assert_eq!(temps.len(), 5, "{output}");
    // only what some program of the command writes is made, and nothing
    // beside vitest's cache in `node_modules/.vite`
    for (dir, made) in [
        (".svelte-kit/generated/dev", true),
        ("node_modules/.vite/vitest", true),
        ("node_modules/.vite/deps", false),
        (".fuz", false),
    ] {
        assert_eq!(project.join(dir).is_dir(), made, "{dir}: {output}");
    }
    // a flag naming a tool directory gives no symlink, FIFO, or entry moved
    // in there, though its writes, the directory named once in the note
    let mut command = fgro_command(&["test", "--allow-run-write=vitest:.svelte-kit/generated/dev"]);
    for name in CLEARED {
        command.env_remove(name);
    }
    command
        .current_dir(&project)
        .env("TMPDIR", scratch.0.join("tmp"));
    let (code, stdout, stderr) = run(command);
    let output = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0), "{output}");
    let probed = crate::probed(&stdout, &stderr);
    for (name, what) in [
        ("vitest_dev", "ok"),
        ("vitest_link_dev", "EACCES"),
        ("vitest_fifo_dev", "EACCES"),
        ("vitest_move_tmp", "EXDEV"),
    ] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    // a flag on a directory holding a tool directory, and lying in none,
    // writes it plain too, so the special rights don't come back inside the
    // tool directory by Landlock's union along the path
    let mut command = fgro_command(&["test", "--allow-run-write=vitest:node_modules"]);
    for name in CLEARED {
        command.env_remove(name);
    }
    command
        .current_dir(&project)
        .env("TMPDIR", scratch.0.join("tmp"));
    let (code, stdout, stderr) = run(command);
    let output = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0), "{output}");
    let probed = crate::probed(&stdout, &stderr);
    for (name, what) in [
        ("vitest_cache", "ok"),
        ("vitest_nm", "ok"),
        ("vitest_link_cache", "EACCES"),
        ("vitest_link_app", "EACCES"),
    ] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
}

/// The genfile of [`a_write_flag_on_a_tool_directory_writes_it_plain`]:
/// gro's own code makes a symlink in a tool directory `--allow-write=`
/// names, and in a directory it names elsewhere, `VICTIM` the link's target.
const PLAIN_GENFILE: &str = "import {fs} from '@fuzdev/gro/fs.ts';
const attempt = async (name, dir) => {
\tlet result;
\ttry {
\t\tconst opened = await fs.open_dir(`${fs.project.path}/${dir}`);
\t\tresult = await opened.symlink('realm_l', 'VICTIM');
\t} catch (error) {
\t\tresult = String(error?.kind ?? error?.code ?? error);
\t}
\tconsole.error(`probe: ${name} ${result}`);
};
export const gen = async () => {
\tawait attempt('realm_link_dev', '.svelte-kit/generated/dev');
\tawait attempt('realm_link_out', 'out');
\treturn '// probed\\n';
};
";

/// `--allow-write=` on a tool directory writes it plain, as
/// `--allow-run-write=` does: a genfile under `fgro gen` makes no symlink
/// there, where it still makes one in a directory the flag names elsewhere.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_write_flag_on_a_tool_directory_writes_it_plain() {
    let package_json =
        r#"{"name": "p", "type": "module", "devDependencies": {"@sveltejs/kit": "*"}}"#;
    let scratch = Scratch::new(
        "verb_box_flag_plain",
        &[
            ("tmp/.keep", ""),
            ("victim.txt", "kept\n"),
            ("project/package.json", package_json),
            ("project/out/.keep", ""),
            ("project/.svelte-kit/generated/dev/.keep", ""),
            ("project/.svelte-kit/types/route_meta_data.json", "{}"),
            ("project/node_modules/$app/tsconfig.json", "{}"),
            ("project/node_modules/.bin/svelte-kit", FAKE_TOOL),
            (
                "project/node_modules/@sveltejs/kit/package.json",
                r#"{"name": "@sveltejs/kit", "version": "1.0.0"}"#,
            ),
        ],
    );
    scratch.set_mode("project/node_modules/.bin/svelte-kit", 0o755);
    let victim = scratch.0.join("victim.txt");
    std::fs::create_dir_all(scratch.0.join("project/src")).unwrap();
    std::fs::write(
        scratch.0.join("project/src/probe.gen.ts"),
        PLAIN_GENFILE.replace("VICTIM", &victim.display().to_string()),
    )
    .unwrap();
    let project = scratch.0.join("project");
    for package in ["@fuzdev/fuz_util", "@fuzdev/fuz_schema", "esm-env"] {
        copy_tree(
            &checkout_node_modules().join(package),
            &project.join("node_modules").join(package),
        );
    }
    let mut command = fgro_command(&[
        "gen",
        "--allow-write=.svelte-kit/generated/dev",
        "--allow-write=out",
    ]);
    for name in CLEARED {
        command.env_remove(name);
    }
    command
        .current_dir(&project)
        .env("TMPDIR", scratch.0.join("tmp"));
    let (code, stdout, stderr) = run(command);
    let output = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0), "{output}");
    let probed = probed(&stdout, &stderr);
    for (name, what) in [
        ("realm_link_dev", "permission_denied"),
        ("realm_link_out", "created"),
    ] {
        assert_eq!(
            probed.get(name).map(String::as_str),
            Some(what),
            "{name}: {output}"
        );
    }
    assert!(
        !project.join(".svelte-kit/generated/dev/realm_l").exists(),
        "{output}"
    );
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "kept\n");
}
