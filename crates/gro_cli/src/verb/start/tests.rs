use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gro_runtime::confine::ConfineError;
use gro_runtime::confine::plan::{
    BoxFacts, BoxInputs, BoxPlan, DirFlag, GrantedFlagDir, PlanRefusal, RootWriter, RuleSource,
    RunWriteDir, RunWriteGrant, ToolDir, ToolWrites,
};
use gro_runtime::resolver::InstalledPackage;
use gro_runtime::{BINARY_NAME, RunPins};

use super::{BoxStartError, TempHolder, box_inputs};
use crate::cli::{FlagDirs, RunGrant};
use crate::run::Environment;
use crate::verb::fallback::{FallbackPolicy, FallbackReason, KernelLandlock, fallback_note};
use crate::verb::prepass::{VerbError, VerbRun, floor_fallback};
use crate::{EXIT_FAILURE, EXIT_USAGE};

/// An environment of the test's own.
struct TestEnvironment(Vec<(&'static str, &'static str)>);

impl Environment for TestEnvironment {
    fn var(&self, name: &str) -> Option<OsString> {
        self.0
            .iter()
            .find(|(var, _)| *var == name)
            .map(|(_, value)| OsString::from(value))
    }

    fn vars(&self) -> Vec<(OsString, OsString)> {
        self.0
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }
}

/// The programs of the grants table `tools` lists, each local, by its
/// name in the run, writing the tool directories listed for it.
fn tools(tools: &[(&str, &[ToolDir])]) -> BTreeMap<String, ToolWrites> {
    tools
        .iter()
        .map(|&(name, dirs)| {
            let tool = ToolWrites {
                name: name.to_owned(),
                tool_dirs: dirs.iter().copied().collect(),
            };
            (format!("node_modules/.bin/{name}"), tool)
        })
        .collect()
}

/// The plan of a run at `/p` that may start a program, granting the
/// grants table's `tools` and `--allow-run-write=` naming `run_writes`,
/// whose write sets a tools' note reads.
fn spawning_plan(tools: BTreeMap<String, ToolWrites>, run_writes: Vec<RunWriteGrant>) -> BoxPlan {
    let inputs = BoxInputs {
        root: PathBuf::from("/p"),
        spawns: true,
        tools,
        run_writes,
        ..BoxInputs::default()
    };
    BoxPlan::for_run(&inputs, &BoxFacts::default()).unwrap()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|&item| item.to_owned()).collect()
}

/// A run of `check` at `/p` with the grant flags `flags`, its closure
/// reading `.` and granting git and a local vitest, the policy `policy`
/// and the installed Gro `gro`.
fn run(
    flags: FlagDirs,
    temp: bool,
    policy: FallbackPolicy,
    gro: Option<InstalledPackage>,
) -> VerbRun {
    let mut dirs = flags.clone();
    dirs.read.insert(".".to_owned());
    VerbRun {
        cli_args: vec!["check".to_owned(), "--watch".to_owned()],
        root: PathBuf::from("/p"),
        grant: RunGrant::Named {
            env: BTreeSet::new(),
            temp,
            run: set(&["git", "node_modules/.bin/vitest"]),
            run_env: BTreeSet::new(),
        },
        dirs,
        flag_dirs: flags,
        writes_root: false,
        tools: tools(&[("vitest", &[ToolDir::Vitest])]),
        declared: set(&["vitest"]),
        policy,
        gro,
        programs: BTreeMap::new(),
        run_writes: BTreeMap::new(),
        may_start: BTreeSet::new(),
        development: true,
        entry: String::new(),
    }
}

/// The box's inputs take the grant flags' directories alone, as they
/// resolved, never the closure's, and what git's home is found by from the
/// environment.
#[test]
fn the_box_takes_the_grant_flags_directories_alone() {
    let flags = FlagDirs {
        read: set(&["r"]),
        write: set(&["out"]),
        run_in: BTreeSet::new(),
        run_write: BTreeMap::new(),
    };
    let resolved = [
        (DirFlag::Read, ".", "/p"),
        (DirFlag::Read, "r", "/real/r"),
        (DirFlag::Write, "out", "/p/out"),
    ]
    .map(|(flag, dir, real)| GrantedFlagDir {
        flag,
        dir: dir.to_owned(),
        real: PathBuf::from(real),
    });
    let environment = TestEnvironment(vec![
        ("HOME", "/home/u"),
        ("GIT_CONFIG_GLOBAL", "/g/config"),
    ]);
    let mut run = run(flags, true, FallbackPolicy::All, None);
    run.programs.insert(
        "node_modules/.bin/vitest".to_owned(),
        gro_runtime::ProgramGrant::Flag {
            forward: Vec::new(),
        },
    );
    run.run_writes = BTreeMap::from([
        ("vitest".to_owned(), "node_modules/.bin/vitest".to_owned()),
        (
            "node_modules/.bin/vitest".to_owned(),
            "node_modules/.bin/vitest".to_owned(),
        ),
        ("git".to_owned(), "git".to_owned()),
    ]);
    let run_write = |named: &str, dir: &str, real: &str| RunWriteDir {
        named: named.to_owned(),
        dir: dir.to_owned(),
        real: PathBuf::from(real),
    };
    let run_writes = [
        run_write("git", ".git", "/p/.git"),
        run_write("vitest", "src", "/p/src"),
        run_write("node_modules/.bin/vitest", "out", "/p/out"),
        // a name the pre-pass didn't resolve, which no run reaches
        run_write("x", "y", "/p/y"),
    ];
    let inputs = box_inputs(
        &run,
        &resolved,
        &run_writes,
        &RunPins::default(),
        &environment,
    );
    assert_eq!(inputs.flags, resolved[1..]);
    // grouped by the program each name resolved to
    assert_eq!(
        inputs.run_writes,
        [
            RunWriteGrant {
                program: "git".to_owned(),
                dirs: vec![run_writes[0].clone()],
            },
            RunWriteGrant {
                program: "node_modules/.bin/vitest".to_owned(),
                dirs: vec![run_writes[1].clone(), run_writes[2].clone()],
            },
        ]
    );
    assert_eq!(inputs.root, Path::new("/p"));
    assert!(inputs.spawns && inputs.temp && !inputs.unconfined && !inputs.writes_root);
    assert_eq!(inputs.tools, run.tools);
    assert_eq!(inputs.declared, set(&["vitest"]));
    assert_eq!(inputs.home, Some(PathBuf::from("/home/u")));
    assert_eq!(inputs.xdg_config_home, None);
    assert_eq!(inputs.git_config_global, Some(PathBuf::from("/g/config")));
    assert!(inputs.programs.is_empty());
}

/// The kernel's shortfall reads from the floor probe's refusal, and an
/// error reading the status is none.
#[test]
fn the_floor_s_refusal_names_the_kernel_s_landlock() {
    for (error, kernel, text) in [
        (
            ConfineError::NoLandlock { built: false },
            KernelLandlock::Absent,
            "Landlock isn't available here (the kernel has none, or a seccomp filter refuses its \
             calls), which the kernel box needs at ABI 6 (Linux 6.12) or later",
        ),
        (
            ConfineError::NoLandlock { built: true },
            KernelLandlock::Disabled,
            "the kernel's Landlock isn't enabled (it is missing from the boot's lsm= list), \
             which the kernel box needs at ABI 6 (Linux 6.12) or later",
        ),
        (
            ConfineError::LandlockAbi { abi: 5 },
            KernelLandlock::Abi(5),
            "the kernel's Landlock is ABI 5, below the ABI 6 (Linux 6.12) the kernel box needs",
        ),
    ] {
        assert_eq!(KernelLandlock::from_floor_error(&error), Some(kernel));
        assert_eq!(FallbackReason::KernelFloor(kernel).to_string(), text);
    }
    assert_eq!(
        KernelLandlock::from_floor_error(&ConfineError::LandlockPartial),
        None
    );
}

/// A kernel below the floor runs the project's Node Gro, saying why; under
/// `GRO_FALLBACK=command` it's refused, naming the Node Gro's command; and
/// with no Node Gro installed it's refused as any fallback is.
#[test]
fn a_kernel_below_the_floor_falls_back_unless_the_policy_refuses() {
    let gro = InstalledPackage {
        root: PathBuf::from("/p/node_modules/@fuzdev/gro"),
        version: Some("1.2.3".to_owned()),
    };
    let reason = FallbackReason::KernelFloor(KernelLandlock::Abi(5));
    let fallback = floor_fallback(
        run(
            FlagDirs::default(),
            false,
            FallbackPolicy::All,
            Some(gro.clone()),
        ),
        reason.clone(),
    )
    .unwrap();
    assert_eq!(
        fallback_note(&fallback),
        format!(
            "note: {BINARY_NAME} check runs on the Node gro in node_modules/@fuzdev/gro 1.2.3, \
             which confines nothing, because the kernel's Landlock is ABI 5, below the ABI 6 \
             (Linux 6.12) the kernel box needs"
        )
    );
    let refused = floor_fallback(
        run(
            FlagDirs::default(),
            false,
            FallbackPolicy::Command,
            Some(gro),
        ),
        reason.clone(),
    )
    .unwrap_err();
    assert!(matches!(refused, VerbError::StrictHost { .. }), "{refused}");
    assert_eq!(refused.exit_code(), EXIT_USAGE);
    assert_eq!(
        refused.to_string(),
        "GRO_FALLBACK=command refuses running the project's own gro, which confines nothing, in \
         the kernel box's place: the kernel's Landlock is ABI 5, below the ABI 6 (Linux 6.12) \
         the kernel box needs. Run node_modules/.bin/gro check --watch for the project's, unconfined"
    );
    let none = floor_fallback(
        run(FlagDirs::default(), false, FallbackPolicy::All, None),
        reason,
    )
    .unwrap_err();
    assert_eq!(none.exit_code(), EXIT_USAGE);
    assert_eq!(
        none.to_string(),
        format!(
            "{BINARY_NAME} check runs on the Node gro, because the kernel's Landlock is ABI 5, \
             below the ABI 6 (Linux 6.12) the kernel box needs, and this project installs none: npm i \
             -D @fuzdev/gro"
        )
    );
}

/// The refusals before the box: `-A` names the Node Gro's command, a build
/// with no snapshot says how to build one, and a refusal of the plan prints
/// its paths escaped.
#[test]
fn the_refusals_before_the_box_print_their_texts_and_codes() {
    let unconfined = BoxStartError::Unconfined {
        cli_args: vec!["check".to_owned(), "a b".to_owned()],
    };
    assert_eq!(unconfined.exit_code(), EXIT_USAGE);
    assert_eq!(
        unconfined.to_string(),
        "-A would run a command of Gro's unconfined, and the binary runs each in a kernel box: \
         the project's Node gro is the unconfined spelling (node_modules/.bin/gro check 'a b')"
    );
    let unbuilt = BoxStartError::NoSnapshot {
        command: Some("check".to_owned()),
    };
    assert_eq!(unbuilt.exit_code(), EXIT_FAILURE);
    assert_eq!(
        unbuilt.to_string(),
        format!(
            "{BINARY_NAME} check runs in a kernel box, which starts only from a startup snapshot \
             embedded in the binary, and this build has none: started from source, the runtime \
             reads its JS from the build tree, outside the box. Build {BINARY_NAME} with its \
             snapshot: cargo xtask build"
        )
    );
    let planted = BoxStartError::Plan {
        refusal: PlanRefusal::NodeModulesLink {
            real: PathBuf::from("/h/.ssh\u{1b}[2J"),
        },
        command: Some("check".to_owned()),
    };
    assert_eq!(planted.exit_code(), EXIT_USAGE);
    let text = planted.to_string();
    assert!(text.contains("/h/.ssh\\u{1b}[2J"), "{text}");
    assert!(!text.contains('\u{1b}'), "{text}");
    // the corner's refusal names the command that writes the project, or
    // the flag that does, escaped
    let corner = |command: Option<&str>, cause| BoxStartError::Plan {
        refusal: PlanRefusal::RealmWritesProgramDir {
            program: "svelte-kit".to_owned(),
            dir: ".git\u{1b}[2J".to_owned(),
            cause,
        },
        command: command.map(str::to_owned),
    };
    let closure = corner(Some("gen"), RootWriter::Closure);
    assert_eq!(closure.exit_code(), EXIT_USAGE);
    assert_eq!(
        closure.to_string(),
        format!(
            "--allow-run-write=svelte-kit:.git\\u{{1b}}[2J would let gro's own code write \
             .git\\u{{1b}}[2J as well, since {BINARY_NAME} gen writes the project and only the \
             kernel box keeps it out of .git, node_modules, and the dot-entries: add \
             --allow-write=.git\\u{{1b}}[2J to grant it to both"
        )
    );
    assert_eq!(
        corner(Some("test"), RootWriter::Flag("./".to_owned())).to_string(),
        "--allow-run-write=svelte-kit:.git\\u{1b}[2J would let gro's own code write \
         .git\\u{1b}[2J as well, since --allow-write=./ lets gro's own code write the project \
         and only the kernel box keeps it out of .git, node_modules, and the dot-entries: add \
         --allow-write=.git\\u{1b}[2J to grant it to both"
    );
    let stdio = BoxStartError::Confine(ConfineError::StdioSocket { fds: vec![0] });
    assert_eq!(stdio.exit_code(), EXIT_USAGE);
    let threads = BoxStartError::Confine(ConfineError::Threads(gro_runtime::confine::Threads {
        count: 2,
        first_step: false,
    }));
    assert_eq!(threads.exit_code(), EXIT_FAILURE);
}

/// Every path planted where the box opens a rule or makes a tool
/// directory, which the project or the command line holds, exits 2, as do
/// the refusals the caller fixes; a step that fails exits 1.
#[test]
fn a_planted_path_exits_2_and_a_failed_step_1() {
    use gro_runtime::confine::rules::RuleError;
    use gro_runtime::confine::run::RunStepError;
    use gro_runtime::confine::tool_dirs::ToolDirError;

    let path = || PathBuf::from("/p/x");
    let io = || std::io::Error::other("x");
    let usage = [
        BoxStartError::Unconfined {
            cli_args: Vec::new(),
        },
        BoxStartError::Confine(ConfineError::StdioSocket { fds: vec![0] }),
        BoxStartError::Plan {
            refusal: PlanRefusal::Unconfined,
            command: None,
        },
        BoxStartError::TempInProject {
            path: path(),
            variable: Some("TMPDIR"),
        },
        BoxStartError::TempHeldByRule {
            path: path(),
            rule: PathBuf::from("/"),
            holder: TempHolder::Flag(DirFlag::Read),
            variable: Some("TMPDIR"),
        },
        BoxStartError::ToolDir(ToolDirError::NotADirectory { path: path() }),
        BoxStartError::ToolDir(ToolDirError::Symlink { path: path() }),
        BoxStartError::Rule(RuleError::Symlink { path: path() }),
        BoxStartError::Rule(RuleError::Escapes { path: path() }),
        BoxStartError::Rule(RuleError::NotReal {
            path: path(),
            opened: path(),
        }),
    ];
    for error in usage {
        assert_eq!(error.exit_code(), EXIT_USAGE, "{error}");
    }
    let failure = [
        BoxStartError::NoSnapshot { command: None },
        BoxStartError::Confine(ConfineError::LandlockPartial),
        BoxStartError::Step(RunStepError::Descriptors(io())),
        BoxStartError::TempDir {
            path: path(),
            source: io(),
        },
        BoxStartError::Facts {
            root: path(),
            source: io(),
        },
        BoxStartError::ToolDir(ToolDirError::Make {
            path: path(),
            source: io(),
        }),
        BoxStartError::Rule(RuleError::Missing { path: path() }),
        BoxStartError::Rule(RuleError::Open {
            path: path(),
            source: io(),
        }),
        BoxStartError::Rule(RuleError::NoPrivateDir { path: path() }),
        BoxStartError::Environment {
            name: "TMPDIR",
            source: io(),
        },
        BoxStartError::ChildSignal(io()),
        BoxStartError::Thread(io()),
    ];
    for error in failure {
        assert_eq!(error.exit_code(), EXIT_FAILURE, "{error}");
    }
    let inside = BoxStartError::TempInProject {
        path: PathBuf::from("/p/src\u{1b}[2J"),
        variable: Some("TMP"),
    };
    let text = inside.to_string();
    assert!(
        text.contains("/p/src\\u{1b}[2J is inside the project"),
        "{text}"
    );
    assert!(!text.contains('\u{1b}'), "{text}");
    assert!(text.ends_with(": point TMP outside the project"), "{text}");
    let default = BoxStartError::TempInProject {
        path: PathBuf::from("/tmp"),
        variable: None,
    };
    assert!(
        default
            .to_string()
            .ends_with(": it's the default, so set TMPDIR outside the project"),
        "{default}"
    );
}

/// A rule as the wall texts read it.
fn wall(path: &str, source: RuleSource, writes: bool) -> super::WallRule {
    super::WallRule {
        path: PathBuf::from(path),
        source,
        writes,
    }
}

/// The wall texts are built from the rules: the top-level entries
/// summarized, tool and flag directories named and escaped, the private
/// directory last; `.git` and `node_modules` left out of what stays
/// read-only where a write rule covers them, `node_modules` naming its tool
/// directories only where there are some; and the cause added only to a
/// path no write rule holds.
#[test]
fn the_wall_texts_follow_the_rules() {
    use gro_runtime::confine::run::RunNetwork;

    let generating = VerbRun {
        cli_args: vec!["gen".to_owned()],
        ..run(FlagDirs::default(), false, FallbackPolicy::All, None)
    };
    let mut check = run(FlagDirs::default(), false, FallbackPolicy::All, None);
    check.may_start = set(&["git", "vitest"]);
    let writing = [
        wall("/p", RuleSource::Project, false),
        wall("/p/src", RuleSource::Entry, true),
        wall("/p/package.json", RuleSource::Entry, true),
        wall("/p/node_modules", RuleSource::NodeModules, false),
        wall(
            "/p/.svelte-kit",
            RuleSource::ToolDir(ToolDir::SvelteKit),
            true,
        ),
        wall(
            "/p/node_modules/.vite/vitest",
            RuleSource::ToolDir(ToolDir::Vitest),
            true,
        ),
        wall("/dev/null", RuleSource::Device, true),
        wall("/tmp/gro-1-2", RuleSource::PrivateTemp, true),
    ];
    let cause = super::denied_write_cause(&generating, &writing, true);
    assert_eq!(
        cause.cause,
        format!(
            "the run's kernel box lets {BINARY_NAME} gen write only to the top-level entries the \
             project had as it started (but .git, node_modules, the dot-entries, symlinks), \
             .svelte-kit, node_modules/.vite/vitest, and a temporary directory of the run's; \
             .git, node_modules but its tool directories, the other dot-entries, and a new \
             top-level entry stay read-only"
        )
    );
    assert_eq!(
        cause.writable,
        [
            "/p/src",
            "/p/package.json",
            "/p/.svelte-kit",
            "/p/node_modules/.vite/vitest",
            "/dev/null",
            "/tmp/gro-1-2"
        ]
        .map(PathBuf::from)
    );
    // check mode, a linked node_modules with no tool directory there: each
    // program names only its own of the tool directories the box made
    let checking = [
        wall("/p", RuleSource::Project, false),
        wall("/o/node_modules", RuleSource::NodeModules, false),
        wall(
            "/p/.svelte-kit",
            RuleSource::ToolDir(ToolDir::SvelteKit),
            true,
        ),
        wall("/tmp/gro-1-2", RuleSource::PrivateTemp, true),
    ];
    let table = spawning_plan(
        tools(&[
            ("svelte-kit", &[ToolDir::SvelteKit, ToolDir::App]),
            ("vitest", &[ToolDir::SvelteKitDev, ToolDir::SvelteKitTypes]),
        ]),
        Vec::new(),
    );
    assert_eq!(
        super::walls_note(&check, &checking, &table, &RunNetwork::Namespace),
        format!(
            "note: {BINARY_NAME} check's tools each ran in a kernel box of their own inside the \
             run's, which lets them reach no network but the run's own loopback and write only \
             to a temporary directory of their own and to their own directories (svelte-kit: \
             .svelte-kit): a tool that hits a wall fails with EACCES, EPERM, or ENOSYS; \
             --allow-run-write=<program>:<dirs> lets one of them (git or vitest) write more, \
             --allow-read= and --allow-run-in= widen every box, and --allow-write= widens only \
             what gro's own code writes"
        )
    );
    // write flags naming .git and the linked node_modules's checkout, and
    // a name the terminal would read
    let flagged = [
        wall("/p", RuleSource::Project, false),
        wall("/o/node_modules", RuleSource::NodeModules, false),
        wall("/p/.git", RuleSource::Flag(DirFlag::Write), true),
        wall("/o", RuleSource::Flag(DirFlag::Write), true),
        wall("/p/out\u{1b}[2J", RuleSource::Flag(DirFlag::Write), true),
    ];
    let cause = super::denied_write_cause(&generating, &flagged, false);
    assert_eq!(
        cause.cause,
        format!(
            "the run's kernel box lets {BINARY_NAME} gen write only to .git, /o, and \
             out\\u{{1b}}[2J; the other dot-entries and a new top-level entry stay read-only"
        )
    );
    // a program's write rules, and the entries only a program's names the
    // root for: the tools' note names them for the program each is for,
    // a program of the table's beside its own tool directories, one its
    // flag names too named once, and never a write flag's; the realm's cause leaves them out, though the kernel
    // lets the run write there, and they cover nothing it keeps read-only
    let programs = [
        wall("/p", RuleSource::Project, false),
        wall("/p/src", RuleSource::Entry, true),
        wall("/p/.git", RuleSource::RunWrite(0), true),
        wall("/p/node_modules", RuleSource::RunWrite(1), true),
        wall("/o", RuleSource::RunWrite(1), true),
        wall("/p/lib", RuleSource::RunWrite(2), true),
        wall(
            "/p/.svelte-kit/generated/dev",
            RuleSource::RunWrite(2),
            true,
        ),
        wall("/p/out", RuleSource::Flag(DirFlag::Write), true),
        wall(
            "/p/.svelte-kit",
            RuleSource::ToolDir(ToolDir::SvelteKit),
            true,
        ),
        wall(
            "/p/.svelte-kit/generated/dev",
            RuleSource::ToolDir(ToolDir::SvelteKitDev),
            true,
        ),
        wall("/tmp/gro-1-2", RuleSource::PrivateTemp, true),
    ];
    let run_write = |program: &str, named: &str, dirs: &[&str]| RunWriteGrant {
        program: program.to_owned(),
        dirs: dirs
            .iter()
            .map(|dir| RunWriteDir {
                named: named.to_owned(),
                dir: (*dir).to_owned(),
                real: PathBuf::from(dir),
            })
            .collect(),
    };
    let named = spawning_plan(
        tools(&[
            ("svelte-kit", &[ToolDir::SvelteKit]),
            ("vitest", &[ToolDir::SvelteKitDev, ToolDir::Vitest]),
        ]),
        vec![
            run_write("git", "git", &["/p", "/p/.git"]),
            run_write(
                "vi\u{1b}[2Jtest",
                "vi\u{1b}[2Jtest",
                &["/p/node_modules", "/o"],
            ),
            run_write(
                "node_modules/.bin/vitest",
                "vitest",
                &["/p/lib", "/p/.svelte-kit/generated/dev"],
            ),
        ],
    );
    let cause = super::denied_write_cause(&check, &programs, false);
    assert_eq!(
        cause.cause,
        format!(
            "the run's kernel box lets {BINARY_NAME} check write only to out, .svelte-kit, \
             .svelte-kit/generated/dev, and a temporary directory of the run's; .git, \
             node_modules, the other dot-entries, and a new top-level entry stay read-only"
        )
    );
    assert_eq!(
        cause.writable,
        [
            "/p/src",
            "/p/.git",
            "/p/node_modules",
            "/o",
            "/p/lib",
            "/p/.svelte-kit/generated/dev",
            "/p/out",
            "/p/.svelte-kit",
            "/p/.svelte-kit/generated/dev",
            "/tmp/gro-1-2"
        ]
        .map(PathBuf::from)
    );
    // the entries are the realm's where it writes the root
    assert!(
        super::denied_write_cause(&check, &programs, true)
            .cause
            .contains("write only to the top-level entries the project had as it started"),
    );
    assert_eq!(
        super::walls_note(&check, &programs, &named, &RunNetwork::Namespace),
        format!(
            "note: {BINARY_NAME} check's tools each ran in a kernel box of their own inside the \
             run's, which lets them reach no network but the run's own loopback and write only \
             to a temporary directory of their own and to their own directories (git: the \
             top-level entries the project had as it started (but .git, node_modules, the \
             dot-entries, symlinks), .git; svelte-kit: .svelte-kit; vi\\u{{1b}}[2Jtest: \
             node_modules, /o; vitest: .svelte-kit/generated/dev, lib): a tool that hits a wall \
             fails with EACCES, EPERM, or ENOSYS; --allow-run-write=<program>:<dirs> lets one of \
             them (git or vitest) write more, --allow-read= and --allow-run-in= widen every box, \
             and --allow-write= widens only what gro's own code writes"
        )
    );
    // a lone program, and none
    let mut alone = check.clone();
    alone.may_start = set(&["vitest"]);
    assert!(
        super::walls_note(&alone, &checking, &table, &RunNetwork::Namespace)
            .contains("lets one of them (vitest) write more"),
    );
    alone.may_start = set(&["eslint", "git", "vitest"]);
    assert!(
        super::walls_note(&alone, &checking, &table, &RunNetwork::Namespace)
            .contains("lets one of them (eslint, git, or vitest) write more"),
    );
    // only programs found beneath an exec root, which no name reaches and
    // which write nothing of their own
    alone.may_start = BTreeSet::new();
    let found = spawning_plan(BTreeMap::new(), Vec::new());
    assert_eq!(
        super::walls_note(&alone, &checking, &found, &RunNetwork::Namespace),
        format!(
            "note: {BINARY_NAME} check's tools each ran in a kernel box of their own inside the \
             run's, which lets them reach no network but the run's own loopback and write only \
             to a temporary directory of their own: a tool that hits a wall fails with EACCES, \
             EPERM, or ENOSYS; --allow-read= and --allow-run-in= widen every box, and \
             --allow-write= widens only what gro's own code writes"
        )
    );
    let inet = super::walls_note(
        &check,
        &[wall("/p", RuleSource::Project, false)],
        &found,
        &RunNetwork::InetRefused {
            cause: gro_sys::namespace::Refusal::Filtered,
            errno: None,
        },
    );
    assert!(
        inet.contains("which lets them reach no network and write nothing: a tool"),
        "{inet}"
    );
}

/// A temporary directory lies in the project by its text, or by its
/// identity up its ancestors where another path names the project (here
/// `/proc/self/root`, as a bind mount would), and not otherwise.
#[test]
fn a_temporary_directory_lies_in_the_project_by_text_or_identity() {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("gro_cli_lies_in_{}", std::process::id()));
    let root = base.join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(base.join("outside")).unwrap();
    let other_name = Path::new("/proc/self/root").join(root.strip_prefix("/").unwrap());
    let cases = [
        (root.join("src"), true),
        (root.clone(), true),
        (other_name.join("src"), true),
        (base.join("outside"), false),
        (base.clone(), false),
    ];
    let found: Vec<bool> = cases
        .iter()
        .map(|(path, _)| super::lies_in(path, &root))
        .collect();
    std::fs::remove_dir_all(&base).unwrap();
    assert_eq!(found, cases.map(|(_, expected)| expected));
}

/// A rule holding the temporary directory is refused naming it: a flag by
/// its spelling and the fix of narrowing it, any other rule by its path,
/// and the variable to move, or `TMPDIR` for the default; each path
/// escaped.
#[test]
fn a_rule_holding_the_temporary_directory_names_its_fix() {
    let held = |rule: &str, holder, variable| {
        BoxStartError::TempHeldByRule {
            path: PathBuf::from("/tmp\u{1b}[2J"),
            rule: PathBuf::from(rule),
            holder,
            variable,
        }
        .to_string()
    };
    assert_eq!(
        held("/", TempHolder::Flag(DirFlag::RunIn), Some("TMP")),
        "the kernel box makes the run's private directory in the OS's temporary directory, \
         /tmp\\u{1b}[2J, and --allow-run-in=/ holds it, so every program the run starts could \
         read the others' temporary directories there: narrow --allow-run-in to a directory \
         that doesn't hold /tmp\\u{1b}[2J, or point TMP elsewhere"
    );
    assert_eq!(
        held(
            "/tmp",
            TempHolder::RunWrite {
                program: "vitest\u{1b}".to_owned()
            },
            Some("TMPDIR")
        ),
        "the kernel box makes the run's private directory in the OS's temporary directory, \
         /tmp\\u{1b}[2J, and --allow-run-write=vitest\\u{1b}:/tmp holds it, so vitest\\u{1b} \
         could read every other program's temporary directory there: narrow vitest\\u{1b}'s \
         directories to ones that don't hold /tmp\\u{1b}[2J, or point TMPDIR elsewhere"
    );
    assert_eq!(
        held("/usr", TempHolder::Rule, None),
        "the kernel box makes the run's private directory in the OS's temporary directory, \
         /tmp\\u{1b}[2J, and the box's rule on /usr, which every program the run starts reads, \
         holds it, so each could read the others' temporary directories there: it's the \
         default, so set TMPDIR elsewhere"
    );
}

/// A box that fails to go up says the failure's cause once: the wrapped
/// error's message holds it already, so the variant names no source for the
/// box's start to print again.
#[test]
fn a_box_that_fails_to_go_up_says_its_cause_once() {
    use gro_runtime::confine::seccomp::tree::TreeError;

    assert_eq!(
        BoxStartError::Confine(ConfineError::Denylist(TreeError::Twice { nr: 1 })).to_string(),
        "failed to assemble the run's seccomp denylist: the call number 1 is classified twice"
    );
}

/// A program's write directory holding the temporary directory is named by
/// the spelling of the flag that named that directory, where two flags
/// name one program two ways, not by the program's first.
#[test]
fn a_held_temporary_directory_names_the_flag_that_holds_it() {
    let dir = |named: &str, dir: &str| RunWriteDir {
        named: named.to_owned(),
        dir: dir.to_owned(),
        real: PathBuf::from(dir),
    };
    let inputs = BoxInputs {
        root: PathBuf::from("/p"),
        spawns: true,
        run_writes: vec![RunWriteGrant {
            program: "node_modules/.bin/vitest".to_owned(),
            dirs: vec![
                dir("vitest", "/p/src"),
                dir("node_modules/.bin/vitest", "/tmp"),
            ],
        }],
        ..BoxInputs::default()
    };
    let plan = BoxPlan::for_run(&inputs, &BoxFacts::default()).unwrap();
    let (rule, from) = plan.holding(Path::new("/tmp/gro-1")).unwrap();
    assert_eq!(rule, Path::new("/tmp"));
    let holder = TempHolder::of(from, &rule, &plan);
    assert_eq!(
        holder,
        TempHolder::RunWrite {
            program: "node_modules/.bin/vitest".to_owned()
        }
    );
    assert!(
        BoxStartError::TempHeldByRule {
            path: PathBuf::from("/tmp"),
            rule,
            holder,
            variable: None,
        }
        .to_string()
        .contains("and --allow-run-write=node_modules/.bin/vitest:/tmp holds it")
    );
}
