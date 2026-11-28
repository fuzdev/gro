//! `fgro run` in the run's kernel box, end to end: what a script and the
//! programs it's granted reach from inside it (a secret in the home
//! directory, `.git`, a new top-level entry, a program's own directories and
//! another program's temporary one, git's home files), where `fs.temp_dir`
//! makes its directories, an entry with no project's `package.json` at or
//! above it (its type marker read, its directory granted to its code and
//! its programs only by a flag, a world-writable one that is the temporary
//! directory, its import graph read before the box and served from those
//! reads, a module outside it refused, a package a project nested there
//! links from outside refused, a `node_modules` beside it read by nothing),
//! a project root that is or holds the home directory, the run's network, a
//! working directory below the root, and where the box can't go up: below
//! the kernel's floor, a socket on stdin, and a build with no snapshot.
//!
//! `fgro run` runs in its box only from a snapshot build, so these run from
//! `cargo xtask build --verify` and are ignored in a plain `cargo test`, but
//! for the plain build's own case and those decided before the box (an
//! entry's own directory, a root holding the home directory), which run in
//! both. They need Landlock ABI 6 and `python3`.

#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

mod support;

use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use support::{KillOnDrop, Scratch, UNBOXED_NOTE, fgro_command, spawn};

/// The variables the run and its box read, cleared so the harness's own
/// don't leak in.
const CLEARED: [&str; 9] = [
    "GRO_FALLBACK",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_CONFIG_HOME",
    "GIT_CONFIG_GLOBAL",
];

/// `landlock_create_ruleset`'s call number on `x86_64`, and `ENOSYS`.
const LANDLOCK_CREATE_RULESET: u32 = 444;
const ENOSYS: u32 = 38;

/// A seccomp filter `python3` installs before it execs the rest of its
/// arguments, answering the call `argv[1]` with the error `argv[2]`.
const FILTER: &str = r#"
import ctypes, os, struct, sys
number, error = int(sys.argv[1]), int(sys.argv[2])
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
os.execv(sys.argv[3], sys.argv[3:])
"#;

/// A project under the target's temporary directory, its `package.json`
/// beside `files`, with a home holding a secret and git's config, and a
/// temporary directory of its own beside it.
fn project(name: &str, files: &[(&str, &str)]) -> Scratch {
    let mut all = vec![
        ("project/package.json", "{\"name\": \"p\"}\n"),
        ("home/secret.txt", "secret"),
        ("home/.gitconfig", "[user]\n\tname = boxed\n"),
        ("tmp/.keep", ""),
    ];
    all.extend_from_slice(files);
    Scratch::new(name, &all)
}

/// `fgro` with `args` in `dir`, its home and temporary directory
/// `scratch`'s, the variables the box reads cleared.
fn in_scratch(scratch: &Scratch, dir: &Path, args: &[&str]) -> Command {
    let mut command = fgro_command(args);
    in_env(&mut command, scratch, dir);
    command
}

/// `command` given `scratch`'s home and temporary directory, in `dir`.
fn in_env(command: &mut Command, scratch: &Scratch, dir: &Path) {
    for name in CLEARED {
        command.env_remove(name);
    }
    command
        .current_dir(dir)
        .env("HOME", scratch.0.join("home"))
        .env("TMPDIR", scratch.0.join("tmp"));
}

/// The run of `command`: its code, stdout, and stderr.
fn run(command: Command) -> (Option<i32>, String, String) {
    let output = spawn(command, b"");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// `python3` on this process's `PATH`, by its path.
fn python3() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("python3"))
        .find(|path| path.is_file())
        .unwrap()
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

/// The private directories under `scratch`'s temporary directory.
fn private_dirs(scratch: &Scratch) -> Vec<String> {
    let mut made: Vec<String> = std::fs::read_dir(scratch.0.join("tmp"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("gro-"))
        .collect();
    made.sort();
    made
}

/// What a write the box refuses says to `fgro run`'s code, where it writes
/// the project's root by a flag naming it, and starts a program, which
/// gives it a temporary directory of its own.
const WRITE_CAUSE: &str = "the run's kernel box lets fgro run write only to the top-level \
                           entries the project had as it started (but .git, node_modules, the \
                           dot-entries, symlinks) and a temporary directory of the run's; .git, \
                           node_modules, the other dot-entries, and a new top-level entry stay \
                           read-only";

/// From inside the box, a program the run starts reads nothing of the
/// home directory, holds the run's seccomp filter on top of its launcher's,
/// and the run's own code, granted the root's writes by `--allow-write=.`,
/// writes its top-level entries but neither `.git` nor a new one, each
/// refusal naming where the box lets it write.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn the_walls_hold_from_inside_a_run() {
    let scratch = project(
        "run_boxed_walls",
        &[
            ("project/.git/HEAD", "ref: refs/heads/main\n"),
            ("project/src/a.txt", "a"),
            (
                "project/main.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';
import {spawn} from '@fuzdev/gro/spawn.ts';
const cat = await spawn.program('cat').out([`${fs.project.path}/../home/secret.txt`]);
console.log('cat', cat.code, cat.stdout === '' && cat.stderr.includes('Permission denied'));
const status = await spawn.program('python3').out(['-I', '-c', 'print([l for l in open(\"/proc/self/status\") if l.startswith(\"Seccomp_filters:\")][0].split()[1])']);
console.log('filters', status.stdout.trim());
for (const [name, path] of [['entry', 'src/b.txt'], ['git', '.git/hooks_x'], ['top', 'new.txt']]) {
\ttry {
\t\tawait fs.project.write(path, 'x');
\t\tconsole.log(name, 'ok');
\t} catch (error) {
\t\tconsole.log(name, error.kind, error.cause?.message ?? '');
\t}
}
",
            ),
        ],
    );
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &scratch.0.join("project"),
        &[
            "run",
            "--allow-run=cat,python3",
            "--allow-write=.",
            "main.mjs",
        ],
    ));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "cat 1 true", "{stdout}{stderr}");
    assert_eq!(
        lines[1],
        format!("filters {}", own_filters() + 1),
        "{stdout}"
    );
    assert_eq!(lines[2], "entry ok", "{stdout}");
    for (line, name) in [(lines[3], "git"), (lines[4], "top")] {
        assert!(
            line.starts_with(&format!("{name} permission_denied ")) && line.contains(WRITE_CAUSE),
            "{line}"
        );
    }
}

/// A program the run grants writes its own temporary directory and what
/// `--allow-run-write=` names for it, nothing else, and reads no other
/// program's temporary directory; the run's own code doesn't write what
/// the flag names for the program.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn a_program_writes_only_its_own_directories() {
    let scratch = project(
        "run_boxed_programs",
        &[
            ("project/out/.keep", ""),
            ("project/elsewhere/.keep", ""),
            (
                "project/main.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';
import {spawn} from '@fuzdev/gro/spawn.ts';
const sh = await spawn.program('sh').out(['-c', 'echo a > out/a && echo ok; echo b > elsewhere/b || echo refused; echo t > \"$TMPDIR/t\" && echo \"$TMPDIR/t\"']);
const [out, elsewhere, made] = sh.stdout.trim().split('\\n');
console.log('out', out, 'elsewhere', elsewhere);
const cat = await spawn.program('cat').out([made]);
console.log('sibling', cat.code, cat.stderr.includes('Permission denied'));
try {
\tawait fs.project.write('out/realm', 'x');
\tconsole.log('realm ok');
} catch (error) {
\tconsole.log('realm', error.kind);
}
",
            ),
        ],
    );
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &scratch.0.join("project"),
        &[
            "run",
            "--allow-run=sh,cat",
            "--allow-run-write=sh:out",
            "main.mjs",
        ],
    ));
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert_eq!(
        stdout, "out ok elsewhere refused\nsibling 1 true\nrealm not_granted\n",
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("project/out/a")).unwrap(),
        "a\n"
    );
    assert!(!scratch.0.join("project/elsewhere/b").exists());
}

/// `fs.temp_dir` makes its directories in the run's private directory,
/// which `TMPDIR` names to the run; a run that makes one sweeps the
/// private directories of runs gone, and one that makes none sweeps
/// nothing.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn fs_temp_dir_is_in_the_run_s_private_directory() {
    let scratch = project(
        "run_boxed_temp",
        &[
            // a run gone: no process has this id
            ("tmp/gro-4194304-1/left.txt", ""),
            (
                "project/main.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';
import {env} from '@fuzdev/gro/env.ts';
await using made = await fs.temp_dir('t-');
console.log(made.path);
console.log(env.get('TMPDIR'));
",
            ),
            ("project/none.mjs", "console.log('none');\n"),
        ],
    );
    let dir = scratch.0.join("project");
    let (code, stdout, stderr) = run(in_scratch(&scratch, &dir, &["run", "none.mjs"]));
    assert_eq!((code, stdout.as_str()), (Some(0), "none\n"), "{stderr}");
    assert_eq!(private_dirs(&scratch), ["gro-4194304-1"]);
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &dir,
        &["run", "--allow-temp", "--allow-env=TMPDIR", "main.mjs"],
    ));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    let private = Path::new(lines[1]);
    assert_eq!(
        private.parent(),
        Some(scratch.0.join("tmp").as_path()),
        "{stdout}"
    );
    assert!(
        private
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("gro-"),
        "{stdout}"
    );
    assert_eq!(Path::new(lines[0]).parent(), Some(private), "{stdout}");
    assert!(
        Path::new(lines[0])
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("t-")
    );
    // the run gone's swept, the run's own left for the next sweep
    assert!(!private_dirs(&scratch).contains(&"gro-4194304-1".to_owned()));
}

/// A `.js` module's type marker above an entry with no project's
/// `package.json` at or above it, read before the box with the entry's
/// import graph, is served to each module's load inside it.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn a_type_marker_above_an_entry_with_no_project_is_read() {
    let scratch = Scratch::in_system_temp(
        "run_boxed_marker",
        &[
            ("marker/package.json", "{\"type\": \"module\"}\n"),
            (
                "marker/b/x.js",
                "import {y} from './y.js';\nconsole.log('ok', y);\n",
            ),
            ("marker/b/y.js", "export const y = 1;\n"),
            ("home/.keep", ""),
            ("tmp/.keep", ""),
        ],
    );
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &scratch.0.join("marker"),
        &["run", "b/x.js"],
    ));
    assert_eq!((code, stdout.as_str()), (Some(0), "ok 1\n"), "{stderr}");
}

/// An entry with no project's `package.json` at or above it, whose root is
/// its own directory, reads and writes there only as a flag naming it
/// grants, a refusal hinting that flag: `.` from the directory, else its
/// path; its imports load all the same.
#[test]
fn an_entry_with_no_project_reads_only_what_a_flag_names() {
    let scratch = Scratch::in_system_temp(
        "run_boxed_loose",
        &[
            (
                "loose/read.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';\nimport {data} from './lib.mjs';\n\
                 console.log(data, await fs.project.read_text('data.txt'));\n",
            ),
            ("loose/lib.mjs", "export const data = 'lib';\n"),
            (
                "loose/write.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';\nawait fs.project.write('o.txt', 'x');\n",
            ),
            ("loose/data.txt", "data"),
            ("home/.keep", ""),
            ("tmp/.keep", ""),
        ],
    );
    let loose = scratch.0.join("loose");
    let hint = |flag: &str, value: &str| {
        let (verb, rest) = if flag == "--allow-write" {
            (
                "writes",
                "; in the run's kernel box a new entry at its top level stays read-only, so \
                 write in a directory already there",
            )
        } else {
            ("reads", "")
        };
        format!(
            "hint: {} is in no project (no package.json at or above the entry holds more than \
             \"type\" or \"sideEffects\"), so the run {verb} it only as a flag grants: add \
             {flag}={value} to grant what fs refused{rest}\n",
            loose.display()
        )
    };
    let (code, stdout, stderr) = run(in_scratch(&scratch, &loose, &["run", "read.mjs"]));
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.ends_with(&hint("--allow-read", ".")), "{stderr}");
    let (code, _, stderr) = run(in_scratch(&scratch, &scratch.0, &["run", "loose/read.mjs"]));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.ends_with(&hint("--allow-read", &loose.display().to_string())),
        "{stderr}"
    );
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &loose,
        &["run", "--allow-read=.", "read.mjs"],
    ));
    assert_eq!((code, stdout.as_str()), (Some(0), "lib data\n"), "{stderr}");
    let (code, _, stderr) = run(in_scratch(&scratch, &loose, &["run", "write.mjs"]));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.ends_with(&hint("--allow-write", ".")), "{stderr}");
}

/// An entry with no project in a world-writable directory that is the
/// temporary directory makes the run's private directory there, which no
/// project holds; a program the run starts there reads nothing of that
/// directory, another run's files among it, so the run isn't refused, but
/// a read flag naming the directory, which every program reads, holds the
/// private directory and is.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn an_entry_in_the_temporary_directory_makes_its_private_directory_there() {
    let scratch = Scratch::in_system_temp(
        "run_boxed_shared",
        &[
            (
                "shared/t.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';\nawait using made = await \
                 fs.temp_dir();\nconsole.log('made');\n",
            ),
            (
                "shared/cat.mjs",
                "import {spawn} from '@fuzdev/gro/spawn.ts';\n\
                 const cat = await spawn.program('cat').out(['other/token']);\n\
                 console.log(cat.code, cat.stdout, cat.stderr.includes('Permission denied'));\n",
            ),
            ("shared/other/token", "token"),
            ("home/.keep", ""),
        ],
    );
    scratch.set_mode("shared", 0o1777);
    let shared = scratch.0.join("shared");
    let in_shared = |args: &[&str]| {
        let mut command = in_scratch(&scratch, &shared, args);
        command.env("TMPDIR", &shared);
        run(command)
    };
    let (code, stdout, stderr) = in_shared(&["run", "--allow-temp", "t.mjs"]);
    assert_eq!((code, stdout.as_str()), (Some(0), "made\n"), "{stderr}");
    let (code, stdout, stderr) = in_shared(&["run", "--allow-temp", "--allow-run=cat", "cat.mjs"]);
    assert_eq!((code, stdout.as_str()), (Some(0), "1  true\n"), "{stderr}");
    let (code, stdout, stderr) =
        in_shared(&["run", "--allow-read=.", "--allow-run=cat", "cat.mjs"]);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    let shown = shared.display();
    assert_eq!(
        stderr,
        format!(
            "error: the kernel box makes the run's private directory in the OS's temporary \
             directory, {shown}, and --allow-read={shown} holds it, so every program the run \
             starts could read the others' temporary directories there: narrow --allow-read \
             to a directory that doesn't hold {shown}, or point TMPDIR elsewhere\n"
        )
    );
    scratch.set_mode("shared", 0o755);
}

/// An entry with no project directly in the home directory loads its
/// sibling modules, but a program it starts reads nothing of that
/// directory, its own working directory, unless a read flag names it.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn a_program_reads_an_entry_in_no_project_s_directory_only_as_a_flag_names_it() {
    let scratch = Scratch::in_system_temp(
        "run_boxed_loose_home",
        &[
            (
                "home/x.mjs",
                "import {spawn} from '@fuzdev/gro/spawn.ts';\nimport {lib} from './lib.mjs';\n\
                 const cat = await spawn.program('cat').out(['.ssh/id']);\n\
                 const ls = await spawn.program('ls').out(['.']);\n\
                 console.log(JSON.stringify([lib, cat.code, cat.stdout, ls.code, \
                 ls.stdout.includes('lib.mjs')]));\n",
            ),
            ("home/lib.mjs", "export const lib = 'lib';\n"),
            ("home/.ssh/id", "secret"),
            ("tmp/.keep", ""),
        ],
    );
    let home = scratch.0.join("home");
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &home,
        &["run", "--allow-run=cat,ls", "x.mjs"],
    ));
    assert_eq!(
        (code, stdout.as_str()),
        (Some(0), "[\"lib\",1,\"\",2,false]\n"),
        "{stderr}"
    );
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &home,
        &["run", "--allow-run=cat,ls", "--allow-read=.", "x.mjs"],
    ));
    assert_eq!(
        (code, stdout.as_str()),
        (Some(0), "[\"lib\",0,\"secret\",0,true]\n"),
        "{stderr}"
    );
}

/// A project root found at or above the entry that is the home directory,
/// or holds it, is refused before anything runs, naming what made it the
/// root, a `package.json` or a `node_modules`; `-A` runs it.
#[test]
fn a_root_that_is_or_holds_the_home_directory_is_refused() {
    let scratch = Scratch::in_system_temp(
        "run_boxed_home",
        &[
            ("home/package.json", "{\"name\": \"oops\"}\n"),
            ("home/dev/x.mjs", "console.log('ran');\n"),
            (
                "home/node_modules/pkg/package.json",
                "{\"name\": \"pkg\"}\n",
            ),
            ("home/node_modules/pkg/x.mjs", "console.log('ran');\n"),
            ("tmp/.keep", ""),
        ],
    );
    let home = scratch.0.join("home");
    let shown = home.display();
    let at = |home_dir: &Path, args: &[&str]| {
        let mut command = in_scratch(&scratch, &scratch.0, args);
        command.env("HOME", home_dir);
        run(command)
    };
    let (code, _, stderr) = at(&home, &["run", "home/dev/x.mjs"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "error: fgro run's project root, {shown}, is your home directory, since \
             {shown}/package.json is the first package.json at or above the entry, and the run \
             reads its root whole: remove that package.json, or give the entry a project of its \
             own (a package.json beside it)\n"
        )
    );
    let (code, _, stderr) = at(&home.join("dev"), &["run", "home/dev/x.mjs"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.starts_with(&format!(
            "error: fgro run's project root, {shown}, holds your home directory"
        )),
        "{stderr}"
    );
    let (code, _, stderr) = at(&home, &["run", "home/node_modules/pkg/x.mjs"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "since the entry is in {shown}/node_modules, whose directory is its root, and the \
             run reads its root whole: run it from a project of its own\n"
        )),
        "{stderr}"
    );
    let (code, stdout, stderr) = at(&home, &["run", "-A", "home/dev/x.mjs"]);
    assert_eq!(
        (code, stdout.as_str(), stderr.as_str()),
        (Some(0), "ran\n", "")
    );
}

/// Below the box's Landlock floor `fgro run` runs in no box after a note
/// saying so, its programs' clause where it may start one; under
/// `GRO_FALLBACK=command` it's refused, and a `GRO_FALLBACK` naming no
/// policy is refused wherever the box goes up.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn below_the_floor_it_runs_in_no_box_unless_the_policy_refuses() {
    let scratch = project(
        "run_boxed_floor",
        &[("project/main.mjs", "console.log('ran');\n")],
    );
    let dir = scratch.0.join("project");
    let below = |args: &[&str], policy: Option<&str>| {
        let mut command = Command::new(python3());
        command
            .args(["-I", "-c", FILTER])
            .args([LANDLOCK_CREATE_RULESET.to_string(), ENOSYS.to_string()]);
        let mut command = support::wrapped_fgro_command(command, args);
        in_env(&mut command, &scratch, &dir);
        if let Some(policy) = policy {
            command.env("GRO_FALLBACK", policy);
        }
        run(command)
    };
    let why = "Landlock isn't available here (the kernel has none, or a seccomp filter refuses \
               its calls), which the kernel box needs at ABI 6 (Linux 6.12) or later";
    let (code, stdout, stderr) = below(&["run", "main.mjs"], None);
    assert_eq!((code, stdout.as_str()), (Some(0), "ran\n"), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "note: fgro run runs in no kernel box here, so only the runtime's own checks hold \
             its grants: {why}\n"
        )
    );
    let (code, _, stderr) = below(&["run", "--allow-run=true", "main.mjs"], Some("all"));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "note: fgro run runs in no kernel box here, so only the runtime's own checks hold \
             its grants, and the programs it starts run in no box of their own, writing \
             wherever you can: {why}\n"
        )
    );
    let (code, stdout, stderr) = below(&["run", "main.mjs"], Some("command"));
    assert_eq!((code, stdout.as_str()), (Some(2), ""), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "error: GRO_FALLBACK=command refuses running fgro run in no kernel box, where only \
             the runtime's own checks hold its grants: {why}\n"
        )
    );
    let mut command = in_scratch(&scratch, &dir, &["run", "main.mjs"]);
    command.env("GRO_FALLBACK", "Command");
    let (code, _, stderr) = run(command);
    assert_eq!(code, Some(2), "{stderr}");
    assert_eq!(
        stderr,
        "error: GRO_FALLBACK is \"Command\", and it takes all or command\n"
    );
    // `-A` reads no policy, and runs in no box, saying nothing
    let mut command = in_scratch(&scratch, &dir, &["run", "-A", "main.mjs"]);
    command.env("GRO_FALLBACK", "Command");
    let (code, stdout, stderr) = run(command);
    assert_eq!(
        (code, stdout.as_str(), stderr.as_str()),
        (Some(0), "ran\n", "")
    );
}

/// A socket on stdin that isn't a connected unix stream is refused before
/// anything changes.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn a_socket_on_stdin_is_refused() {
    let scratch = project(
        "run_boxed_stdin",
        &[("project/main.mjs", "console.log('ran');\n")],
    );
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut command = in_scratch(&scratch, &scratch.0.join("project"), &["run", "main.mjs"]);
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
}

/// From a working directory below the project's root, the grant flags'
/// paths, a relative `TMPDIR`, and the cwd host module are the working
/// directory's, while a pinned name holding `/`, the temporary directory's
/// containment, and a write flag's place against the root are the root's;
/// a flag through a symlink into `.git` is noted, not refused, as the
/// command line's own; the same through a symlink to the working
/// directory.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn below_the_root_the_cwd_and_the_root_each_hold_their_own() {
    let scratch = project(
        "run_boxed_cwd",
        &[
            ("a/f.txt", "outside"),
            ("project/.git/HEAD", "ref: refs/heads/main\n"),
            ("project/src/x.txt", "x"),
            ("project/bin/t", "#!/bin/sh\necho root\n"),
            ("project/sub/bin/t", "#!/bin/sh\necho sub\n"),
            (
                "project/sub/read.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';\nimport {cwd} from '@fuzdev/gro/cwd.ts';\n\
                 const a = await fs.open_dir(`${fs.project.path}/../a`);\n\
                 console.log(cwd(), await a.read_text('f.txt'));\n",
            ),
            (
                "project/sub/pin.mjs",
                "import {spawn} from '@fuzdev/gro/spawn.ts';\n\
                 console.log((await spawn.program('./bin/t').out([])).stdout.trim());\n",
            ),
            (
                "project/sub/write.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.write('src/y.txt', 'y');\n\
                 try { await fs.project.write('new.txt', 'n'); console.log('new ok'); } \
                 catch (error) { console.log('new', error.kind); }\n",
            ),
        ],
    );
    scratch.make_executable("project/bin/t");
    scratch.make_executable("project/sub/bin/t");
    let project = scratch.0.join("project");
    let sub = project.join("sub");
    std::os::unix::fs::symlink(&project, scratch.0.join("alias")).unwrap();
    for dir in [sub.clone(), scratch.0.join("alias/sub")] {
        let (code, stdout, stderr) = run(in_scratch(
            &scratch,
            &dir,
            &["run", "--allow-read=../../a", "read.mjs"],
        ));
        assert_eq!(code, Some(0), "{stdout}{stderr}");
        assert_eq!(stdout, format!("{} outside\n", sub.display()), "{stderr}");
    }
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &sub,
        &["run", "--allow-run=./bin/t", "pin.mjs"],
    ));
    assert_eq!((code, stdout.as_str()), (Some(0), "root\n"), "{stderr}");
    // `tmp` beneath the working directory, and `../tmp2` beneath the root
    // and not the working directory, are each inside the project
    for (temp, at) in [("tmp", sub.join("tmp")), ("../tmp2", project.join("tmp2"))] {
        std::fs::create_dir_all(&at).unwrap();
        let mut command = in_scratch(&scratch, &sub, &["run", "--allow-temp", "read.mjs"]);
        command.env("TMPDIR", temp);
        let (code, _, stderr) = run(command);
        assert_eq!(code, Some(2), "{temp}: {stderr}");
        assert_eq!(
            stderr,
            format!(
                "error: the kernel box makes the run's private directory in the OS's \
                 temporary directory, and {} is inside the project, where the run could write \
                 what later commands read: point TMPDIR outside the project\n",
                at.display()
            ),
            "{temp}"
        );
    }
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &sub,
        &["run", "--allow-write=..", "write.mjs"],
    ));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert_eq!(stdout, "new permission_denied\n", "{stderr}");
    assert_eq!(
        std::fs::read_to_string(project.join("src/y.txt")).unwrap(),
        "y"
    );
    let (code, _, stderr) = run(in_scratch(
        &scratch,
        &sub,
        &["run", "--allow-write=../..", "write.mjs"],
    ));
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.starts_with("error: --allow-write=../.. names "),
        "{stderr}"
    );
    std::os::unix::fs::symlink("../.git", sub.join("l")).unwrap();
    let (code, _, stderr) = run(in_scratch(
        &scratch,
        &sub,
        &["run", "--allow-read=l,../../a", "read.mjs"],
    ));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stderr,
        format!(
            "note: --allow-read=l grants {}\n",
            project.join(".git").display()
        )
    );
}

/// Git's home config is in the box only for a run that grants `git`: git
/// reads it, and any other program it grants is refused it.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn git_s_home_files_are_read_only_for_git() {
    let scratch = project(
        "run_boxed_git",
        &[
            (
                "project/git.mjs",
                "import {spawn} from '@fuzdev/gro/spawn.ts';\n\
                 console.log((await spawn.program('git').out(['config', '--global', \
                 'user.name'])).stdout.trim());\n",
            ),
            (
                "project/cat.mjs",
                "import {spawn} from '@fuzdev/gro/spawn.ts';\n\
                 const cat = await spawn.program('cat').out(['../home/.gitconfig']);\n\
                 console.log(cat.code, cat.stderr.includes('Permission denied'));\n",
            ),
        ],
    );
    let dir = scratch.0.join("project");
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &dir,
        &["run", "--allow-run=git", "git.mjs"],
    ));
    assert_eq!((code, stdout.as_str()), (Some(0), "boxed\n"), "{stderr}");
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &dir,
        &["run", "--allow-run=cat", "cat.mjs"],
    ));
    assert_eq!((code, stdout.as_str()), (Some(0), "1 true\n"), "{stderr}");
}

/// A run that starts no program stays in the host's network, saying
/// nothing; one that may start one takes a namespace of its own where the
/// host allows it, saying nothing, and otherwise stays in the host's
/// network, saying only that.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn only_a_run_that_may_start_a_program_takes_a_network_namespace() {
    let scratch = project(
        "run_boxed_network",
        &[(
            "project/wait.mjs",
            "console.log('ready');\nawait new Promise((resolve) => setTimeout(resolve, 3000));\n",
        )],
    );
    // a process the box marks not dumpable keeps its namespace links from
    // others, so its network is told by the interfaces it sees
    let interfaces = |pid: &str| {
        let mut names: Vec<String> = std::fs::read_to_string(format!("/proc/{pid}/net/dev"))
            .unwrap()
            .lines()
            .skip(2)
            .filter_map(|line| line.split(':').next())
            .map(|name| name.trim().to_owned())
            .collect();
        names.sort();
        names
    };
    let own = interfaces("self");
    for (args, own_network) in [
        (&["run", "wait.mjs"][..], true),
        (
            &["run", "--allow-run=true", "wait.mjs"],
            !support::run_takes_a_network_namespace(),
        ),
    ] {
        let mut command = in_scratch(&scratch, &scratch.0.join("project"), args);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = KillOnDrop(command.spawn().unwrap());
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line, "ready\n", "{args:?}");
        let network = interfaces(&child.id().to_string());
        assert_eq!(network == own, own_network, "{args:?}");
        let status = support::wait_bounded("the run", &mut child);
        assert!(status.success(), "{args:?}");
        let mut stderr = String::new();
        std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
        let stderr = String::from_utf8(support::without_unboxed_note(stderr.into_bytes())).unwrap();
        if args.len() == 2 {
            assert_eq!(stderr, "", "{args:?}");
        } else {
            assert_eq!(support::past_network_note(&stderr), "", "{args:?}");
        }
    }
}

/// A build with no snapshot runs `fgro run` in no box after a note saying
/// so, which the other cases' support strips, and `GRO_FALLBACK=command`
/// refuses it.
#[test]
#[cfg_attr(
    feature = "snapshot",
    ignore = "a plain build's own: the snapshot build runs in its box"
)]
fn a_build_with_no_snapshot_runs_in_no_box_saying_so() {
    let scratch = project(
        "run_boxed_plain",
        &[("project/main.mjs", "console.log('ran');\n")],
    );
    let dir = scratch.0.join("project");
    let output = in_scratch(&scratch, &dir, &["run", "main.mjs"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "ran\n");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("{UNBOXED_NOTE}\n")
    );
    let output = in_scratch(&scratch, &dir, &["run", "--allow-run=true", "main.mjs"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "{UNBOXED_NOTE}, and the programs it starts run in no box of their own, writing \
             wherever you can\n"
        )
    );
    let mut command = in_scratch(&scratch, &dir, &["run", "main.mjs"]);
    command.env("GRO_FALLBACK", "command");
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: GRO_FALLBACK=command refuses running fgro run in no kernel box, where only the \
         runtime's own checks hold its grants: this build has no startup snapshot (cargo xtask \
         build makes one)\n"
    );
}

/// What a run refused, then why that fails it, as stderr ends.
const REFUSALS_FAIL: &str =
    "error: a refusal fails the run, whether or not the program caught it\n";

/// The listed refusal of an `import()` of `module`, outside an entry in no
/// project's import graph, as `fgro run` prints it.
fn outside_graph(module: &Path) -> String {
    format!(
        "error: import() refused: cannot load file://{}: the entry is in no project, so the run \
         loads only the modules its imports name by a string literal, read before it started, \
         and this isn't one. Fix: add --allow-read= naming its directory\n",
        module.display()
    )
}

/// The hint for a module outside the import graph of an entry in no
/// project whose directory is `dir`, the read flag naming it as `flag`.
fn graph_hint(dir: &Path, flag: &str) -> String {
    format!(
        "hint: {} is in no project (no package.json at or above the entry holds more than \
         \"type\" or \"sideEffects\"), so the run loads only the import graph it read before \
         it started: add --allow-read={flag} to load from disk there\n",
        dir.display()
    )
}

/// An entry in no project loads the modules its imports name as string
/// literals, read before the box: a static import, a re-export's module, one
/// whose every specifier is a type (which the strip keeps), and a literal
/// `import()`, while an import only of types names a missing file freely
/// and `import.meta.resolve` reads nothing; an `import()` of a name built at
/// run time is refused, failing the run though caught, hinting the read flag
/// (`.` from the directory, else its path), which loads it, as `-A` does, and
/// as a run in no box below the floor refuses it too. A module only an
/// import of types names isn't read, so an `import()` of it is refused.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn an_entry_in_no_project_loads_only_its_import_graph() {
    let scratch = Scratch::in_system_temp(
        "run_boxed_graph",
        &[
            (
                "loose/x.ts",
                "import {lib} from './lib.ts';\nimport type {Nope} from './missing.ts';\n\
                 import {type Spec} from './spec.ts';\nexport {deep} from './sub/deep.mjs';\n\
                 import {deep} from './sub/deep.mjs';\nconsole.log('static', lib, deep);\n\
                 const dyn = await import('./dyn.ts');\nconsole.log('literal', dyn.dyn);\n\
                 console.log('resolve', import.meta.resolve('./unreached.ts').endsWith('.ts'));\n\
                 try { await import(['./unreached', '.ts'].join('')); console.log('computed ok'); \
                 } catch { console.log('computed refused'); }\n",
            ),
            ("loose/lib.ts", "export const lib = 'lib';\n"),
            (
                "loose/spec.ts",
                "console.log('spec ran');\nexport type Spec = 1;\n",
            ),
            ("loose/sub/deep.mjs", "export const deep = 'deep';\n"),
            ("loose/dyn.ts", "export const dyn = 'dyn';\n"),
            ("loose/unreached.ts", "console.log('unreached ran');\n"),
            (
                "loose/t.ts",
                "import type {T} from './types.ts';\n\
                 try { await import(['./types', '.ts'].join('')); console.log('types loaded'); \
                 } catch { console.log('types refused'); }\n",
            ),
            ("loose/types.ts", "export type T = 1;\n"),
            ("home/.keep", ""),
            ("tmp/.keep", ""),
        ],
    );
    let loose = scratch.0.join("loose");
    let ran = "spec ran\nstatic lib deep\nliteral dyn\nresolve true\n";
    let refused = format!(
        "{}{REFUSALS_FAIL}{}",
        outside_graph(&loose.join("unreached.ts")),
        graph_hint(&loose, ".")
    );
    let (code, stdout, stderr) = run(in_scratch(&scratch, &loose, &["run", "x.ts"]));
    assert_eq!(
        (code, stdout, stderr),
        (Some(1), format!("{ran}computed refused\n"), refused.clone())
    );
    let (code, _, stderr) = run(in_scratch(&scratch, &scratch.0, &["run", "loose/x.ts"]));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.ends_with(&graph_hint(&loose, &loose.display().to_string())),
        "{stderr}"
    );
    for flags in [&["--allow-read=."][..], &["-A"]] {
        let args = [&["run"], flags, &["x.ts"]].concat();
        let (code, stdout, stderr) = run(in_scratch(&scratch, &loose, &args));
        assert_eq!(
            (code, stdout),
            (Some(0), format!("{ran}unreached ran\ncomputed ok\n")),
            "{flags:?} {stderr}"
        );
    }
    let (code, stdout, stderr) = run(in_scratch(&scratch, &loose, &["run", "t.ts"]));
    assert_eq!(
        (code, stdout.as_str()),
        (Some(1), "types refused\n"),
        "{stderr}"
    );
    // below the floor, in no box, the run's loader refuses it all the same
    let mut command = Command::new(python3());
    command
        .args(["-I", "-c", FILTER])
        .args([LANDLOCK_CREATE_RULESET.to_string(), ENOSYS.to_string()]);
    let mut command = support::wrapped_fgro_command(command, &["run", "x.ts"]);
    in_env(&mut command, &scratch, &loose);
    let (code, stdout, stderr) = run(command);
    assert_eq!(
        (code, stdout),
        (Some(1), format!("{ran}computed refused\n")),
        "{stderr}"
    );
    assert!(
        stderr.starts_with("note: fgro run runs in no kernel box here")
            && stderr.ends_with(&refused),
        "{stderr}"
    );
}

/// An entry in no project's loader serves each module the bytes read
/// before the box: one rewritten in place since runs as it was read, and
/// one replaced by a rename since isn't loaded.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn an_entry_in_no_project_runs_the_modules_as_read_before_the_box() {
    let scratch = Scratch::in_system_temp(
        "run_boxed_graph_changed",
        &[
            (
                "loose/x.ts",
                "import {fs} from '@fuzdev/gro/fs.ts';\n\
                 const sub = await fs.open_dir(`${import.meta.dirname}/sub`);\n\
                 await sub.write('inplace.ts', \"console.log('rewritten');\\n\");\n\
                 await sub.rename('fresh.ts', 'renamed.ts');\n\
                 await import('./sub/inplace.ts');\n\
                 try { await import('./sub/renamed.ts'); } catch (error) { \
                 console.log(String(error)); }\n",
            ),
            ("loose/sub/inplace.ts", "console.log('as read');\n"),
            ("loose/sub/renamed.ts", "console.log('renamed as read');\n"),
            ("loose/sub/fresh.ts", "console.log('fresh');\n"),
            ("home/.keep", ""),
            ("tmp/.keep", ""),
        ],
    );
    let loose = scratch.0.join("loose");
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &loose,
        &["run", "--allow-write=.", "x.ts"],
    ));
    assert_eq!(
        (code, stdout),
        (
            Some(0),
            format!(
                "as read\nTypeError: cannot load file://{}: the file at its path isn't the one \
                 the run read before it started, so it isn't loaded\n",
                loose.join("sub/renamed.ts").display()
            )
        ),
        "{stderr}"
    );
}

/// Beside an entry in no project, whose modules, a project nested in its
/// directory and that project's packages included, the run reads before
/// the box goes up and only beneath that directory or one `--allow-read=`
/// names, neither the run nor a program it starts reads a `node_modules`, a
/// symlink to another checkout's included, which the box neither notes nor
/// refuses whatever it leads to.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn an_entry_in_no_project_reads_no_node_modules() {
    let scratch = Scratch::in_system_temp(
        "run_boxed_graph_modules",
        &[
            (
                "loose/x.mjs",
                "import {fs} from '@fuzdev/gro/fs.ts';\nimport {spawn} from '@fuzdev/gro/spawn.ts';\n\
                 const cat = await spawn.program('cat').out(['node_modules/pkg/secret.txt']);\n\
                 let realm = 'read';\n\
                 try { await (await fs.open_dir(`${import.meta.dirname}/node_modules`))\
                 .read_text('pkg/secret.txt'); } catch { realm = 'refused'; }\n\
                 console.log(JSON.stringify([cat.code, cat.stdout, realm]));\n",
            ),
            ("other/package.json", "{\"name\": \"other\"}\n"),
            ("other/node_modules/pkg/secret.txt", "secret"),
            ("odd/x.mjs", "console.log('ran');\n"),
            ("odd/target/.keep", ""),
            ("home/.keep", ""),
            ("tmp/.keep", ""),
        ],
    );
    std::os::unix::fs::symlink(
        "../other/node_modules",
        scratch.0.join("loose/node_modules"),
    )
    .unwrap();
    std::os::unix::fs::symlink("target", scratch.0.join("odd/node_modules")).unwrap();
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &scratch.0.join("loose"),
        &["run", "--allow-run=cat", "x.mjs"],
    ));
    // the realm's refusal fails the run, and the box notes nothing but,
    // where the host refuses the run a network namespace, that it does
    assert_eq!(
        (code, stdout.as_str()),
        (Some(1), "[1,\"\",\"refused\"]\n"),
        "{stderr}"
    );
    assert!(
        support::past_network_note(&stderr).starts_with("error: fs refused a read: "),
        "{stderr}"
    );
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &scratch.0.join("odd"),
        &["run", "x.mjs"],
    ));
    assert_eq!(
        (code, stdout.as_str(), stderr.as_str()),
        (Some(0), "ran\n", "")
    );
}

/// A JS module tsv can't parse to read its imports, past its depth limits,
/// isn't loaded, naming the read flag, which loads it from disk; a
/// symlinked sibling, a hashbang, and a byte order mark load as read, from
/// a working directory reached through a symlink too, whose hint is `.`.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn an_entry_in_no_project_s_graph_reads_what_v8_runs() {
    let deep = format!(
        "export const d = {}1{};\n",
        "[".repeat(300),
        "]".repeat(300)
    );
    let scratch = Scratch::in_system_temp(
        "run_boxed_graph_parse",
        &[
            (
                "loose/x.mjs",
                "#!/usr/bin/env fgro\nimport {lib} from './link.mjs';\nimport './bom.mjs';\n\
                 console.log('lib', lib);\nawait import('./deep.mjs');\nconsole.log('deep');\n",
            ),
            ("loose/lib.mjs", "export const lib = 'lib';\n"),
            ("loose/bom.mjs", "\u{feff}console.log('bom');\n"),
            ("loose/deep.mjs", &deep),
            ("home/.keep", ""),
            ("tmp/.keep", ""),
        ],
    );
    let loose = scratch.0.join("loose");
    std::os::unix::fs::symlink("lib.mjs", loose.join("link.mjs")).unwrap();
    std::os::unix::fs::symlink("loose", scratch.0.join("alias")).unwrap();
    let alias = scratch.0.join("alias");
    let (code, stdout, stderr) = run(in_scratch(&scratch, &alias, &["run", "x.mjs"]));
    assert_eq!(
        (code, stdout.as_str()),
        (Some(1), "bom\nlib lib\n"),
        "{stderr}"
    );
    assert!(
        stderr.starts_with(&format!(
            "error: Uncaught (in promise) TypeError: cannot parse the JS for its imports: \
             Nesting exceeds tsv's depth limit of 256 wire levels. Fix: add --allow-read= naming \
             the entry's directory, which loads it and what it imports from disk without that \
             parse\n    at file://{}:1:",
            loose.join("deep.mjs").display()
        )) && stderr.ends_with(&graph_hint(&loose, ".")),
        "{stderr}"
    );
    let (code, stdout, stderr) = run(in_scratch(
        &scratch,
        &alias,
        &["run", "--allow-read=.", "x.mjs"],
    ));
    assert_eq!(
        (code, stdout.as_str()),
        (Some(0), "bom\nlib lib\ndeep\n"),
        "{stderr}"
    );
}

/// An entry in no project reads nothing outside its directory before the
/// box: a package a project nested there links from outside, by the
/// package's own directory or by its `node_modules`, is refused as the box
/// refuses a read, hinting no flag for the entry's directory, and loads
/// once `--allow-read=` names the directory it lies in; a linked package
/// whose `package.json` links back into the entry's directory is refused
/// at its module; a package installed beneath the entry's directory loads.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "fgro run runs in its box only from a snapshot build"
)]
fn an_entry_in_no_project_reads_no_package_linked_from_outside_its_directory() {
    let nested = |dir: &str| {
        [
            (
                format!("loose/{dir}.ts"),
                format!("import './{dir}/a.ts';\n"),
            ),
            (
                format!("loose/{dir}/package.json"),
                format!("{{\"name\": \"{dir}\", \"type\": \"module\"}}\n"),
            ),
            (
                format!("loose/{dir}/a.ts"),
                "import {foo} from 'foo';\nconsole.log(foo);\n".to_owned(),
            ),
        ]
    };
    let package = |dir: &str, value: &str| {
        [
            (
                format!("{dir}/foo/package.json"),
                "{\"name\": \"foo\", \"type\": \"module\", \"exports\": \"./index.js\"}\n"
                    .to_owned(),
            ),
            (
                format!("{dir}/foo/index.js"),
                format!("export const foo = '{value}';\n"),
            ),
        ]
    };
    let files: Vec<(String, String)> = [
        nested("pkgdir"),
        nested("nmlink"),
        nested("jsonback"),
        nested("inside"),
    ]
    .into_iter()
    .flatten()
    .chain(package("elsewhere/node_modules", "outside"))
    .chain(package("loose/inside/node_modules", "inside"))
    .chain([
        (
            "loose/fake.json".to_owned(),
            "{\"name\": \"foo\", \"type\": \"module\", \"exports\": \"./index.js\"}\n".to_owned(),
        ),
        (
            "away/foo/index.js".to_owned(),
            "export const foo = 'outside';\n".to_owned(),
        ),
        ("home/.keep".to_owned(), String::new()),
        ("tmp/.keep".to_owned(), String::new()),
    ])
    .collect();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let scratch = Scratch::in_system_temp("run_boxed_graph_nested_link", &files);
    let loose = scratch.0.join("loose");
    std::fs::create_dir(loose.join("pkgdir/node_modules")).unwrap();
    std::os::unix::fs::symlink(
        "../../../elsewhere/node_modules/foo",
        loose.join("pkgdir/node_modules/foo"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        "../../elsewhere/node_modules",
        loose.join("nmlink/node_modules"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        "../../loose/fake.json",
        scratch.0.join("away/foo/package.json"),
    )
    .unwrap();
    std::fs::create_dir(loose.join("jsonback/node_modules")).unwrap();
    std::os::unix::fs::symlink("../../../away/foo", loose.join("jsonback/node_modules/foo"))
        .unwrap();
    let json =
        std::fs::canonicalize(scratch.0.join("elsewhere/node_modules/foo/package.json")).unwrap();
    for entry in ["pkgdir.ts", "nmlink.ts"] {
        let (code, stdout, stderr) = run(in_scratch(&scratch, &loose, &["run", entry]));
        assert_eq!((code, stdout.as_str()), (Some(1), ""), "{entry} {stderr}");
        assert!(
            stderr.contains(&format!(
                "cannot read {}: it lies outside the box's roots",
                json.display()
            )) && !stderr.contains("hint:"),
            "{entry} {stderr}"
        );
        let (code, stdout, stderr) = run(in_scratch(
            &scratch,
            &loose,
            &["run", "--allow-read=../elsewhere", entry],
        ));
        assert_eq!(
            (code, stdout.as_str()),
            (Some(0), "outside\n"),
            "{entry} {stderr}"
        );
    }
    let module = std::fs::canonicalize(scratch.0.join("away/foo/index.js")).unwrap();
    let (code, stdout, stderr) = run(in_scratch(&scratch, &loose, &["run", "jsonback.ts"]));
    assert_eq!((code, stdout.as_str()), (Some(1), ""), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "{} lies outside the box's roots, so it isn't read",
            module.display()
        )) && !stderr.contains("hint:"),
        "{stderr}"
    );
    let (code, stdout, stderr) = run(in_scratch(&scratch, &loose, &["run", "inside.ts"]));
    assert_eq!(
        (code, stdout.as_str(), stderr.as_str()),
        (Some(0), "inside\n", "")
    );
}
