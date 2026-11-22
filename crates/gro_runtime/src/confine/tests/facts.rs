//! Tests for the box's facts, read from scratch projects, and the plans
//! they make.

use std::path::{Path, PathBuf};

use super::Scratch;
use crate::confine::facts::MERGED_DIRS;
use crate::confine::plan::{
    BoxFacts, BoxInputs, BoxPlan, EntryKind, NodeModules, PlanRefusal, PlannedRule, RuleSource,
    TopEntry,
};

/// A run of the project at `root` with `HOME` at `home`, writing the root
/// when `writes`.
fn inputs(root: &Path, home: &Path, writes: bool) -> BoxInputs {
    BoxInputs {
        root: root.to_owned(),
        writes_root: writes,
        home: Some(home.to_owned()),
        ..BoxInputs::default()
    }
}

/// A run of the project at `root` with `HOME` at `home` that may start a
/// program, for which the facts read git's files.
fn git_inputs(root: &Path, home: &Path) -> BoxInputs {
    BoxInputs {
        spawns: true,
        ..inputs(root, home, false)
    }
}

/// The path a rule opens on, as it opens.
fn opened(rule: &PlannedRule) -> PathBuf {
    rule.opening.path().to_owned()
}

/// A project's top-level entries are read without following a symlink, a
/// hard-linked file with its count; a package linked from outside is in
/// the link closure, a planted symlink to a directory that isn't a package
/// and `node_modules/node_modules` back to its own directory aren't; and
/// the merged system directories are read as `lstat` reads them.
#[test]
fn the_facts_are_what_the_disk_holds() {
    let scratch = Scratch::new("facts");
    let project = scratch.dir("w/project");
    let home = scratch.dir("w/home");
    let outside = scratch.dir("w/outside/pkg");
    scratch.file("w/outside/pkg/package.json");
    scratch.dir("w/project/src");
    scratch.dir("w/project/.git/hooks");
    scratch.dir("w/project/node_modules/installed");
    scratch.file("w/project/node_modules/installed/package.json");
    scratch.file("w/project/package.json");
    scratch.file("w/project/CLAUDE.md");
    scratch.link(Path::new("CLAUDE.md"), "w/project/AGENTS.md");
    scratch.link(&home, "w/project/planted");
    scratch.link(&outside, "w/project/node_modules/linked");
    scratch.link(&home, "w/project/node_modules/home");
    scratch.link(
        &project.join("node_modules"),
        "w/project/node_modules/node_modules",
    );
    scratch.file("w/outside/hard.txt");
    std::fs::hard_link(
        scratch.path().join("w/outside/hard.txt"),
        project.join("hard.txt"),
    )
    .unwrap();
    let facts = BoxFacts::read(&inputs(&project, &home, false)).unwrap();
    let entry = |name: &str, kind| TopEntry {
        name: name.into(),
        kind,
    };
    let file = EntryKind::File { links: 1 };
    assert_eq!(
        facts.entries,
        [
            entry(".git", EntryKind::Directory),
            entry("AGENTS.md", EntryKind::Symlink),
            entry("CLAUDE.md", file),
            entry("hard.txt", EntryKind::File { links: 2 }),
            entry("node_modules", EntryKind::Directory),
            entry("package.json", file),
            entry("planted", EntryKind::Symlink),
            entry("src", EntryKind::Directory),
        ]
    );
    assert_eq!(facts.node_modules, NodeModules::Directory);
    assert_eq!(facts.linked, [outside]);
    let system: Vec<(PathBuf, EntryKind)> = MERGED_DIRS
        .iter()
        .filter_map(|dir| {
            let metadata = std::fs::symlink_metadata(dir).ok()?;
            let kind = if metadata.file_type().is_symlink() {
                EntryKind::Symlink
            } else if metadata.is_dir() {
                EntryKind::Directory
            } else {
                return None;
            };
            Some((PathBuf::from(dir), kind))
        })
        .collect();
    assert_eq!(facts.system, system);
    // writing, the symlinks and the hard-linked file get no rule
    let plan = BoxPlan::for_run(&inputs(&project, &home, true), &facts).unwrap();
    let written: Vec<PathBuf> = plan
        .rules()
        .iter()
        .filter(|rule| rule.source == RuleSource::Entry)
        .map(opened)
        .collect();
    assert_eq!(
        written,
        [
            PathBuf::from("CLAUDE.md"),
            "package.json".into(),
            "src".into()
        ]
    );
    let skipped: Vec<&Path> = plan
        .skipped()
        .iter()
        .map(|skipped| skipped.path.as_path())
        .filter(|path| path.starts_with(&project))
        .collect();
    assert_eq!(
        skipped,
        [
            project.join("AGENTS.md"),
            project.join("hard.txt"),
            project.join("planted")
        ]
    );
}

/// A nested `vite.config.ts` with a `node_modules` of its own gets no rule
/// in check mode, where Vite's write there fails with `EACCES`, and is
/// written beneath its top-level entry's rule by a writing command, which
/// can't leave it out.
#[test]
fn a_nested_node_modules_gets_no_rule_of_its_own() {
    let scratch = Scratch::new("facts_nested");
    let project = scratch.dir("w/project");
    let home = scratch.dir("w/home");
    scratch.dir("w/project/examples/app/node_modules");
    scratch.file("w/project/examples/app/vite.config.ts");
    scratch.file("w/project/package.json");
    let mut checking = inputs(&project, &home, false);
    checking.tools = ["vitest".to_owned()].into();
    checking.declared = ["vitest".to_owned()].into();
    let facts = BoxFacts::read(&checking).unwrap();
    let plan = BoxPlan::for_run(&checking, &facts).unwrap();
    let paths: Vec<PathBuf> = plan.rules().iter().map(opened).collect();
    assert!(
        !paths.iter().any(|path| path.starts_with("examples")),
        "{paths:?}"
    );
    let writing = BoxPlan::for_run(&inputs(&project, &home, true), &facts).unwrap();
    assert!(
        writing
            .rules()
            .iter()
            .any(|rule| rule.source == RuleSource::Entry && opened(rule) == Path::new("examples"))
    );
}

/// A `node_modules` that is a symlink is followed to its real path, whose
/// packages are inside it, not each a root; its own `node_modules` back to
/// itself adds nothing, and a package it links from outside is a root.
#[test]
fn a_symlinked_node_modules_is_read_where_it_leads() {
    let scratch = Scratch::new("facts_linked_node_modules");
    let project = scratch.dir("w/project");
    let home = scratch.dir("w/home");
    let shared = scratch.dir("w/shared/node_modules");
    scratch.dir("w/shared/node_modules/installed");
    scratch.file("w/shared/node_modules/installed/package.json");
    let outside = scratch.dir("w/outside/pkg");
    scratch.file("w/outside/pkg/package.json");
    scratch.link(&outside, "w/shared/node_modules/linked");
    scratch.link(&shared, "w/shared/node_modules/node_modules");
    scratch.link(&shared, "w/project/node_modules");
    let facts = BoxFacts::read(&inputs(&project, &home, false)).unwrap();
    assert_eq!(facts.node_modules, NodeModules::Link { real: shared });
    assert_eq!(facts.linked, [outside]);
    // one leading nowhere is none
    std::fs::remove_file(project.join("node_modules")).unwrap();
    scratch.link(&scratch.path().join("w/gone"), "w/project/node_modules");
    let facts = BoxFacts::read(&inputs(&project, &home, false)).unwrap();
    assert_eq!(facts.node_modules, NodeModules::Absent);
}

/// Git's home config is read by real path: `~/.gitconfig` through a
/// dotfile manager's symlink, `config` and the defaults `ignore` and
/// `attributes` in its directory under `~/.config` but never
/// `credentials`, `XDG_CONFIG_HOME`'s in its place when it's set; with
/// `GIT_CONFIG_GLOBAL` set, its target, relative to the repository's top,
/// and the defaults alone, one in the project needing no rule of its own;
/// one missing, or a symlink leading nowhere, is none.
#[test]
fn git_s_home_config_is_read_by_real_path() {
    let scratch = Scratch::new("facts_git");
    // a config the reader reads, empty, where `Scratch::file` writes `{}`
    let empty = |path: &str| std::fs::write(scratch.path().join(path), "").unwrap();
    let project = scratch.dir("w/project");
    let home = scratch.dir("w/home");
    scratch.dir("w/dotfiles");
    empty("w/dotfiles/gitconfig");
    scratch.link(
        &scratch.path().join("w/dotfiles/gitconfig"),
        "w/home/.gitconfig",
    );
    scratch.dir("w/home/.config/git");
    for file in ["config", "ignore", "credentials"] {
        empty(&format!("w/home/.config/git/{file}"));
    }
    scratch.dir("w/xdg/git");
    empty("w/xdg/git/attributes");
    empty("w/project/global");
    empty("w/global");
    let base = git_inputs(&project, &home);
    let real = |path: &str| scratch.path().join(path);
    assert_eq!(
        BoxFacts::read(&base).unwrap().git_home,
        [
            real("w/dotfiles/gitconfig"),
            real("w/home/.config/git/config"),
            real("w/home/.config/git/ignore"),
        ]
    );
    let xdg = BoxInputs {
        xdg_config_home: Some(real("w/xdg")),
        ..base.clone()
    };
    assert_eq!(
        BoxFacts::read(&xdg).unwrap().git_home,
        [real("w/dotfiles/gitconfig"), real("w/xdg/git/attributes")]
    );
    let global = BoxInputs {
        git_config_global: Some(real("w/global")),
        ..base.clone()
    };
    assert_eq!(
        BoxFacts::read(&global).unwrap().git_home,
        [real("w/global"), real("w/home/.config/git/ignore")]
    );
    let in_project = BoxInputs {
        git_config_global: Some("global".into()),
        ..base.clone()
    };
    assert_eq!(
        BoxFacts::read(&in_project).unwrap().git_home,
        [real("w/home/.config/git/ignore")]
    );
    std::fs::remove_file(real("w/dotfiles/gitconfig")).unwrap();
    let dangling = BoxInputs {
        git_config_global: Some(real("w/missing")),
        ..base.clone()
    };
    assert_eq!(
        BoxFacts::read(&dangling).unwrap().git_home,
        [real("w/home/.config/git/ignore")]
    );
    assert_eq!(
        BoxFacts::read(&base).unwrap().git_home,
        [
            real("w/home/.config/git/config"),
            real("w/home/.config/git/ignore"),
        ]
    );
}

/// The files git's home config names are read by real path: an include
/// relative to its file and one from `~/`, an `includeIf` whatever its
/// condition, includes of includes, `core.excludesFile` and
/// `core.attributesFile`, each that exists; a missing one, and one in the
/// project, which the project's rule covers, are none.
#[test]
fn git_s_includes_are_read() {
    let scratch = Scratch::new("facts_git_includes");
    let project = scratch.dir("w/project");
    let home = scratch.dir("w/home");
    scratch.dir("w/home/conf");
    let write = |path: &str, text: &str| std::fs::write(scratch.path().join(path), text).unwrap();
    write(
        "w/home/.gitconfig",
        "# a comment\n[user]\n\tname = \"A ; B\" ; trailing\n[include]\n\tpath = conf/local\n\
         [includeIf \"gitdir:~/work/\"]\n\tpath = ~/conf/work\n[core]\n\texcludesFile = \
         ~/conf/ignore\n\tattributesfile = \"conf/attrs\"\n\tbare\n[include]\n\tpath = \
         ~/missing\n",
    );
    write("w/home/conf/local", "[include]\npath = deeper\n");
    write("w/home/conf/deeper", "[core]\n");
    write("w/home/conf/work", "");
    write("w/home/conf/ignore", "");
    scratch.dir("w/project/conf");
    write("w/project/conf/attrs", "");
    let facts = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
    let real = |path: &str| scratch.path().join(path);
    assert_eq!(facts.git_fault, None);
    assert_eq!(
        facts.git_home,
        [
            real("w/home/.gitconfig"),
            real("w/home/conf/local"),
            real("w/home/conf/deeper"),
            real("w/home/conf/work"),
            real("w/home/conf/ignore"),
        ]
    );
}

/// A relative include is resolved as git resolves it, from the including
/// file as named: beside a dotfile manager's symlinked `~/.gitconfig`, not
/// beside its target.
#[test]
fn git_s_relative_include_is_beside_the_config_as_named() {
    let scratch = Scratch::new("facts_git_include_link");
    let project = scratch.dir("w/project");
    let home = scratch.dir("w/home");
    let dots = scratch.dir("w/dots");
    std::fs::write(
        dots.join("gitconfig"),
        "[include]\n\tpath = .gitconfig.local\n",
    )
    .unwrap();
    std::fs::write(dots.join(".gitconfig.local"), "").unwrap();
    scratch.link(&dots.join("gitconfig"), "w/home/.gitconfig");
    std::fs::write(home.join(".gitconfig.local"), "").unwrap();
    let facts = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
    assert_eq!(
        facts.git_home,
        [
            scratch.path().join("w/dots/gitconfig"),
            home.join(".gitconfig.local")
        ]
    );
}

/// What git's config names through the project gets no rule: a relative
/// `core.excludesFile` the project links to a key, and an include into the
/// project whose own keys name one, so the project's links and files
/// widen nothing.
#[test]
fn git_s_config_naming_the_project_widens_nothing() {
    let scratch = Scratch::new("facts_git_through_project");
    let project = scratch.dir("w/home/dev/project");
    let home = scratch.path().join("w/home");
    scratch.dir("w/home/.ssh");
    std::fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
    std::fs::write(
        home.join(".gitconfig"),
        format!(
            "[core]\n\texcludesFile = .ignore\n[include]\n\tpath = {}\n",
            project.join("shared").display()
        ),
    )
    .unwrap();
    scratch.link(&home.join(".ssh/id_ed25519"), "w/home/dev/project/.ignore");
    std::fs::write(
        project.join("shared"),
        format!(
            "[core]\n\tattributesFile = {}\n",
            home.join(".ssh/id_ed25519").display()
        ),
    )
    .unwrap();
    let facts = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
    assert_eq!(facts.git_fault, None);
    assert_eq!(facts.git_home, [home.join(".gitconfig")]);
}

/// A link of the user's into the project, a file or a directory on the
/// way, whose target the project replaces with a link out, gets no rule:
/// the walk meets the tree between the two links, and so it does where a
/// `..` after a link of the user's leads into it.
#[test]
fn git_s_link_through_the_project_widens_nothing() {
    let scratch = Scratch::new("facts_git_link_through_project");
    let project = scratch.dir("w/home/dev/project");
    let home = scratch.path().join("w/home");
    scratch.dir("w/home/.ssh");
    std::fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
    // a stow-style link to the project's file, which the project made a
    // link to the key
    scratch.dir("w/home/.config/git");
    scratch.dir("w/home/dev/project/git");
    scratch.link(&project.join("git/ignore"), "w/home/.config/git/ignore");
    scratch.link(
        &home.join(".ssh/id_ed25519"),
        "w/home/dev/project/git/ignore",
    );
    // a directory linked into the project, which the project made a link
    // to `~/.ssh`
    scratch.link(&project.join("keys"), "w/home/keys");
    scratch.link(&home.join(".ssh"), "w/home/dev/project/keys");
    // `..` from where a link of the user's leads, never from its name
    scratch.dir("w/home/a/b");
    scratch.link(&home.join("a/b"), "w/home/up");
    std::fs::write(
        home.join(".gitconfig"),
        "[core]\n\tattributesFile = ~/keys/id_ed25519\n\texcludesFile = \
         ~/up/../../dev/project/keys/id_ed25519\n",
    )
    .unwrap();
    let facts = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
    assert_eq!(facts.git_fault, None);
    assert_eq!(facts.git_home, [home.join(".gitconfig")]);
}

/// What git itself rejects is a fault, which refuses a run that may start
/// a program and no other, worded as git words it: an unterminated quote,
/// an unknown escape, an unclosed header, a variable not starting with a
/// letter, one after a form feed, one followed by a lone `\r`; so are
/// another user's home, a named directory, and includes nested past git's
/// limit. A run that starts no program reads none of it.
#[test]
fn git_s_config_faults_refuse_a_spawning_run() {
    for (case, text, why) in [
        ("quote", "[core]\nexcludesFile = \"a\n", "bad config line 2"),
        (
            "escape",
            "[core]\nexcludesFile = a\\qb\n",
            "bad config line 2",
        ),
        ("header", "[core\nexcludesFile = a\n", "bad config line 1"),
        ("variable", "[core]\n1bad = a\n", "bad config line 2"),
        (
            "form_feed",
            "[core]\n\x0cexcludesFile = a\n",
            "bad config line 2",
        ),
        ("lone_cr", "[core]\nbare\r", "bad config line 2"),
        (
            "user",
            "[core]\nexcludesFile = ~root/x\n",
            "another user's home",
        ),
        (
            "directory",
            "[include]\npath = ~/dir\n",
            "names a directory",
        ),
        ("home", "[include]\npath = ~\n", "names a directory"),
        ("deep", "[include]\npath = ~/.gitconfig\n", "nest deeper"),
    ] {
        let scratch = Scratch::new(&format!("facts_git_fault_{case}"));
        let project = scratch.dir("w/project");
        let home = scratch.dir("w/home");
        std::fs::write(home.join(".gitconfig"), text).unwrap();
        if case == "directory" {
            std::fs::write(home.join(".gitconfig"), "[include]\npath = ~/dir\n").unwrap();
            scratch.dir("w/home/dir");
        }
        if case == "deep" {
            // a file including itself is read once, so a chain of distinct
            // files is what nests
            for at in 0..12 {
                std::fs::write(
                    home.join(format!("c{at}")),
                    format!("[include]\npath = c{}\n", at + 1),
                )
                .unwrap();
            }
            std::fs::write(home.join(".gitconfig"), "[include]\npath = c0\n").unwrap();
        }
        let facts = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
        if case == "deep" {
            // the same chain, its file past git's limit missing, which git
            // reads past
            std::fs::remove_file(home.join("c10")).unwrap();
            let missing = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
            assert_eq!(missing.git_fault, None);
            std::fs::write(home.join("c10"), "[include]\npath = c11\n").unwrap();
        }
        let fault = facts
            .git_fault
            .clone()
            .unwrap_or_else(|| panic!("{case}: no fault"));
        assert!(fault.why.contains(why), "{case}: {fault}");
        let mut spawning = inputs(&project, &home, false);
        spawning.spawns = true;
        assert_eq!(
            BoxPlan::for_run(&spawning, &facts).err(),
            Some(PlanRefusal::Git(fault)),
            "{case}"
        );
        // a run that starts no program reads none of git's files
        let quiet = BoxFacts::read(&inputs(&project, &home, false)).unwrap();
        assert_eq!((quiet.git_fault, quiet.git_home.len()), (None, 0), "{case}");
        assert!(
            BoxPlan::for_run(&inputs(&project, &home, false), &facts).is_ok(),
            "{case}"
        );
    }
}

/// Git's directories outside the root: an ancestor's `.git` is read; a
/// linked worktree's git directory and common directory are, its `.git`
/// file checked; a `.git` file naming `~/.ssh`, a worktree whose `gitdir`
/// leads elsewhere, and a `--separate-git-dir` one are faults; a `.git` at
/// the home directory gets no rule; and the root's own `.git` directory
/// needs none.
#[test]
fn git_s_directories_are_read_when_checked() {
    let scratch = Scratch::new("facts_git_dirs");
    let home = scratch.dir("w/home");
    let real = |path: &str| scratch.path().join(path);
    let dirs = |root: &Path| BoxFacts::read(&git_inputs(root, &home)).unwrap();
    // the root's own
    let own = scratch.dir("w/home/own");
    scratch.dir("w/home/own/.git");
    assert_eq!(dirs(&own).git_dirs, Vec::<PathBuf>::new());
    // a monorepo's subpackage
    scratch.dir("w/home/mono/.git");
    let sub = scratch.dir("w/home/mono/packages/sub");
    assert_eq!(dirs(&sub).git_dirs, [real("w/home/mono/.git")]);
    // a linked worktree
    scratch.dir("w/home/main/.git/worktrees/wt");
    let tree = scratch.dir("w/home/wt");
    std::fs::write(
        real("w/home/wt/.git"),
        format!(
            "gitdir: {}\n",
            real("w/home/main/.git/worktrees/wt").display()
        ),
    )
    .unwrap();
    std::fs::write(real("w/home/main/.git/worktrees/wt/commondir"), "../..\n").unwrap();
    std::fs::write(
        real("w/home/main/.git/worktrees/wt/gitdir"),
        format!("{}\n", real("w/home/wt/.git").display()),
    )
    .unwrap();
    assert_eq!(
        dirs(&tree).git_dirs,
        [
            real("w/home/main/.git/worktrees/wt"),
            real("w/home/main/.git")
        ]
    );
    // its gitdir file leading elsewhere
    std::fs::write(
        real("w/home/main/.git/worktrees/wt/gitdir"),
        format!("{}\n", real("w/home/own/.git").display()),
    )
    .unwrap();
    assert!(
        dirs(&tree)
            .git_fault
            .is_some_and(|fault| fault.why.contains("lead back"))
    );
    // a hostile one naming the home directory's keys
    scratch.dir("w/home/.ssh");
    let hostile = scratch.dir("w/home/hostile");
    std::fs::write(real("w/home/hostile/.git"), "gitdir: ../.ssh\n").unwrap();
    assert!(
        dirs(&hostile)
            .git_fault
            .is_some_and(|fault| fault.why.contains("commondir"))
    );
    // a worktree-shaped directory whose common directory is no git
    // directory, though it leads back
    scratch.dir("w/home/fake/worktrees/f");
    let faked = scratch.dir("w/home/faked");
    std::fs::write(real("w/home/faked/.git"), "gitdir: ../fake/worktrees/f\n").unwrap();
    std::fs::write(real("w/home/fake/worktrees/f/commondir"), "../..\n").unwrap();
    std::fs::write(
        real("w/home/fake/worktrees/f/gitdir"),
        format!("{}\n", real("w/home/faked/.git").display()),
    )
    .unwrap();
    assert!(
        dirs(&faked)
            .git_fault
            .is_some_and(|fault| fault.why.contains("neither a .git"))
    );
    // a separate git dir, a git directory of its own
    scratch.dir("w/home/separate.git/objects");
    scratch.dir("w/home/separate.git/refs");
    scratch.file("w/home/separate.git/HEAD");
    let separate = scratch.dir("w/home/separate");
    std::fs::write(real("w/home/separate/.git"), "gitdir: ../separate.git\n").unwrap();
    assert!(dirs(&separate).git_fault.is_some());
    // the home directory's own, a dotfiles repository's
    scratch.dir("w/home/.git");
    let bare = scratch.dir("w/home/bare");
    assert_eq!(dirs(&bare).git_dirs, Vec::<PathBuf>::new());
    assert_eq!(dirs(&bare).git_fault, None);
    // the same, `HOME` naming it through a symlink
    scratch.link(&home, "w/home_link");
    let linked = BoxFacts::read(&git_inputs(&bare, &real("w/home_link"))).unwrap();
    assert_eq!(linked.git_dirs, Vec::<PathBuf>::new());
}

/// A linked worktree whose files hold relative paths, as git's
/// `worktree.useRelativePaths` writes them, is read as one holding
/// absolute paths is.
#[test]
fn git_s_relative_worktree_is_read() {
    let scratch = Scratch::new("facts_git_relative_worktree");
    let home = scratch.dir("w/home");
    let git_dir = scratch.dir("w/home/main/.git/worktrees/wt");
    let tree = scratch.dir("w/home/wt");
    std::fs::write(tree.join(".git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
    std::fs::write(git_dir.join("commondir"), "../..\n").unwrap();
    std::fs::write(git_dir.join("gitdir"), "../../../../wt/.git\n").unwrap();
    let facts = BoxFacts::read(&git_inputs(&tree, &home)).unwrap();
    assert_eq!(
        facts.git_dirs,
        [git_dir, scratch.path().join("w/home/main/.git")]
    );
}

/// A git directory's file the project plants as a FIFO or a link to
/// `/dev/zero` is a fault, never waited on or read whole.
#[test]
fn git_s_planted_special_files_are_faults() {
    for (case, plant) in [("fifo", "fifo"), ("zero", "/dev/zero")] {
        let scratch = Scratch::new(&format!("facts_git_special_{case}"));
        let home = scratch.dir("w/home");
        let project = scratch.dir("w/home/p");
        scratch.dir("w/home/p/fake/worktrees/x");
        std::fs::write(project.join(".git"), "gitdir: fake/worktrees/x\n").unwrap();
        let commondir = project.join("fake/worktrees/x/commondir");
        if plant == "fifo" {
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(&commondir)
                    .status()
                    .unwrap()
                    .success()
            );
        } else {
            scratch.link(Path::new(plant), "w/home/p/fake/worktrees/x/commondir");
        }
        let facts = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
        assert!(
            facts
                .git_fault
                .is_some_and(|fault| fault.why.contains("commondir that isn't a regular file")),
            "{case}"
        );
    }
}

/// The reader reads git's grammar as git 2.47 does: a BOM, `\r\n`, a
/// continued line, a legacy dotted header (`[includeIf.x]`), a variable on
/// its header's line, a quoted trailing space kept, a path that isn't
/// UTF-8, a header with an empty name, and a tab in a value kept a tab.
#[test]
fn git_s_grammar_is_read_as_git_reads_it() {
    use std::os::unix::ffi::OsStrExt as _;
    let scratch = Scratch::new("facts_git_grammar");
    let project = scratch.dir("w/project");
    let home = scratch.dir("w/home");
    let mut config =
        b"\xef\xbb\xbf[core]\r\n\texcludesFile = ~/ig\\\nnore ; a comment\r\n".to_vec();
    config.extend_from_slice(b"[includeIf.x]\n\tpath = ~/inc\n");
    config.extend_from_slice(b"[core] attributesFile = \"~/at \"\n");
    config.extend_from_slice(b"[include]\n\tpath = ~/\xe9\n");
    config.extend_from_slice(b"[ \"x\"]\n[include]\n\tpath = ~/a\tb\n");
    std::fs::write(home.join(".gitconfig"), &config).unwrap();
    for name in [
        &b"ignore"[..],
        b"inc",
        b"at",
        b"at ",
        b"\xe9",
        b"a b",
        b"a\tb",
    ] {
        std::fs::write(home.join(std::ffi::OsStr::from_bytes(name)), "").unwrap();
    }
    let facts = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
    assert_eq!(facts.git_fault, None);
    assert_eq!(
        facts.git_home,
        [
            home.join(".gitconfig"),
            home.join("ignore"),
            home.join("inc"),
            home.join("at "),
            home.join(std::ffi::OsStr::from_bytes(b"\xe9")),
            home.join("a\tb"),
        ]
    );
}

/// In a subpackage, the tree that gets no rule is the repository's, from
/// the top that holds the `.git` found: a path git's config names in the
/// repository outside the subpackage, which the repository links to a key,
/// widens nothing.
#[test]
fn git_s_tree_is_the_repository_s_above_a_subpackage() {
    let scratch = Scratch::new("facts_git_subpackage");
    let home = scratch.dir("w/home");
    let mono = scratch.dir("w/home/mono");
    scratch.dir("w/home/mono/.git");
    let package = scratch.dir("w/home/mono/package");
    scratch.dir("w/home/.ssh");
    std::fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
    scratch.dir("w/home/mono/shared");
    scratch.link(&home.join(".ssh/id_ed25519"), "w/home/mono/shared/ignore");
    std::fs::write(
        home.join(".gitconfig"),
        format!(
            "[core]\n\texcludesFile = {}\n",
            mono.join("shared/ignore").display()
        ),
    )
    .unwrap();
    let facts = BoxFacts::read(&git_inputs(&package, &home)).unwrap();
    assert_eq!(facts.git_fault, None);
    assert_eq!(facts.git_dirs, [mono.join(".git")]);
    assert_eq!(facts.git_home, [home.join(".gitconfig")]);
}

/// `GIT_CONFIG_GLOBAL=/dev/null`, git's own way to read no global config,
/// and an include of `/dev/null` read as empty, as git reads them, and
/// refuse nothing.
#[test]
fn git_s_config_at_dev_null_is_empty() {
    let scratch = Scratch::new("facts_git_dev_null");
    let project = scratch.dir("w/project");
    let home = scratch.dir("w/home");
    let facts = BoxFacts::read(&BoxInputs {
        git_config_global: Some("/dev/null".into()),
        ..git_inputs(&project, &home)
    })
    .unwrap();
    assert_eq!(facts.git_fault, None);
    assert_eq!(facts.git_home, [PathBuf::from("/dev/null")]);
    std::fs::write(home.join(".gitconfig"), "[include]\n\tpath = /dev/null\n").unwrap();
    let facts = BoxFacts::read(&git_inputs(&project, &home)).unwrap();
    assert_eq!(facts.git_fault, None);
    assert_eq!(
        facts.git_home,
        [home.join(".gitconfig"), PathBuf::from("/dev/null")]
    );
}

/// A git directory of its own: `HEAD`, `objects`, and `refs` under `dir`,
/// and a `config` holding `config`.
fn git_dir(dir: &Path, config: &str) {
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    std::fs::create_dir_all(dir.join("refs")).unwrap();
    std::fs::write(dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(dir.join("config"), config).unwrap();
}

/// A submodule's checkout is read when its git directory lies beneath
/// `<super>/.git/modules/`, nested modules too, and its `core.worktree`
/// leads back to the checkout; another checkout's submodule, a git
/// directory outside any `modules/`, one with no `core.worktree`, and one
/// whose `config` is a planted FIFO are faults.
#[test]
fn git_s_submodules_are_read_by_backlink() {
    let scratch = Scratch::new("facts_git_submodules");
    let home = scratch.dir("w/home");
    let real = |path: &str| scratch.path().join(path);
    let dirs = |root: &Path| BoxFacts::read(&git_inputs(root, &home)).unwrap();
    let fault = |root: &Path, says: &str| {
        let facts = dirs(root);
        assert!(
            facts
                .git_fault
                .as_ref()
                .is_some_and(|fault| fault.why.contains(says)),
            "{}: {:?}",
            root.display(),
            facts.git_fault
        );
    };
    // a submodule, and one nested in it
    git_dir(
        &real("w/home/super/.git/modules/sub"),
        "[core]\n\tworktree = ../../../sub\n",
    );
    let sub = scratch.dir("w/home/super/sub");
    std::fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").unwrap();
    assert_eq!(dirs(&sub).git_dirs, [real("w/home/super/.git/modules/sub")]);
    git_dir(
        &real("w/home/super/.git/modules/sub/modules/inner"),
        "[core]\n\tworktree = ../../../../../sub/inner\n",
    );
    let inner = scratch.dir("w/home/super/sub/inner");
    std::fs::write(
        inner.join(".git"),
        "gitdir: ../../.git/modules/sub/modules/inner\n",
    )
    .unwrap();
    assert_eq!(
        dirs(&inner).git_dirs,
        [real("w/home/super/.git/modules/sub/modules/inner")]
    );
    // a hostile checkout naming another's submodule
    let hostile = scratch.dir("w/home/hostile");
    std::fs::write(hostile.join(".git"), "gitdir: ../super/.git/modules/sub\n").unwrap();
    fault(&hostile, "doesn't lead back");
    // a git directory outside any modules/
    git_dir(&real("w/home/loose"), "[core]\n\tworktree = ../outside\n");
    let outside = scratch.dir("w/home/outside");
    std::fs::write(outside.join(".git"), "gitdir: ../loose\n").unwrap();
    fault(&outside, "isn't a submodule's");
    // no core.worktree
    git_dir(
        &real("w/home/bare_super/.git/modules/m"),
        "[core]\n\tbare = false\n",
    );
    let unset = scratch.dir("w/home/bare_super/m");
    std::fs::write(unset.join(".git"), "gitdir: ../.git/modules/m\n").unwrap();
    fault(&unset, "sets no core.worktree");
    // a planted FIFO for its config
    git_dir(&real("w/home/fifo_super/.git/modules/f"), "");
    std::fs::remove_file(real("w/home/fifo_super/.git/modules/f/config")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(real("w/home/fifo_super/.git/modules/f/config"))
            .status()
            .unwrap()
            .success()
    );
    let fifo = scratch.dir("w/home/fifo_super/f");
    std::fs::write(fifo.join(".git"), "gitdir: ../.git/modules/f\n").unwrap();
    fault(&fifo, "no config it can read");
}

/// A linked worktree's files with `\r` and `\n` at their ends, as git
/// reads them, are read as the worktree's.
#[test]
fn git_s_worktree_files_are_read_past_their_line_ends() {
    let scratch = Scratch::new("facts_git_line_ends");
    let home = scratch.dir("w/home");
    let git_dir = scratch.dir("w/home/main/.git/worktrees/wt");
    let tree = scratch.dir("w/home/wt");
    std::fs::write(
        tree.join(".git"),
        format!("gitdir: {}\r\n\n", git_dir.display()),
    )
    .unwrap();
    std::fs::write(git_dir.join("commondir"), "../..\r\n\n").unwrap();
    std::fs::write(
        git_dir.join("gitdir"),
        format!("{}\r\n", tree.join(".git").display()),
    )
    .unwrap();
    let facts = BoxFacts::read(&git_inputs(&tree, &home)).unwrap();
    assert_eq!(facts.git_fault, None);
    assert_eq!(
        facts.git_dirs,
        [git_dir, scratch.path().join("w/home/main/.git")]
    );
}

/// A submodule checked out in a linked worktree, its git directory beneath
/// `<super>/.git/worktrees/<w>/modules/`, is read when its `core.worktree`
/// leads back to the checkout; a hostile checkout naming it is a fault.
#[test]
fn git_s_worktree_submodules_are_read_by_backlink() {
    let scratch = Scratch::new("facts_git_worktree_submodules");
    let home = scratch.dir("w/home");
    let real = |path: &str| scratch.path().join(path);
    git_dir(
        &real("w/home/super/.git/worktrees/w/modules/sub"),
        "[core]\n\tworktree = ../../../../../../w/sub\n",
    );
    let sub = scratch.dir("w/home/w/sub");
    std::fs::write(
        sub.join(".git"),
        "gitdir: ../../super/.git/worktrees/w/modules/sub\n",
    )
    .unwrap();
    let facts = BoxFacts::read(&git_inputs(&sub, &home)).unwrap();
    assert_eq!(facts.git_fault, None);
    assert_eq!(
        facts.git_dirs,
        [real("w/home/super/.git/worktrees/w/modules/sub")]
    );
    let hostile = scratch.dir("w/home/hostile");
    std::fs::write(
        hostile.join(".git"),
        "gitdir: ../super/.git/worktrees/w/modules/sub\n",
    )
    .unwrap();
    let facts = BoxFacts::read(&git_inputs(&hostile, &home)).unwrap();
    assert!(
        facts
            .git_fault
            .is_some_and(|fault| fault.why.contains("doesn't lead back")),
    );
    // a git directory in a worktree's tree but not beneath its modules/
    git_dir(
        &real("w/home/super/.git/worktrees/w/other"),
        "[core]\n\tworktree = ../../../../../stray\n",
    );
    let stray = scratch.dir("w/home/stray");
    std::fs::write(
        stray.join(".git"),
        "gitdir: ../super/.git/worktrees/w/other\n",
    )
    .unwrap();
    let facts = BoxFacts::read(&git_inputs(&stray, &home)).unwrap();
    assert!(
        facts
            .git_fault
            .is_some_and(|fault| fault.why.contains("isn't a submodule's")),
    );
}
