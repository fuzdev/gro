//! A spawned child's life past its call, with real processes, each wait
//! bounded: what the run kills when it ends, by `exit`, an uncaught throw,
//! or a signal, what a call kills at the drain bound, past its cap, and
//! after a timeout's grace, the pin's checks at launch, the working
//! directory a call names, a real `git`, and a herd of children up to the
//! signal handler's cap. Each child writes its pid where the test reads
//! it, and the test waits, bounded, for each process the run must have
//! killed to be gone.

use std::os::unix::process::ExitStatusExt as _;
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::temp_fixtures;
use crate::support::{self, KillOnDrop, Scratch, fgro_command, spawn};

/// How long a killed process may take to be gone, and a child to write
/// its pid.
const BOUND: Duration = Duration::from_secs(10);

/// The signals a test sends, by Linux's numbers, `libc` being no
/// dependency of the tests.
const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

/// Whether `pid` is a process that hasn't exited: present in `/proc` and
/// no zombie, which an orphan is until its new parent reaps it.
fn is_running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z'))
    })
}

/// Waits, bounded, for `pid`, `what`, to be gone.
fn await_gone(pid: u32, what: &str) {
    let gone_by = Instant::now() + BOUND;
    while is_running(pid) {
        assert!(Instant::now() < gone_by, "{what} {pid} outlived its run");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The pid a child wrote to `file` in `fixtures`, waited for, bounded.
fn read_pid(fixtures: &Scratch, file: &str) -> u32 {
    let path = fixtures.0.join(file);
    let by = Instant::now() + BOUND;
    loop {
        if let Some(pid) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        assert!(Instant::now() < by, "no pid in {}", path.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Kills `pid`, a process a test left running on purpose.
fn kill(pid: u32) {
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status();
}

/// A run that launches two children and then ends as its first argument
/// says: a captured one, which leads a group of its own, and a `run`, in
/// the run's group; each writes its pid, then sleeps, its output going
/// nowhere, so no pipe of the test's waits for it.
const ENDINGS: &str = "import { args } from '@fuzdev/gro/args.ts';
import { exit } from '@fuzdev/gro/exit.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
sh.out(['-c', 'echo $$ > grouped.pid; exec sleep 30']);
sh.run(['-c', 'echo $$ > shared.pid; exec sleep 30 > /dev/null 2>&1']);
while (true) {
\tconst found = await sh.out(['-c', 'test -s grouped.pid && test -s shared.pid']);
\tif (found.code === 0) break;
}
if (args[0] === 'exit') exit(5);
if (args[0] === 'throw') throw new Error('ended');
await new Promise(() => setInterval(() => {}, 1000));
";

/// `exit` kills every child the run launched before the process ends, and
/// so does an uncaught throw's end of the run: the group a captured child
/// leads, and a `run`'s child in the run's own group.
#[test]
fn no_child_outlives_a_run_that_exits_or_throws() {
    for (ending, code) in [("exit", 5), ("throw", 1)] {
        let fixtures = temp_fixtures(&format!("spawn_ending_{ending}"), &[("main.js", ENDINGS)]);
        let mut command = fgro_command(&["run", "--allow-run=sh", "main.js", ending]);
        command.current_dir(&fixtures.0);
        let output = spawn(command, b"");
        assert_eq!(output.status.code(), Some(code), "{ending}: {output:?}");
        await_gone(read_pid(&fixtures, "grouped.pid"), "the grouped child");
        await_gone(
            read_pid(&fixtures, "shared.pid"),
            "the child sharing the run's group",
        );
    }
}

/// A signal that ends the run kills the groups its children lead, as a
/// terminal's interrupt or hangup never reaches them; a `SIGTERM` is passed
/// on to a child in the run's own group, which a supervisor's `SIGTERM` of
/// the run alone doesn't reach, and an interrupt or a hangup isn't, as a
/// terminal sends those to that child itself. The run dies by the signal.
#[test]
fn a_signal_ending_the_run_kills_its_groups_and_passes_a_sigterm_on() {
    for (signal, number) in [("INT", SIGINT), ("TERM", SIGTERM), ("HUP", SIGHUP)] {
        let fixtures = temp_fixtures(&format!("spawn_signal_{signal}"), &[("main.js", ENDINGS)]);
        let mut command = std::process::Command::new("env");
        command.arg("--default-signal=INT,TERM,HUP");
        let mut command =
            support::wrapped_fgro_command(command, &["run", "--allow-run=sh", "main.js", "signal"]);
        command
            .current_dir(&fixtures.0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = KillOnDrop(command.spawn().unwrap());
        let grouped = read_pid(&fixtures, "grouped.pid");
        let shared = read_pid(&fixtures, "shared.pid");
        let status = std::process::Command::new("kill")
            .args([&format!("-{signal}"), &child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let status = support::wait_bounded(signal, &mut child);
        assert_eq!(status.signal(), Some(number), "{signal}: {status}");
        await_gone(grouped, "the grouped child");
        if number == SIGTERM {
            await_gone(shared, "the child sharing the run's group");
        } else {
            assert!(
                is_running(shared),
                "{signal}: the child sharing the run's group was signaled"
            );
            kill(shared);
        }
    }
}

/// What a call kills of what its child left behind: a grandchild holding
/// the pipes past the child's exit is killed with its group at the drain
/// bound, and one holding them while the child writes past its cap is
/// killed with the group then; each call's result is `truncated`.
#[test]
fn a_group_left_holding_the_pipes_is_killed_at_the_drain_bound_and_the_cap() {
    let fixtures = temp_fixtures(
        "spawn_drain",
        &[(
            "main.js",
            "import { spawn } from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
const started = Date.now();
const drained = await sh.out(['-c', 'sleep 30 & echo $!']);
const waited = Date.now() - started;
const capped = await sh.out(['-c', 'sleep 30 & echo $!; yes'], { max_bytes: 100000 });
console.log(JSON.stringify({
\tdrained: [drained.kind, drained.code, drained.truncated, waited >= 1900 && waited < 9000],
\tcapped: [capped.kind, capped.signal, capped.truncated],
\tpids: [drained.stdout.trim(), capped.stdout.split('\\n')[0]]
}));
",
        )],
    );
    let mut command = fgro_command(&["run", "--allow-run=sh", "main.js"]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed["drained"],
        serde_json::json!(["exited", 0, true, true])
    );
    assert_eq!(
        printed["capped"],
        serde_json::json!(["signaled", "SIGKILL", true])
    );
    for pid in printed["pids"].as_array().unwrap() {
        let pid: u32 = pid.as_str().unwrap().parse().unwrap();
        await_gone(pid, "a grandchild holding the pipes");
    }
}

/// A child that ignores the timeout's `SIGTERM` is killed with its group
/// once the grace has passed: `timed_out` by `SIGKILL`, the call settling
/// after the grace and well before its child would have ended.
#[test]
fn a_child_that_ignores_the_timeout_s_sigterm_is_killed_after_the_grace() {
    let fixtures = temp_fixtures(
        "spawn_grace",
        &[(
            "main.js",
            "import { spawn } from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
const started = Date.now();
const result = await sh.out(['-c', 'trap \"\" TERM; echo $$; sleep 30'], { timeout_ms: 100 });
const waited = Date.now() - started;
console.log(JSON.stringify([result.kind, result.code, result.signal, waited >= 2000 && waited < 9000, result.stdout.trim()]));
",
        )],
    );
    let mut command = fgro_command(&["run", "--allow-run=sh", "main.js"]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed.as_array().unwrap()[..4],
        serde_json::json!(["timed_out", null, "SIGKILL", true])
            .as_array()
            .unwrap()[..]
    );
    await_gone(
        printed[4].as_str().unwrap().parse().unwrap(),
        "the child that ignored the timeout",
    );
}

/// The pin is checked as the child launches: a script whose path now
/// leads to another file is refused, as is one whose path no longer
/// resolves, its cause carrying what the OS said, a native binary or a
/// script rewritten in place, which keeps its identity but not its change
/// time, one whose mode changed, and a native binary renamed over, whose
/// file, unlinked, lost a link, said to be replaced or removed; each
/// refusal is `launch_failed`, its cause saying why, and fails only that
/// call.
#[test]
fn the_pin_is_checked_as_the_child_launches() {
    let fixtures = temp_fixtures(
        "spawn_pins",
        &[
            ("bin/script", "#!/bin/sh\nexit 0\n"),
            ("bin/swapped", "#!/bin/sh\nexit 0\n"),
            ("bin/removed", "#!/bin/sh\nexit 0\n"),
            (
                "main.js",
                "import { fs } from '@fuzdev/gro/fs.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
const bin = await fs.open_dir(`${fs.project.path}/bin`);
const show = async (call) => {
\ttry {
\t\tconst result = await call();
\t\treturn [result.kind, result.code];
\t} catch (error) {
\t\tconst shown = [error.kind, error.cause.message.replaceAll(fs.project.path, '{project}')];
\t\treturn error.cause.cause ? [...shown, error.cause.cause.message] : shown;
\t}
};
const native = spawn.program('./bin/native');
const renamed = spawn.program('./bin/renamed');
const script = spawn.program('./bin/script');
const swapped = spawn.program('./bin/swapped');
const removed = spawn.program('./bin/removed');
const before = [await show(() => native.run([])), await show(() => script.run([]))];
await bin.copy('false', 'replacement');
await bin.rename('replacement', 'renamed');
await bin.write('swapped.new', '#!/bin/sh\\nexit 9\\n');
await bin.set_mode('swapped.new', 0o755);
await bin.rename('swapped.new', 'swapped');
await bin.remove('removed');
await bin.write('script', '#!/bin/sh\\nexit 7\\n');
await bin.set_mode('native', 0o700);
console.log(JSON.stringify({
\tbefore,
\trenamed: await show(() => renamed.run([])),
\tswapped: await show(() => swapped.run([])),
\tremoved: await show(() => removed.run([])),
\trewritten: await show(() => script.run([])),
\tremoded: await show(() => native.run([]))
}));
",
            ),
        ],
    );
    for name in ["native", "renamed"] {
        std::fs::copy("/bin/true", fixtures.0.join("bin").join(name)).unwrap();
    }
    std::fs::copy("/bin/false", fixtures.0.join("bin/false")).unwrap();
    fixtures.set_mode("bin/script", 0o755);
    fixtures.set_mode("bin/swapped", 0o755);
    fixtures.set_mode("bin/removed", 0o755);
    let mut command = fgro_command(&[
        "run",
        "--allow-run=./bin/native,./bin/renamed,./bin/script,./bin/swapped,./bin/removed",
        "--allow-write=bin",
        "main.js",
    ]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed,
        serde_json::json!({
            "before": [["exited", 0], ["exited", 0]],
            "renamed": [
                "launch_failed",
                "./bin/renamed at {project}/bin/renamed was replaced or removed after the run \
                 pinned it"
            ],
            "swapped": [
                "launch_failed",
                "./bin/swapped at {project}/bin/swapped now leads to {project}/bin/swapped, not \
                 the file pinned at {project}/bin/swapped"
            ],
            "removed": [
                "launch_failed",
                "./bin/removed at {project}/bin/removed no longer resolves, pinned to \
                 {project}/bin/removed",
                "No such file or directory (os error 2)"
            ],
            "rewritten": [
                "launch_failed",
                "./bin/script at {project}/bin/script changed after the run pinned it"
            ],
            "remoded": [
                "launch_failed",
                "./bin/native at {project}/bin/native changed after the run pinned it"
            ],
        })
    );
}

/// A call's `cwd` is the directory its fs handle names, opened beneath the
/// handle's root, and with none it's the project's, whatever directory
/// the run was started in; a handle whose directory is gone by the call
/// fails the launch.
#[test]
fn a_child_runs_in_its_call_s_directory_or_the_project_s() {
    let fixtures = temp_fixtures(
        "spawn_cwd",
        &[
            ("sub/inner/.keep", ""),
            ("started/.keep", ""),
            (
                "main.js",
                "import { fs } from '@fuzdev/gro/fs.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
const at = async (options) => {
\ttry {
\t\treturn (await sh.out(['-c', 'pwd'], options)).stdout.trim().replaceAll(fs.project.path, '{project}');
\t} catch (error) {
\t\treturn [error.kind, error.cause.message.endsWith(\" didn't start\"), error.cause.cause.message.replaceAll(fs.project.path, '{project}').split(':')[0]];
\t}
};
const sub = await fs.open_dir(`${fs.project.path}/sub`);
await sub.mkdir('gone');
const gone = sub.dir('gone');
const named = await at({ cwd: gone });
await sub.remove('gone');
console.log(JSON.stringify([await at(), await at({ cwd: sub.dir('inner') }), named, await at({ cwd: gone })]));
",
            ),
        ],
    );
    let mut command = fgro_command(&[
        "run",
        "--allow-run=sh",
        "--allow-write=../sub",
        "../main.js",
    ]);
    command.current_dir(fixtures.0.join("started"));
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed,
        serde_json::json!([
            "{project}",
            "{project}/sub/inner",
            "{project}/sub/gone",
            [
                "launch_failed",
                true,
                "its working directory {project}/sub/gone can't be opened"
            ]
        ])
    );
}

/// A real `git` runs as gro's library runs it: a status read through
/// `out`, and a commit through `out` with `stdin: 'inherit'`, in the
/// run's group, then the log read back.
#[test]
fn a_real_git_reads_and_commits() {
    let fixtures = temp_fixtures(
        "spawn_git",
        &[
            ("file.txt", "text\n"),
            (
                "main.js",
                "import { spawn } from '@fuzdev/gro/spawn.ts';
const git = spawn.program('git');
const config = ['-c', 'user.name=gro', '-c', 'user.email=gro@example.invalid', '-c', 'commit.gpgsign=false'];
const status = await git.out(['status', '--porcelain']);
const add = await git.out(['add', 'file.txt']);
const commit = await git.out([...config, 'commit', '-q', '-m', 'first'], { stdin: 'inherit' });
const log = await git.out(['log', '--format=%s']);
console.log(JSON.stringify([status.stdout, add.code, commit.code, commit.stderr, log.stdout]));
",
            ),
        ],
    );
    let init = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&fixtures.0)
        .env("HOME", &fixtures.0)
        .status()
        .unwrap();
    assert!(init.success());
    // a home of the test's, so no configuration of the machine's is read
    let mut command = fgro_command(&["run", "--allow-run=git", "main.js"]);
    command.current_dir(&fixtures.0).env("HOME", &fixtures.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed,
        serde_json::json!([
            "?? file.txt\n?? main.js\n?? package.json\n",
            0,
            0,
            "",
            "first\n"
        ])
    );
}

/// A herd of children: a few hundred launched at once, each exiting with
/// its own code; then, in one turn, as many group-leading children as the
/// signal handler guards, none of them reaped before the turn ends, and one
/// more, which fails its launch rather than run unguarded, the others each
/// exiting as it would.
#[test]
fn a_herd_of_children_launches_up_to_the_guard_cap() {
    let fixtures = temp_fixtures(
        "spawn_herd",
        &[(
            "main.js",
            &format!(
                "import {{ spawn }} from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
const cap = {cap};
const quick = await Promise.all(Array.from({{ length: 200 }}, (_, i) => sh.out(['-c', `exit ${{i % 7}}`])));
const held = Array.from({{ length: cap }}, () => sh.out(['-c', 'exit 3']));
let over;
try {{
\tawait sh.out(['-c', 'exit 0']);
\tover = 'launched';
}} catch (error) {{
\tover = [error.kind, error.cause.cause.message];
}}
const settled = await Promise.all(held);
console.log(JSON.stringify({{
\tquick: quick.every((result, i) => result.kind === 'exited' && result.code === i % 7),
\tover,
\theld: settled.every((result) => result.kind === 'exited' && result.code === 3)
}}));
",
                cap = gro_sys::GUARDED_GROUPS
            ),
        )],
    );
    let mut command = fgro_command(&["run", "--allow-run=sh", "main.js"]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed,
        serde_json::json!({
            "quick": true,
            "over": [
                "launch_failed",
                format!(
                    "the run guards {} process groups, the most it holds at once",
                    gro_sys::GUARDED_GROUPS
                )
            ],
            "held": true,
        })
    );
}

/// A run that grants a program hands its child no descriptor `fgro`
/// inherited above stderr, and reads how its child ended though `fgro` was
/// started with `SIGCHLD` ignored, which would have the kernel reap the
/// child itself.
#[test]
fn a_child_gets_no_inherited_descriptor_and_is_reaped_with_sigchld_ignored() {
    let fixtures = temp_fixtures(
        "spawn_inherited",
        &[(
            "main.js",
            "import { spawn } from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
const held = await sh.out(['-c', 'for fd in 3 4 5 6 7 8 9; do [ -e /proc/self/fd/$fd ] && printf \"%s \" $fd; done; exit 4']);
console.log(JSON.stringify([held.kind, held.code, held.stdout]));
",
        )],
    );
    let mut command = std::process::Command::new("bash");
    command.args([
        "-c",
        r#"exec 7<"$1"; shift; exec env --ignore-signal=CHLD "$@""#,
        "bash",
        &fixtures.path("main.js"),
    ]);
    let mut command = support::wrapped_fgro_command(command, &["run", "--allow-run=sh", "main.js"]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(printed, serde_json::json!(["exited", 4, ""]));
}

/// A captured child leads a process group of its own unless its stdin is
/// `'inherit'`, which leaves it in the run's group, as the Node module's
/// `is_grouped` decides: a child in a background group would stop on
/// `SIGTTIN` reading the terminal it was handed.
#[test]
fn a_captured_child_shares_the_run_s_group_only_with_stdin_inherit() {
    let fixtures = temp_fixtures(
        "spawn_group",
        &[(
            "main.js",
            "import { spawn } from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
const script = 'set -- $(sed \"s/.*) //\" /proc/$$/stat); mine=$3; set -- $(sed \"s/.*) //\" /proc/$PPID/stat); [ \"$mine\" = \"$3\" ] && echo shared || echo own';
const show = async (stdin) => (await sh.out(['-c', script], { stdin })).stdout.trim();
console.log(JSON.stringify([await show('inherit'), await show('ignore'), await show(new Uint8Array(1))]));
",
        )],
    );
    let mut command = fgro_command(&["run", "--allow-run=sh", "main.js"]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(printed, serde_json::json!(["shared", "own", "own"]));
}

/// A group whose leader exits on the timeout's `SIGTERM`, leaving a member
/// that ignores it and holds no pipe, settles the call at once, and the
/// group's `SIGKILL` still follows once the grace has passed, while the run
/// goes on: the leader held unreaped until then, as the Node module's grace
/// outlasts its result.
#[test]
fn a_group_s_grace_kill_outlasts_the_call_s_result() {
    let fixtures = temp_fixtures(
        "spawn_grace_after",
        &[(
            "main.js",
            "import { spawn } from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
const result = await sh.out(['-c', 'trap \"exit 0\" TERM; sh -c \"trap \\\\\"\\\\\" TERM; echo \\\\$\\\\$ > member.pid; exec sleep 30\" > /dev/null 2>&1 < /dev/null & sleep 30 & wait'], { timeout_ms: 300 });
console.log(JSON.stringify([result.kind, result.code]));
await new Promise((resolve) => setTimeout(resolve, 8000));
",
        )],
    );
    let mut command = fgro_command(&["run", "--allow-run=sh", "main.js"]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut run = KillOnDrop(command.spawn().unwrap());
    let member = read_pid(&fixtures, "member.pid");
    await_gone(member, "the group's member past the grace");
    assert!(
        run.try_wait().unwrap().is_none(),
        "the run ended before its group was killed"
    );
}

/// The processes running `sleep` with `mark`, its only argument, a value
/// no other process's is: each a child a run launched, or an orphan of one.
fn marked(mark: &str) -> Vec<u32> {
    let wanted = format!("sleep\0{mark}\0");
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|cmdline| cmdline == wanted.as_bytes())
                && is_running(*pid)
        })
        .collect()
}

/// The children marked `mark` left running once their run has ended,
/// waited for, bounded, to be gone, then killed: none, unless the run
/// missed some.
fn left_running(mark: &str) -> Vec<u32> {
    let gone_by = Instant::now() + Duration::from_secs(2);
    loop {
        let left = marked(mark);
        if left.is_empty() || Instant::now() >= gone_by {
            for pid in &left {
                kill(*pid);
            }
            return left;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A run of `main.js` in `fixtures` granted `sleep`, its argument `mark`,
/// started with the signals the handler guards against at their default
/// however the test was launched, its streams null.
fn guarded_run(fixtures: &Scratch, mark: &str) -> KillOnDrop {
    let mut command = std::process::Command::new("env");
    command.arg("--default-signal=INT,TERM,HUP");
    let mut command =
        support::wrapped_fgro_command(command, &["run", "--allow-run=sleep", "main.js", mark]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    KillOnDrop(command.spawn().unwrap())
}

/// Waits, bounded, until at least `count` processes are marked `mark`.
fn await_marked(mark: &str, count: usize) {
    let by = Instant::now() + BOUND;
    while marked(mark).len() < count {
        assert!(
            Instant::now() < by,
            "fewer than {count} children marked {mark}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A `SIGTERM` that arrives while the run is launching children, one
/// group-leading child and one in the run's group after another, at a
/// different point of the burst in each trial, kills every child launched:
/// the handler waits for a launch between its slot's reservation and its
/// arming, no launch is made once it has begun, and a handler that lands
/// on the launching thread hands the signal on to the main thread. The
/// run dies by the signal.
#[test]
fn a_signal_during_a_burst_of_launches_kills_every_child() {
    const TRIALS: u64 = 16;
    let fixtures = temp_fixtures(
        "spawn_signal_burst",
        &[(
            "main.js",
            "import { args } from '@fuzdev/gro/args.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
const sleep = spawn.program('sleep');
const launched = [];
for (let i = 0; i < 1000; i++) {
\tlaunched.push(sleep.out([args[0]]).catch(() => {}));
\tlaunched.push(sleep.run([args[0]]).catch(() => {}));
}
await Promise.all(launched);
",
        )],
    );
    let mut missed = Vec::new();
    for trial in 0..TRIALS {
        let mark = format!("41.{}{trial:02}", std::process::id());
        let mut run = guarded_run(&fixtures, &mark);
        await_marked(&mark, 1);
        // spread over the burst, from its first launches on
        std::thread::sleep(Duration::from_millis(trial * 15));
        let status = std::process::Command::new("kill")
            .args(["-TERM", &run.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let status = support::wait_bounded("the burst", &mut run);
        assert_eq!(status.signal(), Some(SIGTERM), "trial {trial}: {status}");
        let left = left_running(&mark);
        if !left.is_empty() {
            missed.push((trial, left.len()));
        }
    }
    assert!(
        missed.is_empty(),
        "trials that left children running, and how many: {missed:?}"
    );
}

/// A second `SIGTERM` about a millisecond after the first, while the
/// handler kills the groups of the run's many children, waits for it or
/// runs the handler again on another thread, rather than end the run
/// mid-scan: every group is killed.
#[test]
fn a_second_sigterm_during_the_handler_s_scan_kills_every_group() {
    const TRIALS: u64 = 3;
    const HELD: usize = 1000;
    let fixtures = temp_fixtures(
        "spawn_signal_twice",
        &[(
            "main.js",
            &format!(
                "import {{ args }} from '@fuzdev/gro/args.ts';
import {{ spawn }} from '@fuzdev/gro/spawn.ts';
const sleep = spawn.program('sleep');
await Promise.all(Array.from({{ length: {HELD} }}, () => sleep.out([args[0]])));
"
            ),
        )],
    );
    let mut missed = Vec::new();
    for trial in 0..TRIALS {
        let mark = format!("42.{}{trial:02}", std::process::id());
        let mut run = guarded_run(&fixtures, &mark);
        await_marked(&mark, HELD);
        // the shell's `kill` is a builtin, and `sleep` a program, about a
        // millisecond to start
        let status = std::process::Command::new("sh")
            .args([
                "-c",
                r#"kill -TERM "$0"; sleep 0.001; kill -TERM "$0""#,
                &run.id().to_string(),
            ])
            .status()
            .unwrap();
        assert!(status.success());
        let status = support::wait_bounded("the held children", &mut run);
        assert_eq!(status.signal(), Some(SIGTERM), "trial {trial}: {status}");
        let left = left_running(&mark);
        if !left.is_empty() {
            missed.push((trial, left.len()));
        }
    }
    assert!(
        missed.is_empty(),
        "trials that left groups running, and how many: {missed:?}"
    );
}

/// A run that grants a program raises its soft limit on open descriptors
/// to the hard one as it starts, as Node does, so a child, which inherits
/// it, shows it raised.
#[test]
fn a_run_that_grants_a_program_raises_its_open_files_limit() {
    let fixtures = temp_fixtures(
        "spawn_open_files",
        &[(
            "main.js",
            "import { spawn } from '@fuzdev/gro/spawn.ts';
const sh = spawn.program('sh');
console.log((await sh.out(['-c', 'echo $(ulimit -Sn) $(ulimit -Hn)'])).stdout.trim());
",
        )],
    );
    let mut command = std::process::Command::new("prlimit");
    command.arg("--nofile=1024:4096");
    let mut command = support::wrapped_fgro_command(command, &["run", "--allow-run=sh", "main.js"]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "4096 4096\n");
}
