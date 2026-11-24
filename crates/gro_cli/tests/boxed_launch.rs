//! A launch in a program's own box (`gro_runtime::spawn_held` given a
//! `ProgramBox`), from inside a process holding the run's box: each case
//! runs in a child, this binary run again with the case's name, which takes
//! the run's steps from its main thread, its only one, with a plan of a run
//! that writes its root and starts programs, then builds two programs'
//! boxes, siblings nested in the run's, and launches `sleep`s in them as a
//! run does, each child it starts printed by its pid. A launch's steps are
//! observed (`gro_runtime::observe_launch_steps`, the `testing` feature),
//! so a case signals a thread between two of them.
//!
//! The injection matrix: two children alive in the sibling boxes, one
//! leading a group of its own and one sharing the run's, then a third
//! launch, during which a `SIGTERM` goes to one thread by `tgkill`: the
//! launch's own thread after its box is installed, after its slot is
//! reserved, after the child is spawned, or after its slot is armed, or the
//! thread that called the launch, blocked in the join, after the slot is
//! reserved or the child spawned. The caller is a thread of the run's, as
//! the verb's isolate is, or the process's main thread. Each run must end
//! by the `SIGTERM` well inside the second the handler waits at most for a
//! slot to be armed, every child it printed dead.
//!
//! The herds: runs that launch until the parent signals them from outside
//! at a random moment, four children alive at once, each run checked to
//! have ended by the signal with every child it printed dead: a `SIGTERM`
//! herd, its children in groups of their own and in the run's, and a
//! `SIGINT` herd, its children in groups of their own alone, since the
//! handler passes on only a `SIGTERM` to a child in the run's group. The
//! parent runs [`HERD_RUNS`] of each; `boxed_launch herds <n>` runs `n`.
//!
//! Then the launch's own cases: its steps all taken on one thread, not the
//! caller's, and with no box on the caller's; a child's blocked signals
//! none (`SigBlk` 0), since std's spawn hands the thread's mask to the
//! child; the program running in its box, refused a write the run's box
//! allows; a child leading a group of its own exactly when the launch
//! says so; and a launch refused, no child started, where the kernel
//! refuses the box's thread a signal to the main thread, which a seccomp
//! filter `python3` installs before the case starts stands in for, since
//! this kernel allows it.
//!
//! The binary runs without libtest's harness, whose threads would refuse
//! the box. Every case needs Landlock ABI 6, and fails below it. Each child
//! starts under `env --default-signal`, so the signals are at their
//! defaults however the test was launched.

// clippy's test allowance covers `#[test]` functions, and this binary has none
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a test's setup and its failures panic"
)]

mod support;

use std::io::{Read as _, Write as _};
use std::os::fd::AsFd as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gro_runtime::confine::plan::{BoxFacts, BoxInputs, BoxPlan};
use gro_runtime::confine::program::{ProgramBox, ProgramWrites};
use gro_runtime::confine::run::{RunClosed, RunConfined, probe_floor};
use gro_runtime::{LaunchStep, Spawned, Unspawned, observe_launch_steps, spawn_held};

use support::{KillOnDrop, Scratch};

/// `SIGINT`'s number.
const SIGINT: i32 = 2;

/// `SIGTERM`'s number.
const SIGTERM: i32 = 15;

/// How long a run may take to end once its signal is sent: well inside the
/// second the handler waits for a slot to be armed, which a launch whose
/// slot it can't wait for spends whole.
const PROMPT: Duration = Duration::from_millis(900);

/// How long a matrix row holds its launch between the reservation and
/// the arming once its signal is sent: well past the handler's scan, well
/// inside [`PROMPT`].
const HOLD: Duration = Duration::from_millis(100);

/// How long a killed child may take to be gone.
const GONE: Duration = Duration::from_secs(10);

/// The runs of each herd the parent makes by default.
const HERD_RUNS: usize = 50;

/// How many children a herd's run keeps alive at once.
const HERD_ALIVE: usize = 4;

/// The longest a herd's run goes before the parent signals it, in
/// milliseconds.
const HERD_MAX_DELAY_MS: u64 = 25;

/// The program every case launches, and its argument.
const SLEEP: [&str; 2] = ["/usr/bin/sleep", "600"];

/// The matrix's thread the `SIGTERM` goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The launch's own thread.
    Launch,
    /// The thread that called the launch.
    Caller,
}

/// The matrix's thread that calls the launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caller {
    /// A thread of the run's, the main thread joining it.
    Run,
    /// The main thread.
    Main,
}

/// The step of the matrix's last launch after which its signal goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// The box installed on the launch's thread.
    Restricted,
    /// The slot reserved.
    Reserved,
    /// The child spawned.
    Spawned,
    /// The slot armed.
    Armed,
}

/// The matrix's rows, each named for its caller, the thread signaled,
/// and the step ([`matrix_row`]).
const MATRIX: &[&str] = &[
    "run_launch_restricted",
    "run_launch_reserved",
    "run_launch_spawned",
    "run_launch_armed",
    "run_caller_reserved",
    "run_caller_spawned",
    "main_launch_restricted",
    "main_launch_reserved",
    "main_launch_spawned",
    "main_launch_armed",
    "main_caller_reserved",
    "main_caller_spawned",
];

/// The caller, the thread signaled, and the step of the matrix's row
/// `case`, `None` for a case of no row.
fn matrix_row(case: &str) -> Option<(Caller, Target, After)> {
    let mut parts = case.split('_');
    let caller = match parts.next()? {
        "run" => Caller::Run,
        "main" => Caller::Main,
        _ => return None,
    };
    let target = match parts.next()? {
        "launch" => Target::Launch,
        "caller" => Target::Caller,
        _ => return None,
    };
    let after = match parts.next()? {
        "restricted" => After::Restricted,
        "reserved" => After::Reserved,
        "spawned" => After::Spawned,
        "armed" => After::Armed,
        _ => return None,
    };
    Some((caller, target, after))
}

/// The cases that end on their own, each with what it prints.
const CASES: &[(&str, &str)] = &[
    (
        "steps",
        "boxed: restricted reserved spawned armed, on one thread, not the caller's, its own \
         group\n\
         unboxed: reserved spawned armed, on the caller's, the run's group\n",
    ),
    ("sigblk", "SigBlk:\t0000000000000000\n"),
    (
        "writes",
        "in its box, the project: refused\n\
         in its box, its temporary directory: written\n\
         in the run's box, the project: written\n",
    ),
    (
        "hand_off_refused",
        "the kernel refuses its box's thread a signal to the run's main thread, which a signal \
         ending the run is handed on to, so a child started there could outlive the run: \
         Operation not permitted (os error 1)\nno child started\n",
    ),
];

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("child") => {
            let case = args.next().unwrap();
            let scratch = PathBuf::from(args.next().unwrap());
            child(&case, &scratch);
        }
        Some("herds") => parent(args.next().unwrap().parse().unwrap()),
        // libtest's own flags (`--test-threads`, a filter) are ignored
        _ => parent(HERD_RUNS),
    }
}

/// Runs every case, `herd_runs` of each herd, and fails naming each that
/// failed.
fn parent(herd_runs: usize) {
    let scratch = Scratch::new("boxed_launch", &[]);
    let mut failures = Vec::new();
    let mut report = |case: &str, failed: Option<String>| {
        println!(
            "test {case} ... {}",
            if failed.is_none() { "ok" } else { "FAILED" }
        );
        if let Some(failed) = failed {
            failures.push(format!("{case}: {failed}"));
        }
    };
    for &(case, expected) in CASES {
        let (status, stdout, stderr) = run_to_end(case, &scratch.0);
        let failed = (!status.success() || stdout != expected)
            .then(|| format!("{status}\ngot:\n{stdout}\nexpected:\n{expected}\nstderr: {stderr}"));
        report(case, failed);
    }
    for &case in MATRIX {
        report(case, signaled(case, &scratch.0, None).err());
    }
    for (herd, signal) in [("herd_term", SIGTERM), ("herd_int", SIGINT)] {
        let mut seed = u64::from(std::process::id());
        let mut children = 0;
        let mut failed = Vec::new();
        for run in 0..herd_runs {
            let delay = random_below(&mut seed, HERD_MAX_DELAY_MS + 1);
            match signaled(
                herd,
                &scratch.0,
                Some((signal, Duration::from_millis(delay))),
            ) {
                Ok(printed) => children += printed,
                Err(why) => failed.push(format!("run {run}: {why}")),
            }
        }
        report(
            &format!("{herd} ({herd_runs} runs, {children} children)"),
            (!failed.is_empty()).then(|| failed.join("\n")),
        );
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The next number below `bound` of a linear congruential sequence.
const fn random_below(seed: &mut u64, bound: u64) -> u64 {
    *seed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*seed >> 33) % bound
}

/// What `python3` runs before the `hand_off_refused` case: a seccomp
/// filter refusing, with `EPERM`, `tgkill` with no signal from this
/// process to its own main thread, the hand-off's check, as a kernel that
/// doesn't exempt a process's own threads from Landlock's signal scope
/// refuses it to a thread in a nested domain, and allowing every other
/// call, a signal sent by `tgkill` and a check of another thread among
/// them; then it execs the rest of its arguments, which keep its pid.
const HAND_OFF_REFUSED: &str = r#"
import ctypes, os, struct, sys
pid = os.getpid()
allow = lambda at: 9 - at - 1
# the call number; past `tgkill` (234), allow
code = struct.pack("HBBI", 0x20, 0, 0, 0)
code += struct.pack("HBBI", 0x15, 0, allow(1), 234)
# its process and its thread, the first two arguments' low words: past
# this process's main thread, allow
code += struct.pack("HBBI", 0x20, 0, 0, 16)
code += struct.pack("HBBI", 0x15, 0, allow(3), pid)
code += struct.pack("HBBI", 0x20, 0, 0, 24)
code += struct.pack("HBBI", 0x15, 0, allow(5), pid)
# its signal, the third argument's low word; past 0, allow
code += struct.pack("HBBI", 0x20, 0, 0, 32)
code += struct.pack("HBBI", 0x15, 0, allow(7), 0)
code += struct.pack("HBBI", 0x06, 0, 0, 0x00050000 | 1)
code += struct.pack("HBBI", 0x06, 0, 0, 0x7fff0000)
buffer = ctypes.create_string_buffer(code)
class Program(ctypes.Structure):
    _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.c_void_p)]
program = Program(10, ctypes.cast(buffer, ctypes.c_void_p))
libc = ctypes.CDLL(None, use_errno=True)
ulong = ctypes.c_ulong
assert libc.prctl(38, ulong(1), ulong(0), ulong(0), ulong(0)) == 0
assert libc.prctl(22, ulong(2), ctypes.byref(program), ulong(0), ulong(0)) == 0
os.execvp(sys.argv[1], sys.argv[1:])
"#;

/// The command running `case` in a child, every guarding signal at its
/// default, the `hand_off_refused` case under its filter.
fn case_command(case: &str, scratch: &Path) -> Command {
    let mut command = if case == "hand_off_refused" {
        let mut command = Command::new("python3");
        command.args(["-I", "-c", HAND_OFF_REFUSED, "env"]);
        command
    } else {
        Command::new("env")
    };
    command
        .args([
            "--default-signal=INT",
            "--default-signal=TERM",
            "--default-signal=HUP",
        ])
        .arg(std::env::current_exe().unwrap())
        .arg("child")
        .arg(case)
        .arg(scratch)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Runs `case` to its end, returning its status, stdout, and stderr.
fn run_to_end(case: &str, scratch: &Path) -> (std::process::ExitStatus, String, String) {
    let mut child = KillOnDrop(case_command(case, scratch).spawn().unwrap());
    let stdout = support::drain(child.stdout.take().unwrap());
    let stderr = support::drain(child.stderr.take().unwrap());
    let status = support::wait_bounded(case, &mut child);
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    (
        status,
        text(stdout.join().unwrap()),
        text(stderr.join().unwrap()),
    )
}

/// Runs `case`, which ends by a signal: a matrix row's, which the child
/// sends itself, timed from its `inject` line, or, given `send`, a herd's,
/// sent to it by its pidfd once its `ready` line and the delay have passed,
/// and timed from the send. Fails unless it ends by that signal within
/// [`PROMPT`], having printed no `survived`, with every child it printed by
/// `child <pid>` dead within [`GONE`]; one that lives is killed.
fn signaled(case: &str, scratch: &Path, send: Option<(i32, Duration)>) -> Result<usize, String> {
    let mut child = KillOnDrop(case_command(case, scratch).spawn().unwrap());
    let lines = support::lines(child.stdout.take().unwrap());
    let stderr = support::drain(child.stderr.take().unwrap());
    let signal = send.map_or(SIGTERM, |(signal, _)| signal);
    let mut printed = Vec::new();
    let mut sent = None;
    let mut survived = false;
    while let Ok(line) = lines.recv_timeout(support::DEADLINE) {
        let line = line.trim_end();
        if let Some(pid) = line.strip_prefix("child ") {
            printed.push(pid.parse::<u32>().unwrap());
        } else if line == "inject" {
            sent = Some(Instant::now());
        } else if line == "ready"
            && let Some((signal, delay)) = send
        {
            std::thread::sleep(delay);
            gro_sys::child::Pidfd::open(child.id())
                .unwrap()
                .send_signal(signal)
                .unwrap();
            sent = Some(Instant::now());
        } else if line == "survived" {
            survived = true;
        }
    }
    let status = support::wait_bounded(case, &mut child);
    let took = sent.map(|sent| sent.elapsed());
    let stderr = String::from_utf8_lossy(&stderr.join().unwrap()).into_owned();
    let alive = alive_after(&printed);
    let mut failed = Vec::new();
    if status.signal() != Some(signal) {
        failed.push(format!("ended {status}, not by signal {signal}"));
    }
    match took {
        None => failed.push("no signal was sent".to_owned()),
        Some(took) if took > PROMPT => failed.push(format!("ended {took:?} after the signal")),
        Some(_) => {}
    }
    if survived {
        failed.push("its launch outlived the signal".to_owned());
    }
    // a herd's run signaled before its first launch prints none
    if printed.is_empty() && send.is_none() {
        failed.push("no child printed".to_owned());
    }
    if !alive.is_empty() {
        failed.push(format!(
            "children alive after it: {alive:?} of {printed:?}, killed now"
        ));
    }
    if failed.is_empty() {
        Ok(printed.len())
    } else {
        Err(format!("{}\nstderr: {stderr}", failed.join("; ")))
    }
}

/// Whether `pid` is a launched `sleep` still running, not a zombie.
fn is_alive(pid: u32) -> bool {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let expected: Vec<u8> = SLEEP
        .iter()
        .flat_map(|arg| [arg.as_bytes(), b"\0"].concat())
        .collect();
    if cmdline != expected {
        return false;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    // the state follows the command's name, in parentheses
    let state = stat.rsplit_once(") ").map(|(_, rest)| rest.chars().next());
    !matches!(state, None | Some(Some('Z' | 'X') | None))
}

/// The children of `printed` alive once [`GONE`] has passed for each, each
/// killed by its pidfd.
fn alive_after(printed: &[u32]) -> Vec<u32> {
    let start = Instant::now();
    while printed.iter().any(|&pid| is_alive(pid)) && start.elapsed() < GONE {
        std::thread::sleep(Duration::from_millis(5));
    }
    let alive: Vec<u32> = printed
        .iter()
        .copied()
        .filter(|&pid| is_alive(pid))
        .collect();
    for &pid in &alive {
        if let Ok(pidfd) = gro_sys::child::Pidfd::open(pid) {
            let _ = pidfd.send_signal(9);
        }
    }
    alive
}

/// Prints `line` at once.
fn say(line: &str) {
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{line}").unwrap();
    stdout.flush().unwrap();
}

/// The calling thread's id.
fn tid() -> i32 {
    let link = std::fs::read_link("/proc/thread-self").unwrap();
    link.file_name().unwrap().to_str().unwrap().parse().unwrap()
}

/// The process's id.
fn pid() -> i32 {
    i32::try_from(std::process::id()).unwrap()
}

/// Takes the run's steps to its box, a run writing `scratch`'s project and
/// starting programs, with its private directory in `scratch`'s `tmp`; the
/// handler that ends the run's children installed, as a run's first launch
/// installs it; and two programs' boxes built, siblings, each a flag's
/// program, which writes nothing of the project.
fn boxed(scratch: &Path) -> (RunConfined, ProgramBox, ProgramBox) {
    for dir in ["project/out", "tmp"] {
        std::fs::create_dir_all(scratch.join(dir)).unwrap();
    }
    std::fs::write(scratch.join("project/package.json"), "{}").unwrap();
    let scratch = std::fs::canonicalize(scratch).unwrap();
    let inputs = BoxInputs {
        root: scratch.join("project"),
        writes_root: true,
        spawns: true,
        ..BoxInputs::default()
    };
    let plan = BoxPlan::for_run(&inputs, &BoxFacts::read(&inputs).unwrap()).unwrap();
    let hardened = RunClosed::close_inherited(probe_floor().unwrap())
        .unwrap()
        .enter_network()
        .unwrap()
        .harden()
        .unwrap();
    let parent: std::os::fd::OwnedFd = std::fs::File::open(scratch.join("tmp")).unwrap().into();
    let private = hardened
        .make_private_dir(parent.as_fd(), &scratch.join("tmp"))
        .unwrap();
    drop(parent);
    let mut confined = hardened
        .prepare(&plan, Some(private))
        .unwrap()
        .confine()
        .unwrap();
    gro_sys::child::default_child_signal().unwrap();
    gro_sys::guard_children_on_signal().unwrap();
    let programs = confined.take_program_rules().unwrap();
    let first = programs.program_box(ProgramWrites::FLAG, 0).unwrap();
    let second = programs.program_box(ProgramWrites::FLAG, 1).unwrap();
    (confined, first, second)
}

/// A `sleep` command.
fn sleeper() -> Command {
    let mut command = Command::new(SLEEP[0]);
    command
        .arg(SLEEP[1])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Launches a `sleep` in `program_box`, leading a group of its own when
/// `grouped`; a refusal once a signal is ending the run waits for the
/// run's end, since nothing may unwind while the handler scans.
fn launch(program_box: &ProgramBox, grouped: bool) -> Spawned {
    match spawn_held(&mut sleeper(), grouped, Some(program_box)) {
        Ok(spawned) => spawned,
        Err(Unspawned::Unreserved { .. }) => outlived(),
        Err(error) => panic!("{error}"),
    }
}

/// Waits for a signal ending the run, then says it didn't.
fn outlived() -> ! {
    std::thread::sleep(Duration::from_secs(30));
    say("survived");
    std::process::exit(0)
}

/// Ends a launched child: kills it, frees its slot, and reaps it only when
/// the release says it may, else waits for the run's end.
fn end(spawned: Spawned) {
    let Spawned {
        mut child, guard, ..
    } = spawned;
    child.kill().unwrap();
    if !guard.release() {
        outlived();
    }
    child.wait().unwrap();
}

/// The matrix row's injection: the thread and the step, armed for the
/// row's last launch alone, and the caller's thread.
struct Injection {
    target: Target,
    after: After,
    caller: AtomicI32,
    armed: AtomicBool,
}

static INJECTION: OnceLock<Injection> = OnceLock::new();

/// The steps a launch took, each with the thread it took it on, for the
/// `steps` case.
static STEPS: Mutex<Vec<(LaunchStep, i32)>> = Mutex::new(Vec::new());

/// Whether the observer prints each child spawned, as it's spawned, so
/// the parent checks it whatever the signal ends.
static PRINT_CHILDREN: AtomicBool = AtomicBool::new(false);

/// The observer of every launch's steps: checks the thread holds a slot
/// reserved and not armed exactly between the reservation and the arming,
/// records each step, and at the armed injection's step sends the `SIGTERM` to its
/// thread, once, then holds a launch with its slot reserved for
/// [`HOLD`], so the handler's wait for the arming is what kills its child.
fn observe(step: LaunchStep) {
    let reserving = gro_sys::reserving_on_this_thread();
    let expected = u32::from(matches!(
        step,
        LaunchStep::Reserved | LaunchStep::Spawned(_)
    ));
    assert_eq!(
        reserving, expected,
        "{step:?} told with {reserving} slots reserved"
    );
    STEPS.lock().unwrap().push((step, tid()));
    if let LaunchStep::Spawned(pid) = step
        && PRINT_CHILDREN.load(Ordering::SeqCst)
    {
        say(&format!("child {pid}"));
    }
    let Some(injection) = INJECTION.get() else {
        return;
    };
    let at = match step {
        LaunchStep::Restricted => After::Restricted,
        LaunchStep::Reserved => After::Reserved,
        LaunchStep::Spawned(_) => After::Spawned,
        LaunchStep::Armed => After::Armed,
    };
    if at != injection.after || !injection.armed.swap(false, Ordering::SeqCst) {
        return;
    }
    let thread = match injection.target {
        Target::Launch => tid(),
        Target::Caller => injection.caller.load(Ordering::SeqCst),
    };
    say("inject");
    gro_sys::signal_thread(pid(), thread, SIGTERM).unwrap();
    if matches!(at, After::Reserved | After::Spawned) {
        std::thread::sleep(HOLD);
    }
}

/// Runs `case` from inside the run's box.
fn child(case: &str, scratch: &Path) {
    assert!(observe_launch_steps(observe));
    let (_confined, first, second) = boxed(scratch);
    if let Some((caller, target, after)) = matrix_row(case) {
        matrix(&first, &second, caller, target, after);
    }
    match case {
        "herd_term" | "herd_int" => herd(&first, &second, case == "herd_term"),
        "steps" => steps(&first),
        "sigblk" => sigblk(&first),
        "writes" => writes(&first, scratch),
        "hand_off_refused" => hand_off_refused(&first),
        _ => panic!("no case {case}"),
    }
}

/// A matrix row: a child in each sibling box, the first leading a group of
/// its own, the second in the run's; then, from `caller`, a third launch in
/// the first's box, in a group of its own from a thread of the run's and in
/// the run's group from the main thread, its `SIGTERM` sent to `target`
/// after `after`; then waits for the run's end.
fn matrix(first: &ProgramBox, second: &ProgramBox, caller: Caller, target: Target, after: After) {
    PRINT_CHILDREN.store(true, Ordering::SeqCst);
    let alive = [launch(first, true), launch(second, false)];
    INJECTION
        .set(Injection {
            target,
            after,
            caller: AtomicI32::new(0),
            armed: AtomicBool::new(false),
        })
        .ok()
        .unwrap();
    let last = |grouped: bool| {
        let injection = INJECTION.get().unwrap();
        injection.caller.store(tid(), Ordering::SeqCst);
        injection.armed.store(true, Ordering::SeqCst);
        // refused once the signal began, it starts nothing: the run is
        // ending either way
        spawn_held(&mut sleeper(), grouped, Some(first))
    };
    let launched = match caller {
        Caller::Run => std::thread::scope(|scope| scope.spawn(|| last(true)).join().unwrap()),
        Caller::Main => last(false),
    };
    let _held = (alive, launched);
    outlived();
}

/// A herd's run: launches until a signal ends it, alternating the two
/// boxes, [`HERD_ALIVE`] children alive at once, the oldest ended as each
/// starts, from a thread of the run's; each child leading a group of its
/// own, and, when `mixed`, every other one in the run's group instead.
fn herd(first: &ProgramBox, second: &ProgramBox, mixed: bool) {
    PRINT_CHILDREN.store(true, Ordering::SeqCst);
    say("ready");
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut alive = std::collections::VecDeque::new();
            let mut round = 0_u64;
            loop {
                let program_box = if round.is_multiple_of(2) {
                    first
                } else {
                    second
                };
                let grouped = !mixed || round % 4 < 2;
                round = round.wrapping_add(1);
                alive.push_back(launch(program_box, grouped));
                if alive.len() > HERD_ALIVE {
                    end(alive.pop_front().unwrap());
                }
            }
        });
    });
}

/// A launch in a box takes its steps on one thread, not the caller's, and
/// one with none on the caller's; a grouped launch's child leads a group of
/// its own, and another's is in the run's.
fn steps(program_box: &ProgramBox) {
    let names = |steps: &[(LaunchStep, i32)]| {
        steps
            .iter()
            .map(|(step, _)| match step {
                LaunchStep::Restricted => "restricted",
                LaunchStep::Reserved => "reserved",
                LaunchStep::Spawned(_) => "spawned",
                LaunchStep::Armed => "armed",
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let caller = tid();
    for (label, program_box, grouped) in
        [("boxed", Some(program_box), true), ("unboxed", None, false)]
    {
        STEPS.lock().unwrap().clear();
        let spawned = spawn_held(&mut sleeper(), grouped, program_box).unwrap();
        let group = match process_group(spawned.child.id()) {
            group if group == spawned.child.id() => "its own group",
            group if group == process_group(std::process::id()) => "the run's group",
            _ => "another group",
        };
        end(spawned);
        let steps = STEPS.lock().unwrap().clone();
        let thread = steps[0].1;
        let one = steps.iter().all(|(_, on)| *on == thread);
        let place = match (one, thread == caller) {
            (true, true) => "on the caller's",
            (true, false) => "on one thread, not the caller's",
            (false, _) => "on more than one thread",
        };
        say(&format!("{label}: {}, {place}, {group}", names(&steps)));
    }
}

/// The process group of the process `pid`, read from `/proc`.
fn process_group(pid: u32) -> u32 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    // the fields after the command's name, in parentheses: the state, the
    // parent, then the group
    let (_, rest) = stat.rsplit_once(") ").unwrap();
    rest.split(' ').nth(2).unwrap().parse().unwrap()
}

/// A child launched in a box starts with no signal blocked.
fn sigblk(program_box: &ProgramBox) {
    let mut command = Command::new("/usr/bin/grep");
    command
        .args(["SigBlk", "/proc/self/status"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    let Spawned {
        mut child, guard, ..
    } = spawn_held(&mut command, true, Some(program_box)).unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    child.wait().unwrap();
    assert!(guard.release());
    print!("{out}");
}

/// A program launched in its box writes its temporary directory and not
/// the project, which a program launched in the run's box alone writes.
fn writes(program_box: &ProgramBox, scratch: &Path) {
    let write = |path: &Path, program_box: Option<&ProgramBox>| {
        let mut command = Command::new("/usr/bin/sh");
        command
            .args([
                "-c",
                r#"if printf '' 2>/dev/null > "$1"; then echo written; else echo refused; fi"#,
            ])
            .arg("sh")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        let Spawned {
            mut child, guard, ..
        } = spawn_held(&mut command, true, program_box).unwrap();
        let mut out = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        child.wait().unwrap();
        assert!(guard.release());
        out.trim_end().to_owned()
    };
    let out = scratch.canonicalize().unwrap().join("project/out");
    say(&format!(
        "in its box, the project: {}",
        write(&out.join("boxed.txt"), Some(program_box))
    ));
    say(&format!(
        "in its box, its temporary directory: {}",
        write(&program_box.temp_dir().join("boxed.txt"), Some(program_box))
    ));
    say(&format!(
        "in the run's box, the project: {}",
        write(&out.join("unboxed.txt"), None)
    ));
}

/// A launch in a box whose thread the kernel refuses a signal to the main
/// thread fails, and starts no child.
fn hand_off_refused(program_box: &ProgramBox) {
    match spawn_held(&mut sleeper(), true, Some(program_box)) {
        Err(error @ Unspawned::HandOffRefused(_)) => say(&error.to_string()),
        other => panic!("not refused for the hand-off: {other:?}"),
    }
    let started = STEPS
        .lock()
        .unwrap()
        .iter()
        .any(|(step, _)| matches!(step, LaunchStep::Reserved | LaunchStep::Spawned(_)));
    say(if started {
        "a child started"
    } else {
        "no child started"
    });
}
