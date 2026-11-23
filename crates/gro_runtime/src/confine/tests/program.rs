//! Tests for a program's own box that hold without installing it: what it
//! takes of the run's rules, the handles the run's box keeps for it, and
//! its temporary directory. The box installed is tested from inside a
//! process holding the run's, harness off (`gro_cli`'s `tests/run_box.rs`).

use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use super::Scratch;
use crate::confine::plan::{DirFlag, RuleSource, ToolDir};
use crate::confine::program::{
    GrantedBy, ProgramBox, ProgramBoxError, ProgramRules, program_rights,
};
use crate::confine::rules::{BoxRules, InstalledRule, Need, Rights};

/// A program reads and executes what the run does, never writes
/// it, and takes nothing of the private directory; the devices it takes as
/// the run has them, `/dev/null` written; a tool directory it writes only
/// as a program of the table.
#[test]
fn a_program_takes_the_run_s_rules_but_its_writes() {
    let read = Rights::READ;
    let read_write = Rights::READ | Rights::WRITE;
    let read_execute = Rights::READ | Rights::EXECUTE;
    let all = Rights::READ | Rights::WRITE | Rights::EXECUTE;
    let tool_dir = RuleSource::ToolDir(ToolDir::SvelteKit);
    for (source, rights, table, flag) in [
        (RuleSource::Project, read, Some(read), Some(read)),
        (RuleSource::Entry, Rights::WRITE, None, None),
        (tool_dir, Rights::WRITE, Some(Rights::WRITE), None),
        (RuleSource::PrivateTemp, all, None, None),
        (
            RuleSource::Device,
            read_write,
            Some(read_write),
            Some(read_write),
        ),
        (RuleSource::Device, read, Some(read), Some(read)),
        (
            RuleSource::Flag(DirFlag::Write),
            read_write,
            Some(read),
            Some(read),
        ),
        (
            RuleSource::Flag(DirFlag::RunIn),
            read_execute,
            Some(read_execute),
            Some(read_execute),
        ),
        (
            RuleSource::NodeModules,
            read_execute,
            Some(read_execute),
            Some(read_execute),
        ),
        (
            RuleSource::System,
            read_execute,
            Some(read_execute),
            Some(read_execute),
        ),
        (
            RuleSource::Program,
            read_execute,
            Some(read_execute),
            Some(read_execute),
        ),
        (RuleSource::Proc, read, Some(read), Some(read)),
        (RuleSource::GitHome, read, Some(read), Some(read)),
    ] {
        assert_eq!(
            program_rights(source, rights, GrantedBy::Table),
            table,
            "{source:?} {rights:?}"
        );
        assert_eq!(
            program_rights(source, rights, GrantedBy::Flag),
            flag,
            "{source:?} {rights:?}"
        );
    }
}

/// The rules of a run's box, opened in `scratch`: the project read, an
/// entry and a tool directory written, a write flag's directory, the
/// private directory, and `/dev/null`, opened beneath `/` as the plan opens
/// it.
fn run_rules(scratch: &Scratch) -> BoxRules {
    for dir in ["project/src", "project/.svelte-kit", "flagged", "private"] {
        scratch.dir(dir);
    }
    let base = std::fs::File::open(scratch.path()).unwrap();
    let slash = std::fs::File::open("/").unwrap();
    let mut rules = BoxRules::new();
    for (path, rights, source) in [
        ("project", Rights::READ, RuleSource::Project),
        ("project/src", Rights::WRITE, RuleSource::Entry),
        (
            "project/.svelte-kit",
            Rights::WRITE,
            RuleSource::ToolDir(ToolDir::SvelteKit),
        ),
        (
            "flagged",
            Rights::READ | Rights::WRITE,
            RuleSource::Flag(DirFlag::Write),
        ),
        (
            "private",
            Rights::READ | Rights::WRITE | Rights::EXECUTE,
            RuleSource::PrivateTemp,
        ),
    ] {
        rules
            .beneath(
                base.as_fd(),
                scratch.path(),
                Path::new(path),
                rights,
                Need::Required,
                source,
            )
            .unwrap();
    }
    rules
        .beneath(
            slash.as_fd(),
            Path::new("/"),
            Path::new("dev/null"),
            Rights::READ | Rights::WRITE,
            Need::Required,
            RuleSource::Device,
        )
        .unwrap();
    rules
}

/// What a box for `program` grants beneath the run's rules, by path.
fn granted(programs: &ProgramRules, program: GrantedBy) -> Vec<(PathBuf, Rights)> {
    programs
        .granted(program)
        .map(|(rule, rights)| (rule.path().to_owned(), rights))
        .collect()
}

/// A run that may start a program keeps every rule as data, and holds the
/// handles some program's box takes, the private directory's among them:
/// a program of the table reads the project and the write flag's
/// directory, writes the tool directory and `/dev/null`, and one a flag
/// grants writes only `/dev/null`; neither takes the entry it writes or the
/// private directory.
#[test]
fn a_spawning_run_holds_what_its_programs_boxes_take() {
    let scratch = Scratch::new("program_hold");
    let at = |path: &str| scratch.path().join(path);
    let (rules, absent) = run_rules(&scratch).into_installed();
    assert!(absent.is_empty());
    let (installed, programs) = ProgramRules::hold(rules, true);
    let sources: Vec<_> = installed.iter().map(InstalledRule::source).collect();
    assert_eq!(
        sources,
        [
            RuleSource::Project,
            RuleSource::Entry,
            RuleSource::ToolDir(ToolDir::SvelteKit),
            RuleSource::Flag(DirFlag::Write),
            RuleSource::PrivateTemp,
            RuleSource::Device,
        ]
    );
    let programs = programs.unwrap();
    let null = Path::new("/").join("dev/null");
    assert_eq!(
        granted(&programs, GrantedBy::Table),
        [
            (at("project"), Rights::READ),
            (at("project/.svelte-kit"), Rights::WRITE),
            (at("flagged"), Rights::READ),
            (null.clone(), Rights::READ | Rights::WRITE),
        ]
    );
    assert_eq!(
        granted(&programs, GrantedBy::Flag),
        [
            (at("project"), Rights::READ),
            (at("flagged"), Rights::READ),
            (null, Rights::READ | Rights::WRITE),
        ]
    );
}

/// A rule on the private directory's parent, or on any directory holding
/// it, is held for no program, whatever planned it, since it would grant
/// every program all of the private directory; the rest are held as
/// before, and no held rule is the private directory's own.
#[test]
fn no_rule_holding_the_private_directory_is_held() {
    let scratch = Scratch::new("program_holding");
    let mut rules = run_rules(&scratch);
    let slash = std::fs::File::open("/").unwrap();
    let base = std::fs::File::open(scratch.path()).unwrap();
    rules
        .beneath(
            base.as_fd(),
            scratch.path(),
            Path::new("."),
            Rights::READ,
            Need::Required,
            RuleSource::Flag(DirFlag::Read),
        )
        .unwrap();
    rules
        .beneath(
            slash.as_fd(),
            Path::new("/"),
            Path::new("."),
            Rights::READ | Rights::EXECUTE,
            Need::Required,
            RuleSource::Flag(DirFlag::RunIn),
        )
        .unwrap();
    let (rules, _) = rules.into_installed();
    let (installed, programs) = ProgramRules::hold(rules, true);
    assert_eq!(installed.len(), 8);
    let programs = programs.unwrap();
    for program in [GrantedBy::Table, GrantedBy::Flag] {
        let sources: Vec<_> = programs
            .granted(program)
            .map(|(rule, _)| rule.source())
            .collect();
        assert!(
            !sources.contains(&RuleSource::Flag(DirFlag::Read))
                && !sources.contains(&RuleSource::Flag(DirFlag::RunIn))
                && !sources.contains(&RuleSource::PrivateTemp),
            "{program:?}: {sources:?}"
        );
    }
    assert_eq!(granted(&programs, GrantedBy::Table).len(), 4);
}

/// A run that starts nothing holds no handle past its box, nor does one
/// with no private directory, which no program's box can be made in; both
/// keep every rule as data.
#[test]
fn a_run_that_starts_nothing_holds_no_handle() {
    let scratch = Scratch::new("program_none");
    let (rules, _) = run_rules(&scratch).into_installed();
    let (installed, programs) = ProgramRules::hold(rules, false);
    assert_eq!(installed.len(), 6);
    assert!(programs.is_none());
    let (rules, _) = run_rules(&scratch).into_installed();
    let without_private: Vec<_> = rules
        .into_iter()
        .filter(|(rule, _)| rule.source() != RuleSource::PrivateTemp)
        .collect();
    let (installed, programs) = ProgramRules::hold(without_private, true);
    assert_eq!(installed.len(), 5);
    assert!(programs.is_none());
}

/// A program's box makes its temporary directory in the private directory,
/// named by its pin, its user's alone; a second box for the same pin, and
/// an entry planted at a pin's name, a symlink out of the private directory
/// among them, are refused, nothing made where the link leads.
#[test]
fn a_program_s_box_makes_its_temp_dir_by_its_pin() {
    let scratch = Scratch::new("program_temp");
    let outside = scratch.dir("outside");
    let (rules, _) = run_rules(&scratch).into_installed();
    let (_, programs) = ProgramRules::hold(rules, true);
    let programs = programs.unwrap();
    let made = programs.program_box(GrantedBy::Table, 3).unwrap();
    let temp = scratch.path().join("private/program_3");
    assert_eq!(made.temp_dir(), temp);
    let metadata = std::fs::symlink_metadata(&temp).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    let refused = |made: Result<ProgramBox, ProgramBoxError>| match made {
        Err(ProgramBoxError::TempDir { path, source }) => (path, source.raw_os_error()),
        other => panic!("not refused for its temporary directory: {other:?}"),
    };
    assert_eq!(
        refused(programs.program_box(GrantedBy::Flag, 3)),
        (temp, Some(libc::EEXIST))
    );
    scratch.link(&outside.join("planted"), "private/program_4");
    assert_eq!(
        refused(programs.program_box(GrantedBy::Flag, 4)),
        (scratch.path().join("private/program_4"), Some(libc::EEXIST))
    );
    assert!(!outside.join("planted").exists());
    assert!(programs.program_box(GrantedBy::Flag, 5).is_ok());
}

/// A program's box is built on the isolate's thread and installed on the
/// thread that launches it.
#[test]
fn a_program_s_box_crosses_threads() {
    const fn shared<T: Send + Sync>() {}
    shared::<ProgramBox>();
    shared::<ProgramRules>();
}
