//! Tests for a program's own box that hold without installing it: what it
//! takes of the run's rules, the handles the run's box keeps for it, and
//! its temporary directory. The box installed is tested from inside a
//! process holding the run's, harness off (`gro_cli`'s `tests/run_box.rs`).

use std::collections::BTreeMap;
use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::Scratch;
use crate::confine::plan::{DirFlag, RuleSource, ToolDir, ToolDirs};
use crate::confine::program::{
    ProgramBox, ProgramBoxError, ProgramBoxes, ProgramRules, ProgramWrites, program_rights,
};
use crate::confine::rules::{BoxRules, InstalledRule, Need, Rights};

/// The tool directories a program of the table writing all of
/// `.svelte-kit` writes.
fn kit_dirs() -> ToolDirs {
    std::iter::once(ToolDir::SvelteKit).collect()
}

/// The write set of a program of the table writing all of `.svelte-kit`,
/// that no `--allow-run-write=` names.
fn kit() -> ProgramWrites {
    ProgramWrites {
        tool_dirs: kit_dirs(),
        ..ProgramWrites::NONE
    }
}

/// The write set of the program `--allow-run-write=` names at `index`,
/// a tool of the table writing all of `.svelte-kit` when `table`,
/// writing the root's entries when `root`.
fn named(index: usize, table: bool, root: bool) -> ProgramWrites {
    ProgramWrites {
        tool_dirs: if table { kit_dirs() } else { ToolDirs::EMPTY },
        root,
        run_write: Some(index),
    }
}

/// A program reads and executes what the run does, never writes it but
/// by its write set, and takes nothing of the private directory or of what
/// a write flag names, read included; the devices it takes as the run has
/// them, `/dev/null` written; a tool directory it writes only as a program
/// whose write set holds it, never one another program's set holds, an entry only as one whose set names the root, and a rule
/// `--allow-run-write=` made only as the program it names, read included.
#[test]
fn a_program_takes_the_run_s_rules_but_its_writes() {
    let read = Rights::READ;
    let read_write = Rights::READ | Rights::WRITE;
    let read_execute = Rights::READ | Rights::EXECUTE;
    let all = Rights::READ | Rights::WRITE | Rights::EXECUTE;
    let tool_dir = RuleSource::ToolDir(ToolDir::SvelteKit);
    let plain = Rights::WRITE_PLAIN;
    // the table's, a flag's, and the one named at 0 writing the root
    let rooted = named(0, false, true);
    for (source, rights, table, flag, named_root) in [
        (
            RuleSource::Project,
            read,
            Some(read),
            Some(read),
            Some(read),
        ),
        (
            RuleSource::Entry,
            Rights::WRITE,
            None,
            None,
            Some(Rights::WRITE),
        ),
        (tool_dir, plain, Some(plain), None, None),
        (RuleSource::PrivateTemp, all, None, None, None),
        (
            RuleSource::Device,
            read_write,
            Some(read_write),
            Some(read_write),
            Some(read_write),
        ),
        (RuleSource::Device, read, Some(read), Some(read), Some(read)),
        (
            RuleSource::Flag(DirFlag::Write),
            read_write,
            None,
            None,
            None,
        ),
        (
            RuleSource::Flag(DirFlag::Read),
            read,
            Some(read),
            Some(read),
            Some(read),
        ),
        (
            RuleSource::Flag(DirFlag::RunIn),
            read_execute,
            Some(read_execute),
            Some(read_execute),
            Some(read_execute),
        ),
        (
            RuleSource::RunWrite(0),
            read_write,
            None,
            None,
            Some(read_write),
        ),
        (RuleSource::RunWrite(1), read_write, None, None, None),
        (
            RuleSource::NodeModules,
            read_execute,
            Some(read_execute),
            Some(read_execute),
            Some(read_execute),
        ),
        (
            RuleSource::System,
            read_execute,
            Some(read_execute),
            Some(read_execute),
            Some(read_execute),
        ),
        (
            RuleSource::Program,
            read_execute,
            Some(read_execute),
            Some(read_execute),
            Some(read_execute),
        ),
        (RuleSource::Proc, read, Some(read), Some(read), Some(read)),
        (
            RuleSource::GitHome,
            read,
            Some(read),
            Some(read),
            Some(read),
        ),
    ] {
        assert_eq!(
            program_rights(source, rights, kit()),
            table,
            "{source:?} {rights:?}"
        );
        assert_eq!(
            program_rights(source, rights, ProgramWrites::NONE),
            flag,
            "{source:?} {rights:?}"
        );
        assert_eq!(
            program_rights(source, rights, rooted),
            named_root,
            "{source:?} {rights:?}"
        );
    }
    // a program writing other tool directories writes none of this one
    let vitest = ProgramWrites {
        tool_dirs: [
            ToolDir::SvelteKitDev,
            ToolDir::SvelteKitTypes,
            ToolDir::App,
            ToolDir::ViteTemp,
            ToolDir::Vitest,
        ]
        .into_iter()
        .collect(),
        ..ProgramWrites::NONE
    };
    assert_eq!(program_rights(tool_dir, plain, vitest), None);
    assert_eq!(
        program_rights(RuleSource::ToolDir(ToolDir::SvelteKitDev), plain, vitest),
        Some(plain)
    );
    assert_eq!(
        program_rights(RuleSource::ToolDir(ToolDir::SvelteKitDev), plain, kit()),
        None
    );
    // a named tool of the table writes its tool directories beside its own
    assert_eq!(
        program_rights(tool_dir, plain, named(1, true, false)),
        Some(plain)
    );
    // a program takes a rule's writing as the run's box has it, plain or
    // whole, never widening a plain one
    assert!(plain.writes() && !plain.writes_special() && Rights::WRITE.writes_special());
    assert_eq!(
        program_rights(RuleSource::RunWrite(0), Rights::READ | plain, rooted),
        Some(Rights::READ | plain)
    );
    assert!(
        program_rights(RuleSource::Entry, Rights::WRITE, rooted)
            .is_some_and(Rights::writes_special)
    );
    assert_eq!(
        program_rights(RuleSource::RunWrite(1), read_write, named(1, true, false)),
        Some(read_write)
    );
    assert_eq!(
        program_rights(RuleSource::Entry, Rights::WRITE, named(1, true, false)),
        None
    );
}

/// The rules of a run's box, opened in `scratch`: the project read, an
/// entry and a tool directory written, a write flag's directory, a
/// directory `--allow-run-write=` names for each of two programs, one of
/// them the entry's too, the private directory, and `/dev/null`, opened
/// beneath `/` as the plan opens it.
fn run_rules(scratch: &Scratch) -> BoxRules {
    for dir in [
        "project/src",
        "project/.svelte-kit",
        "flagged",
        "first",
        "private",
    ] {
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
            Rights::WRITE_PLAIN,
            RuleSource::ToolDir(ToolDir::SvelteKit),
        ),
        (
            "flagged",
            Rights::READ | Rights::WRITE,
            RuleSource::Flag(DirFlag::Write),
        ),
        (
            "first",
            Rights::READ | Rights::WRITE,
            RuleSource::RunWrite(0),
        ),
        (
            "project/src",
            Rights::READ | Rights::WRITE,
            RuleSource::RunWrite(1),
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

/// What a box for a program writing `writes` grants beneath the run's
/// rules, by path.
fn granted(programs: &ProgramRules, writes: ProgramWrites) -> Vec<(PathBuf, Rights)> {
    programs
        .granted(writes)
        .map(|(rule, rights)| (rule.path().to_owned(), rights))
        .collect()
}

/// A run that may start a program keeps every rule as data, and holds the
/// handles some program of its box takes, the private directory's among
/// them: a program of the table reads the project, writes its tool
/// directory and `/dev/null`, and nothing of what a write flag names; one
/// with no write set writes only `/dev/null`; each program `--allow-run-write=`
/// names writes its own directories and no other's, and the entries only
/// where its set names the root. A rule no program of the run takes, an
/// entry no program writes and the write flag's, is held for none.
#[test]
fn a_spawning_run_holds_what_its_programs_boxes_take() {
    let scratch = Scratch::new("program_hold");
    let at = |path: &str| scratch.path().join(path);
    let first = named(0, true, false);
    let second = named(1, false, true);
    let (rules, absent) = run_rules(&scratch).into_installed();
    assert!(absent.is_empty());
    let (installed, programs) = ProgramRules::hold(
        rules,
        &[ProgramWrites::NONE, kit(), first, second],
        BTreeMap::new(),
    );
    let sources: Vec<_> = installed.iter().map(InstalledRule::source).collect();
    assert_eq!(
        sources,
        [
            RuleSource::Project,
            RuleSource::Entry,
            RuleSource::ToolDir(ToolDir::SvelteKit),
            RuleSource::Flag(DirFlag::Write),
            RuleSource::RunWrite(0),
            RuleSource::RunWrite(1),
            RuleSource::PrivateTemp,
            RuleSource::Device,
        ]
    );
    let programs = programs.unwrap();
    let null = Path::new("/").join("dev/null");
    let read_write = Rights::READ | Rights::WRITE;
    assert_eq!(
        granted(&programs, kit()),
        [
            (at("project"), Rights::READ),
            (at("project/.svelte-kit"), Rights::WRITE_PLAIN),
            (null.clone(), read_write),
        ]
    );
    assert_eq!(
        granted(&programs, ProgramWrites::NONE),
        [(at("project"), Rights::READ), (null.clone(), read_write)]
    );
    assert_eq!(
        granted(&programs, first),
        [
            (at("project"), Rights::READ),
            (at("project/.svelte-kit"), Rights::WRITE_PLAIN),
            (at("first"), read_write),
            (null.clone(), read_write),
        ]
    );
    assert_eq!(
        granted(&programs, second),
        [
            (at("project"), Rights::READ),
            (at("project/src"), Rights::WRITE),
            (at("project/src"), read_write),
            (null.clone(), read_write),
        ]
    );
    // with no program naming the root the entry's handle isn't held, nor
    // the write flag's, nor a program's no program of the run is, whatever
    // box asks for them
    let (rules, _) = run_rules(&scratch).into_installed();
    let (_, programs) = ProgramRules::hold(
        rules,
        &[ProgramWrites::NONE, kit(), named(1, false, false)],
        BTreeMap::new(),
    );
    let programs = programs.unwrap();
    assert_eq!(
        granted(&programs, second),
        [
            (at("project"), Rights::READ),
            (at("project/src"), read_write),
            (null, read_write),
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
    let (installed, programs) =
        ProgramRules::hold(rules, &[ProgramWrites::NONE, kit()], BTreeMap::new());
    assert_eq!(installed.len(), 10);
    let programs = programs.unwrap();
    for program in [kit(), ProgramWrites::NONE] {
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
    assert_eq!(granted(&programs, kit()).len(), 3);
}

/// A run that starts nothing holds no handle past its box, nor does one
/// with no private directory, which no program's box can be made in; both
/// keep every rule as data.
#[test]
fn a_run_that_starts_nothing_holds_no_handle() {
    let scratch = Scratch::new("program_none");
    let (rules, _) = run_rules(&scratch).into_installed();
    let (installed, programs) = ProgramRules::hold(rules, &[], BTreeMap::new());
    assert_eq!(installed.len(), 8);
    assert!(programs.is_none());
    let (rules, _) = run_rules(&scratch).into_installed();
    let without_private: Vec<_> = rules
        .into_iter()
        .filter(|(rule, _)| rule.source() != RuleSource::PrivateTemp)
        .collect();
    let (installed, programs) = ProgramRules::hold(without_private, &[kit()], BTreeMap::new());
    assert_eq!(installed.len(), 7);
    assert!(programs.is_none());
}

/// A program's box makes its temporary directory in the private directory,
/// named by its key's place, its user's alone; a second box for the same
/// place, and an entry planted at a place's name, a symlink out of the
/// private directory among them, are refused, nothing made where the link
/// leads.
#[test]
fn a_program_s_box_makes_its_temp_dir_by_its_place() {
    let scratch = Scratch::new("program_temp");
    let outside = scratch.dir("outside");
    let (rules, _) = run_rules(&scratch).into_installed();
    let (_, programs) = ProgramRules::hold(rules, &[kit()], BTreeMap::new());
    let programs = programs.unwrap();
    let made = programs.program_box(kit(), 3).unwrap();
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
        refused(programs.program_box(ProgramWrites::NONE, 3)),
        (temp, Some(libc::EEXIST))
    );
    scratch.link(&outside.join("planted"), "private/program_4");
    assert_eq!(
        refused(programs.program_box(ProgramWrites::NONE, 4)),
        (scratch.path().join("private/program_4"), Some(libc::EEXIST))
    );
    assert!(!outside.join("planted").exists());
    assert!(programs.program_box(ProgramWrites::NONE, 5).is_ok());
}

/// The run's boxes build each place's box once, at its first ask, and keep
/// it, or the text of its first error, with its causes, for every ask
/// after, never building again: a place whose entry was planted fails as
/// it first did once the entry is gone, and another place builds.
#[test]
fn a_program_s_box_is_built_once_and_kept() {
    let scratch = Scratch::new("program_boxes");
    let (rules, _) = run_rules(&scratch).into_installed();
    let (_, programs) = ProgramRules::hold(rules, &[kit()], BTreeMap::new());
    let boxes = ProgramBoxes::new(programs.unwrap());
    let built = boxes.box_for(0, kit()).unwrap();
    assert!(Rc::ptr_eq(&built, &boxes.box_for(0, kit()).unwrap()));
    std::fs::create_dir(scratch.path().join("private/program_1")).unwrap();
    let failed = boxes.box_for(1, ProgramWrites::NONE).unwrap_err();
    assert_eq!(
        &*failed,
        format!(
            "failed to make the program's temporary directory {}: File exists (os error 17)",
            scratch.path().join("private/program_1").display()
        )
    );
    std::fs::remove_dir(scratch.path().join("private/program_1")).unwrap();
    assert_eq!(boxes.box_for(1, ProgramWrites::NONE).unwrap_err(), failed);
    assert!(boxes.box_for(2, ProgramWrites::NONE).is_ok());
}

/// A program's write set is the one the run holds for its name, the
/// table's and `--allow-run-write=`'s joined, and a program with none
/// writes nothing.
#[test]
fn a_program_s_writes_follow_its_name() {
    let scratch = Scratch::new("program_writes");
    let (rules, _) = run_rules(&scratch).into_installed();
    let named_table = named(0, true, false);
    let named_flag = named(1, false, true);
    let (_, programs) = ProgramRules::hold(
        rules,
        &[ProgramWrites::NONE, kit(), named_table, named_flag],
        BTreeMap::from([
            ("node_modules/.bin/svelte-kit".to_owned(), kit()),
            ("node_modules/.bin/vitest".to_owned(), named_table),
            ("python3".to_owned(), named_flag),
        ]),
    );
    let programs = programs.unwrap();
    for (name, writes) in [
        ("node_modules/.bin/svelte-kit", kit()),
        ("node_modules/.bin/vitest", named_table),
        ("python3", named_flag),
        ("svelte-kit", ProgramWrites::NONE),
        ("node_modules/.bin/tsc", ProgramWrites::NONE),
    ] {
        assert_eq!(programs.writes(name), writes, "{name}");
    }
}

/// A program's box is built on the isolate's thread and installed on the
/// thread that launches it.
#[test]
fn a_program_s_box_crosses_threads() {
    const fn shared<T: Send + Sync>() {}
    shared::<ProgramBox>();
    shared::<ProgramRules>();
}
