//! Tests for the box's rules: what each right grants, eval's rules, and
//! how a rule's handle opens.

use std::os::fd::{AsFd as _, AsRawFd as _};
use std::path::{Path, PathBuf};

use landlock::{AccessFs, BitFlags, make_bitflags};

use super::Scratch;
use crate::confine::EvalPrepared;
use crate::confine::plan::RuleSource;
use crate::confine::rules::{BoxRules, Kind, Need, RealRights, Rights, RuleError};

/// The path the kernel names `rule`'s handle by.
fn named(handle: std::os::fd::BorrowedFd<'_>) -> PathBuf {
    std::fs::read_link(format!("/proc/self/fd/{}", handle.as_raw_fd())).unwrap()
}

/// Each right is the Landlock rights it lists, on a directory, and on
/// anything else only those Landlock applies to a file.
#[test]
fn rights_expand_to_landlock_s() {
    let write = make_bitflags!(AccessFs::{
        WriteFile | Truncate | MakeReg | MakeDir | MakeFifo | MakeSym | RemoveFile | RemoveDir
            | Refer
    });
    let read = AccessFs::ReadFile | AccessFs::ReadDir;
    for (rights, directory, file) in [
        (Rights::READ, read, BitFlags::from(AccessFs::ReadFile)),
        (
            Rights::WRITE,
            write,
            AccessFs::WriteFile | AccessFs::Truncate,
        ),
        (
            Rights::WRITE_PLAIN,
            write & !make_bitflags!(AccessFs::{MakeSym | MakeFifo | Refer}),
            AccessFs::WriteFile | AccessFs::Truncate,
        ),
        (
            Rights::READ | Rights::WRITE_PLAIN | Rights::WRITE,
            read | write,
            AccessFs::ReadFile | AccessFs::WriteFile | AccessFs::Truncate,
        ),
        (
            Rights::EXECUTE,
            BitFlags::from(AccessFs::Execute),
            BitFlags::from(AccessFs::Execute),
        ),
        (
            Rights::READ | Rights::WRITE | Rights::EXECUTE,
            read | write | AccessFs::Execute,
            AccessFs::ReadFile | AccessFs::WriteFile | AccessFs::Truncate | AccessFs::Execute,
        ),
    ] {
        assert_eq!(rights.access(Kind::Directory), directory, "{rights:?}");
        assert_eq!(rights.access(Kind::File), file, "{rights:?}");
    }
    assert_eq!(Rights::from(RealRights::Read), Rights::READ);
    assert_eq!(
        Rights::from(RealRights::ReadExecute),
        Rights::READ | Rights::EXECUTE
    );
}

/// Eval's rules are what its box has always held: reading files and
/// listing directories beneath each root, the project's first, each on the
/// handle opened as the root's directory, in the roots' order, and no
/// other right anywhere.
#[test]
fn eval_s_rules_read_each_root() {
    let scratch = Scratch::new("eval_rules");
    let project = scratch.dir("project");
    let lib = scratch.dir("lib");
    scratch.file("lib/package.json");
    scratch.dir("project/node_modules");
    scratch.link(&lib, "project/node_modules/lib");
    let prepared = EvalPrepared::for_project(project.clone()).unwrap();
    assert_eq!(prepared.roots().as_slice(), [project.clone(), lib.clone()]);
    let rules = prepared.rules.rules();
    assert_eq!(rules.len(), 2);
    for (rule, root) in rules.iter().zip([&project, &lib]) {
        assert_eq!(rule.path(), root);
        assert_eq!(rule.kind(), Kind::Directory);
        assert_eq!(rule.rights(), Rights::READ);
        assert_eq!(rule.access(), AccessFs::ReadFile | AccessFs::ReadDir);
        assert_eq!(&named(rule.handle()), root);
    }
    assert!(prepared.rules.absent().is_empty());
}

/// The project's directory, opened to hold rules beneath it.
fn open_project(project: &Path) -> std::os::fd::OwnedFd {
    gro_sys::beneath::open_root(project).unwrap()
}

/// A rule opened beneath a directory gets no handle through a symlink:
/// `node_modules/.vite` linked to a directory outside the project, or
/// reached through a `node_modules` linked outside, is refused, required or
/// optional, naming the symlink as the reason, and adds no rule; the real
/// directory and file beside it are opened as found, a file's rights cut to
/// a file's.
#[test]
fn a_rule_beneath_never_passes_through_a_symlink() {
    let scratch = Scratch::new("rules_links");
    let project = scratch.dir("project");
    let outside = scratch.dir("home/.ssh");
    scratch.dir("project/node_modules");
    scratch.link(&outside, "project/node_modules/.vite");
    let dir = open_project(&project);
    let mut rules = BoxRules::new();
    for need in [Need::Optional, Need::Required] {
        let error = rules
            .beneath(
                dir.as_fd(),
                &project,
                Path::new("node_modules/.vite"),
                Rights::READ | Rights::WRITE,
                need,
                RuleSource::Project,
            )
            .unwrap_err();
        let RuleError::Symlink { path } = &error else {
            panic!("{error:?}");
        };
        assert_eq!(path, &project.join("node_modules/.vite"));
        assert!(
            error
                .to_string()
                .contains("refused: a symlink is on its path"),
            "{error}"
        );
    }
    // a symlink earlier on the path, to a real directory outside
    let linked = scratch.dir("linked");
    scratch.dir("elsewhere/node_modules/.vite");
    scratch.link(
        &scratch.path().join("elsewhere/node_modules"),
        "linked/node_modules",
    );
    let error = rules
        .beneath(
            open_project(&linked).as_fd(),
            &linked,
            Path::new("node_modules/.vite"),
            Rights::WRITE,
            Need::Optional,
            RuleSource::Project,
        )
        .unwrap_err();
    assert!(matches!(error, RuleError::Symlink { .. }), "{error:?}");
    assert!(rules.rules().is_empty());
    assert!(rules.absent().is_empty());
    // what is real beside them opens
    scratch.dir("project/node_modules/.vite-temp");
    scratch.file("project/package.json");
    rules
        .beneath(
            dir.as_fd(),
            &project,
            Path::new("node_modules/.vite-temp"),
            Rights::READ | Rights::WRITE,
            Need::Required,
            RuleSource::Project,
        )
        .unwrap();
    rules
        .beneath(
            dir.as_fd(),
            &project,
            Path::new("package.json"),
            Rights::READ | Rights::WRITE,
            Need::Required,
            RuleSource::Project,
        )
        .unwrap();
    let [temp, json] = rules.rules() else {
        panic!("{:?}", rules.rules());
    };
    assert_eq!(temp.path(), project.join("node_modules/.vite-temp"));
    assert_eq!(temp.kind(), Kind::Directory);
    assert_eq!(
        named(temp.handle()),
        project.join("node_modules/.vite-temp")
    );
    assert_eq!(json.kind(), Kind::File);
    assert_eq!(
        json.access(),
        AccessFs::ReadFile | AccessFs::WriteFile | AccessFs::Truncate
    );
}

/// A symlink that stays inside the directory is refused as one leading out
/// of it is: `node_modules/.vite` linked to the project's own `.git`, and a
/// path through a directory linked to `node_modules`, add no rule, so a
/// rule never reaches what its path doesn't name.
#[test]
fn a_rule_beneath_refuses_a_symlink_that_stays_inside() {
    let scratch = Scratch::new("rules_inner_links");
    let project = scratch.dir("project");
    scratch.dir("project/.git");
    scratch.dir("project/node_modules/.vite-temp");
    scratch.link(Path::new("../.git"), "project/node_modules/.vite");
    scratch.link(Path::new("node_modules"), "project/cache");
    let dir = open_project(&project);
    let mut rules = BoxRules::new();
    for path in ["node_modules/.vite", "cache/.vite-temp"] {
        let error = rules
            .beneath(
                dir.as_fd(),
                &project,
                Path::new(path),
                Rights::WRITE,
                Need::Optional,
                RuleSource::Project,
            )
            .unwrap_err();
        assert!(
            matches!(&error, RuleError::Symlink { path: refused } if refused == &project.join(path)),
            "{path}: {error:?}"
        );
    }
    assert!(rules.rules().is_empty());
    assert!(rules.absent().is_empty());
}

/// A rule opened beneath a directory never leaves it: an absolute path or
/// a `..` above it is refused.
#[test]
fn a_rule_beneath_never_leaves_its_directory() {
    let scratch = Scratch::new("rules_escape");
    let project = scratch.dir("project");
    scratch.dir("sibling");
    let dir = open_project(&project);
    let mut rules = BoxRules::new();
    for path in ["../sibling", "/tmp"] {
        let error = rules
            .beneath(
                dir.as_fd(),
                &project,
                Path::new(path),
                Rights::READ,
                Need::Optional,
                RuleSource::Project,
            )
            .unwrap_err();
        assert!(
            matches!(error, RuleError::Escapes { .. }),
            "{path}: {error:?}"
        );
    }
    assert!(rules.rules().is_empty());
}

/// An optional rule with nothing to name adds none and is recorded; a
/// required one is refused, by either opening.
#[test]
fn only_an_optional_rule_may_have_nothing_to_name() {
    let scratch = Scratch::new("rules_absent");
    let project = scratch.dir("project");
    let dir = open_project(&project);
    let mut rules = BoxRules::new();
    let missing = Path::new("node_modules/.vite");
    rules
        .beneath(
            dir.as_fd(),
            &project,
            missing,
            Rights::WRITE,
            Need::Optional,
            RuleSource::Project,
        )
        .unwrap();
    rules
        .real(
            &scratch.path().join("absent"),
            RealRights::Read,
            Need::Optional,
            RuleSource::Project,
        )
        .unwrap();
    assert!(rules.rules().is_empty());
    assert_eq!(
        rules.absent(),
        [project.join(missing), scratch.path().join("absent")]
    );
    let error = rules
        .beneath(
            dir.as_fd(),
            &project,
            missing,
            Rights::WRITE,
            Need::Required,
            RuleSource::Project,
        )
        .unwrap_err();
    assert!(matches!(error, RuleError::Missing { .. }), "{error:?}");
    let error = rules
        .real(
            &scratch.path().join("absent"),
            RealRights::Read,
            Need::Required,
            RuleSource::Project,
        )
        .unwrap_err();
    assert!(matches!(error, RuleError::Missing { .. }), "{error:?}");
    assert_eq!(rules.absent().len(), 2);
}

/// A rule opened by its real path names what the path names, and only
/// when the path is real: a symlink there, or a path through a symlinked
/// directory, which the kernel names by another path once opened, is
/// refused.
#[test]
fn a_rule_by_real_path_names_only_a_real_path() {
    let scratch = Scratch::new("rules_real");
    let bin = scratch.dir("usr/bin");
    scratch.file("usr/bin/tool");
    scratch.link(&bin.join("tool"), "linked_tool");
    scratch.link(&scratch.path().join("usr"), "alias");
    let mut rules = BoxRules::new();
    rules
        .real(
            &bin,
            RealRights::ReadExecute,
            Need::Required,
            RuleSource::Project,
        )
        .unwrap();
    rules
        .real(
            &bin.join("tool"),
            RealRights::Read,
            Need::Required,
            RuleSource::Project,
        )
        .unwrap();
    let [dir, file] = rules.rules() else {
        panic!("{:?}", rules.rules());
    };
    assert_eq!(dir.kind(), Kind::Directory);
    assert_eq!(dir.rights(), Rights::READ | Rights::EXECUTE);
    assert_eq!(named(dir.handle()), bin);
    assert_eq!(file.kind(), Kind::File);
    assert_eq!(file.access(), BitFlags::from(AccessFs::ReadFile));
    let link = scratch.path().join("linked_tool");
    let error = rules
        .real(&link, RealRights::Read, Need::Optional, RuleSource::Project)
        .unwrap_err();
    assert!(
        matches!(&error, RuleError::Symlink { path } if path == &link),
        "{error:?}"
    );
    let through = scratch.path().join("alias/bin");
    let error = rules
        .real(
            &through,
            RealRights::Read,
            Need::Optional,
            RuleSource::Project,
        )
        .unwrap_err();
    let RuleError::NotReal { path, opened } = &error else {
        panic!("{error:?}");
    };
    assert_eq!((path, opened), (&through, &bin));
    assert_eq!(rules.rules().len(), 2);
}

/// A path that names nothing by its form is refused, never recorded as
/// absent: an empty path beneath a directory, and a relative or empty one
/// by real path, whether or not something is there.
#[test]
fn a_rule_s_path_of_no_form_is_refused_not_absent() {
    let scratch = Scratch::new("rules_form");
    let project = scratch.dir("project");
    let dir = open_project(&project);
    let mut rules = BoxRules::new();
    let error = rules
        .beneath(
            dir.as_fd(),
            &project,
            Path::new(""),
            Rights::READ,
            Need::Optional,
            RuleSource::Project,
        )
        .unwrap_err();
    assert!(matches!(error, RuleError::Open { .. }), "{error:?}");
    for path in ["", "relative/missing", "."] {
        let error = rules
            .real(
                Path::new(path),
                RealRights::Read,
                Need::Optional,
                RuleSource::Project,
            )
            .unwrap_err();
        assert!(
            matches!(error, RuleError::Open { .. }),
            "{path:?}: {error:?}"
        );
    }
    assert!(rules.rules().is_empty());
    assert!(rules.absent().is_empty());
}
