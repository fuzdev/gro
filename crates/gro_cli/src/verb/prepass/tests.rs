use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gro_runtime::resolver::{InstalledPackage, ProjectPackage, ResolveErrorKind};
use gro_runtime::verb::table::BuiltinGrantsTable;
use gro_runtime::{EnvPattern, PassEnvGrant, ProgramGrant};

use super::{
    ProjectReads, TableFault, VerbError, VerbFallback, VerbInputs, VerbPlan, VerbRun, plan,
};
use crate::cli::{FlagDirs, GrantListError, RunGrant};
use crate::verb::fallback::FallbackReason;
use crate::verb::grant_flags::GrantFlagsError;

/// The binary's version in these tests, which no installed Gro's is unless
/// a test plants it.
const VERSION: &str = "1.2.3";

/// The project root the tests read, which nothing reads from disk.
const ROOT: &str = "/p";

/// A project's files as a test gives them.
struct Project {
    /// The root's `package.json`.
    package: Result<Option<ProjectPackage>, ResolveErrorKind>,
    /// The installed Gro.
    installed: Result<Option<InstalledPackage>, ResolveErrorKind>,
}

impl ProjectReads for Project {
    fn package(&self) -> Result<Option<ProjectPackage>, ResolveErrorKind> {
        self.package.clone()
    }

    fn installed_gro(&self) -> Result<Option<InstalledPackage>, ResolveErrorKind> {
        self.installed.clone()
    }
}

/// A project's `package.json`, a type marker or not.
fn package(type_marker: bool) -> ProjectPackage {
    ProjectPackage {
        path: PathBuf::from("/p/package.json"),
        declared: BTreeSet::new(),
        type_marker,
    }
}

/// A project with its own `package.json`, and Gro installed at `version`,
/// or none.
fn project(gro: Option<&str>) -> Project {
    Project {
        package: Ok(Some(package(false))),
        installed: Ok(gro.map(gro_at)),
    }
}

/// Gro installed in the project at `version`.
fn gro_at(version: &str) -> InstalledPackage {
    InstalledPackage {
        root: PathBuf::from("/p/node_modules/@fuzdev/gro"),
        version: Some(version.to_owned()),
    }
}

/// What the process gave, beside the command line.
#[derive(Default, Clone, Copy)]
struct Env<'a> {
    fallback: Option<&'a str>,
    node_env: Option<&'a str>,
}

fn plan_with(
    cli_args: &[&str],
    env: Env<'_>,
    table: &BuiltinGrantsTable,
    project: &Project,
) -> Result<VerbPlan, VerbError> {
    let cli_args: Vec<String> = cli_args.iter().map(|&arg| arg.to_owned()).collect();
    let inputs = VerbInputs {
        cli_args: &cli_args,
        fallback: env.fallback.map(OsStr::new),
        node_env: env.node_env.map(OsStr::new),
        home: Some(OsStr::new("/home/u")),
        binary_version: VERSION,
        root: Path::new(ROOT),
    };
    plan(&inputs, table, project)
}

/// The plan of `cli_args` on the carried table, in a project that installs
/// the binary's own Gro.
fn planned(cli_args: &[&str]) -> Result<VerbPlan, VerbError> {
    let table = BuiltinGrantsTable::carried().unwrap();
    plan_with(cli_args, Env::default(), &table, &project(Some(VERSION)))
}

fn run(cli_args: &[&str]) -> VerbRun {
    match planned(cli_args) {
        Ok(VerbPlan::Run(run)) => run,
        other => panic!("{cli_args:?} runs on the binary: {other:?}"),
    }
}

fn fallback(cli_args: &[&str]) -> VerbFallback {
    match planned(cli_args) {
        Ok(VerbPlan::Fallback(fallback)) => fallback,
        other => panic!("{cli_args:?} falls back: {other:?}"),
    }
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|&item| item.to_owned()).collect()
}

fn dirs(read: &[&str], write: &[&str]) -> FlagDirs {
    FlagDirs {
        read: set(read),
        write: set(write),
        run_in: BTreeSet::new(),
    }
}

fn named(env: &[&str], run: &[&str]) -> RunGrant {
    RunGrant::Named {
        env: set(env),
        temp: false,
        run: set(run),
        run_env: BTreeSet::new(),
    }
}

/// The table's policy of a `local` tool that passes the whole environment.
fn whole(env: &[(&str, &str)]) -> ProgramGrant {
    ProgramGrant::Table {
        pass_env: PassEnvGrant::All {
            omit_env: BTreeSet::new(),
        },
        env: env
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
    }
}

/// git's policy in `check`, with what the flags forward.
fn git(forwards: &[EnvPattern]) -> ProgramGrant {
    let mut patterns = BTreeSet::from([
        EnvPattern::Prefix("GIT_".to_owned()),
        EnvPattern::Exact("XDG_CONFIG_HOME".to_owned()),
        EnvPattern::Exact("SUDO_UID".to_owned()),
    ]);
    patterns.extend(forwards.iter().cloned());
    ProgramGrant::Table {
        pass_env: PassEnvGrant::Names(patterns.into_iter().collect()),
        env: BTreeMap::new(),
    }
}

/// A command's grants are its closure's: the listing reads the project,
/// `format --check` and `gen --check` without their writes, `gen` with
/// them, and `check` its whole closure's programs under the names Gro's TS
/// asks for them by, each by the table's policy, with no write.
#[test]
fn a_command_runs_with_its_closure_s_grants() {
    let listing = run(&[]);
    assert_eq!(listing.grant, named(&[], &[]));
    assert_eq!(listing.dirs, dirs(&["."], &[]));
    assert!(listing.programs.is_empty() && !listing.passes_whole_environment());
    assert_eq!(listing.cli_args, Vec::<String>::new());

    let format = run(&["format", "--check"]);
    assert_eq!(format.dirs, dirs(&["."], &[]));
    assert_eq!(format.grant, named(&[], &[]));
    assert_eq!(run(&["format"]).dirs, dirs(&["."], &["."]));

    let gen_check = run(&["gen", "--check"]);
    assert_eq!(gen_check.dirs, dirs(&["."], &[]));
    assert_eq!(
        gen_check.programs,
        BTreeMap::from([("node_modules/.bin/svelte-kit".to_owned(), whole(&[]))])
    );
    assert!(gen_check.passes_whole_environment());
    assert_eq!(run(&["gen"]).dirs, dirs(&["."], &["."]));

    let check = run(&["check"]);
    assert_eq!(check.dirs, dirs(&["."], &[]));
    let tools = [
        "node_modules/.bin/eslint",
        "node_modules/.bin/svelte-check",
        "node_modules/.bin/svelte-kit",
        "node_modules/.bin/tsc",
        "node_modules/.bin/vitest",
    ];
    let mut expected: BTreeMap<String, ProgramGrant> = tools
        .iter()
        .map(|&tool| (tool.to_owned(), whole(&[])))
        .collect();
    expected.insert(
        "node_modules/.bin/svelte-check".to_owned(),
        whole(&[("FORCE_COLOR", "1")]),
    );
    expected.insert("git".to_owned(), git(&[]));
    assert_eq!(check.programs, expected);
    let mut run_names = tools.to_vec();
    run_names.push("git");
    assert_eq!(check.grant, named(&["CI"], &run_names));
}

/// The flags join the closure's grants, a flag's program the table also
/// grants taking the table's policy, its forwards joining a list and adding
/// nothing to the whole environment; the run's args are the command line
/// without them; and `-A` runs unconfined.
#[test]
fn the_flags_join_the_closure_s_grants() {
    let check = run(&[
        "check",
        "--allow-env=X",
        "--allow-read=r",
        "--allow-write=w",
        "--allow-run=git,node_modules/.bin/vitest,cargo",
        "--allow-run-env=FOO,BAR_*",
        "--allow-temp",
        "--no-lint",
    ]);
    assert_eq!(check.cli_args, ["check", "--no-lint"]);
    assert_eq!(check.dirs, dirs(&[".", "r"], &["w"]));
    let RunGrant::Named {
        env,
        temp,
        run: run_names,
        run_env,
    } = &check.grant
    else {
        panic!("confined");
    };
    assert_eq!(*env, set(&["CI", "X"]));
    assert!(*temp);
    assert!(
        run_names.contains("cargo")
            && run_names.contains("git")
            && run_names.contains("node_modules/.bin/tsc")
    );
    assert_eq!(
        *run_env,
        BTreeSet::from([
            EnvPattern::Exact("FOO".to_owned()),
            EnvPattern::Prefix("BAR_".to_owned())
        ])
    );
    assert_eq!(
        check.programs["git"],
        git(&[
            EnvPattern::Exact("FOO".to_owned()),
            EnvPattern::Prefix("BAR_".to_owned())
        ])
    );
    assert_eq!(check.programs["node_modules/.bin/vitest"], whole(&[]));
    // a program only a flag grants takes the flag's grant, as `fgro run`'s
    assert!(!check.programs.contains_key("cargo"));
    // a listed program the flags don't name gets none of their forwards
    let other = run(&["check", "--allow-run=cargo", "--allow-run-env=FOO"]);
    assert_eq!(other.programs["git"], git(&[]));
    assert!(!other.programs.contains_key("cargo"));
    // a flag spelled otherwise than the table's name is a program of its own
    let test = run(&["test", "--allow-run=./node_modules/.bin/vitest"]);
    assert!(!test.programs.contains_key("./node_modules/.bin/vitest"));
    assert_eq!(test.programs["node_modules/.bin/vitest"], whole(&[]));

    assert_eq!(run(&["check", "-A"]).grant, RunGrant::All);
}

/// A flag's list is checked as `fgro run` checks it, only when the command
/// runs on the binary: a fallback hands the command line to the Node Gro.
#[test]
fn a_flag_s_list_is_checked_for_a_run_alone() {
    assert!(matches!(
        planned(&["check", "--allow-read=,"]),
        Err(VerbError::GrantFlags(GrantFlagsError::List(
            GrantListError::Dir(..)
        )))
    ));
    assert_eq!(
        fallback(&["dev", "--allow-read=,"]).command.as_deref(),
        Some("dev")
    );
    // the flags' spelling is refused before anything else is read
    assert!(matches!(
        planned(&["dev", "--allow-temp=x"]),
        Err(VerbError::GrantFlags(GrantFlagsError::TakesNoValue { .. }))
    ));
}

/// A command whose closure or policy leaves it to Node falls back, each
/// reason decided by the command line, and `task` with no task's path lists
/// on Node.
#[test]
fn a_command_falls_back_for_its_closure_and_its_policies() {
    let reasons = |cli_args: &[&str]| fallback(cli_args).reasons;
    assert_eq!(
        reasons(&["check", "--build"]),
        [FallbackReason::Ungranted {
            builtin: "build".to_owned(),
            why: Some("it runs Vite's build with the project's plugins and adapter"),
        }]
    );
    assert_eq!(reasons(&["sync", "--install"]), [FallbackReason::Install]);
    assert_eq!(
        reasons(&["task", "src/a.task.ts"]),
        [FallbackReason::ProjectTask]
    );
    assert_eq!(reasons(&["task"]), [FallbackReason::TaskListing]);
    // the command word is the closure's, past a flag before it
    assert_eq!(reasons(&["--x=1", "task"]), [FallbackReason::TaskListing]);
    assert_eq!(fallback(&["--x=1", "dev"]).command.as_deref(), Some("dev"));
    assert_eq!(
        fallback(&["--x", "--y=1", "task", "a.task.ts"]).reasons,
        [FallbackReason::ProjectTask]
    );
    assert_eq!(reasons(&["task", "--help"]), [FallbackReason::TaskListing]);
    let dev = fallback(&["dev"]);
    assert_eq!(dev.command.as_deref(), Some("dev"));
    assert_eq!(dev.gro, gro_at(VERSION));
    assert_eq!(
        FallbackReason::TaskListing.to_string(),
        "`task` lists the project's tasks by importing each, and their grants are in no table"
    );
}

/// Another installed Gro falls back, decided by the project's files, which
/// `GRO_FALLBACK=command` refuses, and only those; a command with no Gro
/// installed to fall back to is refused.
#[test]
fn the_fallback_policy_refuses_what_the_project_s_files_decide() {
    let table = BuiltinGrantsTable::carried().unwrap();
    let older = project(Some("1.2.2"));
    let command = Env {
        fallback: Some("command"),
        ..Env::default()
    };
    let Ok(VerbPlan::Fallback(mismatch)) = plan_with(&["check"], Env::default(), &table, &older)
    else {
        panic!("another version falls back");
    };
    assert_eq!(
        mismatch.reasons,
        [FallbackReason::VersionMismatch {
            installed: Some("1.2.2".to_owned()),
            binary: VERSION.to_owned(),
        }]
    );
    let all = Env {
        fallback: Some("all"),
        ..Env::default()
    };
    assert!(matches!(
        plan_with(&["check"], all, &table, &older),
        Ok(VerbPlan::Fallback(_))
    ));
    let refused = plan_with(&["check"], command, &table, &older).unwrap_err();
    assert!(matches!(refused, VerbError::Strict { .. }), "{refused}");
    assert_eq!(refused.exit_code(), 2);
    assert_eq!(
        refused.to_string(),
        "GRO_FALLBACK=command refuses running the project's own gro, which its files pick: the \
         project installed @fuzdev/gro 1.2.2, and this is 1.2.3, so the project's own runs. \
         Install @fuzdev/gro@1.2.3 to run this one, or run node_modules/.bin/gro check for the \
         project's, unconfined"
    );
    // the whole command line, its grant flags split off, each argument as a
    // shell reads it back, so a check never names a writing command
    let quoted = plan_with(
        &[
            "format",
            "--allow-read=.",
            "--check",
            "--",
            "a b",
            "it's",
            "",
            "x\u{1b}[2J",
        ],
        command,
        &table,
        &older,
    )
    .unwrap_err();
    assert!(
        quoted.to_string().ends_with(
            "or run node_modules/.bin/gro format --check -- 'a b' 'it'\\''s' '' \
             'x\\u{1b}[2J' for the project's, unconfined"
        ),
        "{quoted}"
    );
    // a reason the project's files decide beside the command's is refused too
    assert!(matches!(
        plan_with(&["dev"], command, &table, &older),
        Err(VerbError::Strict { .. })
    ));
    // the command's own reasons, and a run on the binary, pass
    let current = project(Some(VERSION));
    assert!(matches!(
        plan_with(&["dev"], command, &table, &current),
        Ok(VerbPlan::Fallback(_))
    ));
    assert!(matches!(
        plan_with(&["check"], command, &table, &current),
        Ok(VerbPlan::Run(_))
    ));
    // no Gro installed: the binary runs what it can, and refuses a fallback
    let none = project(None);
    assert!(matches!(
        plan_with(&["check"], command, &table, &none),
        Ok(VerbPlan::Run(_))
    ));
    let no_gro = plan_with(&["dev"], Env::default(), &table, &none).unwrap_err();
    assert_eq!(
        no_gro.to_string(),
        "fgro dev runs on the Node gro, because `dev` declares no grants: it serves and watches \
         the project with Vite and its plugins, and this project installs none: npm i -D \
         @fuzdev/gro"
    );
    assert_eq!(no_gro.exit_code(), 2);
}

/// `GRO_FALLBACK` is read first: any value but `all` and `command`, the
/// empty one included, is refused before the flags or the project.
#[test]
fn an_unknown_fallback_policy_is_refused_first() {
    let table = BuiltinGrantsTable::carried().unwrap();
    let no_project = Project {
        package: Ok(None),
        installed: Ok(None),
    };
    for value in ["", "none", "Command", "all ", "\u{1b}[2J"] {
        let env = Env {
            fallback: Some(value),
            ..Env::default()
        };
        let error = plan_with(&["check", "--allow-temp=x"], env, &table, &no_project).unwrap_err();
        assert!(matches!(error, VerbError::FallbackPolicy(_)), "{value:?}");
        assert_eq!(error.exit_code(), 2);
    }
    let env = Env {
        fallback: Some("\u{1b}[2J"),
        ..Env::default()
    };
    assert_eq!(
        plan_with(&[], env, &table, &no_project)
            .unwrap_err()
            .to_string(),
        "GRO_FALLBACK is \"\\u{1b}[2J\", and it takes all or command"
    );
}

/// The working directory is the project's root, and must hold a
/// `package.json` that isn't a type marker.
#[test]
fn the_root_holds_a_project_s_package_json() {
    let table = BuiltinGrantsTable::carried().unwrap();
    let read = |package| {
        plan_with(
            &["check"],
            Env::default(),
            &table,
            &Project {
                package,
                installed: Ok(None),
            },
        )
    };
    let none = read(Ok(None)).unwrap_err();
    assert!(matches!(none, VerbError::NoPackageJson { .. }));
    assert_eq!(
        none.to_string(),
        "fgro check runs the project at the working directory, and /p holds no package.json"
    );
    let no_project = Project {
        package: Ok(None),
        installed: Ok(None),
    };
    let message = |cli_args: &[&str]| {
        plan_with(cli_args, Env::default(), &table, &no_project)
            .unwrap_err()
            .to_string()
    };
    // with no command word, a pointer to the binary's own help
    assert_eq!(
        message(&[]),
        "fgro runs the project at the working directory, and /p holds no package.json; fgro \
         --help lists the binary's own commands"
    );
    assert_eq!(
        message(&["--x=1", "check"]),
        "fgro check runs the project at the working directory, and /p holds no package.json"
    );
    assert!(message(&["src/a.task.ts"]).ends_with("own commands"));
    let marker = read(Ok(Some(package(true)))).unwrap_err();
    assert!(matches!(marker, VerbError::TypeMarker { .. }));
    assert_eq!(marker.exit_code(), 2);
    let unreadable = read(Err(ResolveErrorKind::InvalidPackageName)).unwrap_err();
    assert!(matches!(unreadable, VerbError::Project { .. }));
    assert!(matches!(
        read(Ok(Some(package(false)))),
        Ok(VerbPlan::Run(_))
    ));
}

/// The `development` condition is matched unless `NODE_ENV` is
/// `production`, and the entry names the binary's version and `HOME`.
#[test]
fn the_run_s_condition_and_entry() {
    let table = BuiltinGrantsTable::carried().unwrap();
    let current = project(Some(VERSION));
    let development = |node_env| {
        let env = Env {
            node_env,
            ..Env::default()
        };
        match plan_with(&["check"], env, &table, &current) {
            Ok(VerbPlan::Run(run)) => run.development,
            other => panic!("{other:?}"),
        }
    };
    assert!(development(None));
    assert!(development(Some("development")));
    assert!(development(Some("Production")));
    assert!(!development(Some("production")));
    let entry = run(&["check"]).entry;
    assert!(entry.contains("\thome: '/home/u',\n"), "{entry}");
    assert!(entry.contains("\tversion: '1.2.3'\n"), "{entry}");
}

/// A table the TS would refuse, or that grants one program two ways in a
/// closure, is refused: Gro's drift, not the command line's.
#[test]
fn a_table_that_grants_a_program_two_ways_is_refused() {
    let table = |programs_a: &str, programs_b: &str| {
        BuiltinGrantsTable::parse(&format!(
            r#"{{"description": "", "cli": {{}}, "builtins": {{
                "a": {{"programs": {programs_a}, "invokes": {{"b": {{}}}}}},
                "b": {{"programs": {programs_b}}}
            }}}}"#
        ))
        .unwrap()
    };
    let refused =
        |table: &BuiltinGrantsTable| match plan_with(&["a"], Env::default(), table, &project(None))
        {
            Err(VerbError::Table { program, fault }) => (program, fault),
            other => panic!("{other:?}"),
        };
    let two = table(
        r#"{"git_cli": {"name": "git", "pass_env": ["GIT_*"]}}"#,
        r#"{"git_cli": {"name": "git", "pass_env": "all"}}"#,
    );
    assert_eq!(refused(&two), ("git".to_owned(), TableFault::TwoPolicies));
    // two roles naming one program alike are one grant
    let alike = table(
        r#"{"a_cli": {"name": "x", "local": true, "pass_env": "all"}}"#,
        r#"{"b_cli": {"name": "x", "local": true, "pass_env": "all"}}"#,
    );
    let Ok(VerbPlan::Run(run)) = plan_with(&["a"], Env::default(), &alike, &project(None)) else {
        panic!("alike grants run");
    };
    assert_eq!(
        run.programs,
        BTreeMap::from([("node_modules/.bin/x".to_owned(), whole(&[]))])
    );
    let omit = table(
        r#"{"x_cli": {"name": "x", "pass_env": ["A"], "omit_env": ["B"]}}"#,
        "{}",
    );
    assert_eq!(refused(&omit), ("x".to_owned(), TableFault::OmitBesideList));
    let pattern = table(r#"{"x_cli": {"name": "x", "pass_env": ["*"]}}"#, "{}");
    assert_eq!(
        refused(&pattern),
        ("x".to_owned(), TableFault::Pattern("*".to_owned()))
    );
    let error = plan_with(&["a"], Env::default(), &two, &project(None)).unwrap_err();
    assert_eq!(error.exit_code(), 1);
    // an `'all'` with an omit_env passes it whole but those names, and a
    // grant with no pass_env forwards what the flags forward when they name
    // it too
    let omitting = table(
        r#"{"npm_cli": {"name": "npm", "pass_env": "all", "omit_env": ["NODE_ENV"]}, "y_cli": {"name": "y"}}"#,
        "{}",
    );
    let cli_args = ["a", "--allow-run=y,npm", "--allow-run-env=Z"];
    let Ok(VerbPlan::Run(run)) = plan_with(&cli_args, Env::default(), &omitting, &project(None))
    else {
        panic!("runs");
    };
    assert_eq!(
        run.programs["npm"],
        ProgramGrant::Table {
            pass_env: PassEnvGrant::All {
                omit_env: set(&["NODE_ENV"]),
            },
            env: BTreeMap::new(),
        }
    );
    assert_eq!(
        run.programs["y"],
        ProgramGrant::Table {
            pass_env: PassEnvGrant::Names(vec![EnvPattern::Exact("Z".to_owned())]),
            env: BTreeMap::new(),
        }
    );
    let cli_args = ["a", "--allow-run=other", "--allow-run-env=Z"];
    let Ok(VerbPlan::Run(run)) = plan_with(&cli_args, Env::default(), &omitting, &project(None))
    else {
        panic!("runs");
    };
    assert_eq!(
        run.programs["y"],
        ProgramGrant::Table {
            pass_env: PassEnvGrant::Names(Vec::new()),
            env: BTreeMap::new(),
        }
    );
}
