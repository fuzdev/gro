//! Tests for the box's plan: each command's rules, whole, and each flag's,
//! from inputs and facts written here, every rule rendered as one line.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::confine::plan::{
    Anchor, BoxFacts, BoxInputs, BoxPlan, DirFlag, EntryKind, GrantedFlagDir, NodeModules, Opening,
    PlanRefusal, PlannedRule, ProgramFile, Skip, TopEntry,
};
use crate::confine::rules::{Need, RealRights, Rights};

/// `names` as a set.
fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

/// A project at `/p` with a top-level entry of each kind the plan tells
/// apart, an installed `node_modules`, a split `/lib64` beside a merged
/// `/bin` and `/lib`, a linked package, and git's home config.
fn facts() -> BoxFacts {
    let entry = |name: &str, kind| TopEntry {
        name: name.into(),
        kind,
    };
    let file = EntryKind::File { links: 1 };
    BoxFacts {
        entries: vec![
            entry(".env", file),
            entry(".git", EntryKind::Directory),
            entry(".svelte-kit", EntryKind::Directory),
            entry("AGENTS.md", EntryKind::Symlink),
            entry("CLAUDE.md", file),
            entry("fifo", EntryKind::Other),
            entry("linked.txt", EntryKind::File { links: 2 }),
            entry("node_modules", EntryKind::Directory),
            entry("package.json", file),
            entry("src", EntryKind::Directory),
        ],
        node_modules: NodeModules::Directory,
        system: vec![
            ("/bin".into(), EntryKind::Symlink),
            ("/lib".into(), EntryKind::Symlink),
            ("/lib64".into(), EntryKind::Directory),
        ],
        linked: vec!["/elsewhere/pkg".into()],
        git_home: vec!["/h/.gitconfig".into()],
    }
}

/// The `node` the project's tools run under.
fn node() -> ProgramFile {
    ProgramFile::Node {
        found: "/h/bin/node".into(),
        real: "/h/node/bin/node".into(),
        script: false,
    }
}

/// A run at `/p` of a command whose closure writes the root when `writes`,
/// grants the local `tools`, and pins `programs`, in a project declaring
/// `declared`.
fn inputs(
    writes: bool,
    tools: &[&str],
    declared: &[&str],
    programs: Vec<ProgramFile>,
) -> BoxInputs {
    BoxInputs {
        root: "/p".into(),
        writes_root: writes,
        tools: set(tools),
        spawns: !programs.is_empty(),
        declared: set(declared),
        programs,
        home: Some("/h".into()),
        ..BoxInputs::default()
    }
}

/// `rights` as `r`, `w`, and `x`.
fn rights(rights: Rights) -> &'static str {
    let (r, w, x) = (Rights::READ, Rights::WRITE, Rights::EXECUTE);
    [
        (r, "r"),
        (w, "w"),
        (x, "x"),
        (r | w, "rw"),
        (r | x, "rx"),
        (w | x, "wx"),
        (r | w | x, "rwx"),
    ]
    .into_iter()
    .find_map(|(each, name)| (each == rights).then_some(name))
    .unwrap()
}

/// A rule as one line: how it opens, its path, its rights, its need, and
/// its source.
fn line(rule: &PlannedRule) -> String {
    let need = match rule.need {
        Need::Required => "req",
        Need::Optional => "opt",
    };
    let opening = match &rule.opening {
        Opening::Real { path, rights } => {
            assert_eq!(rule.rights(), Rights::from(*rights));
            let rights = match rights {
                RealRights::Read => "r",
                RealRights::ReadExecute => "rx",
            };
            format!("real {} {rights}", path.display())
        }
        Opening::Beneath {
            anchor,
            path,
            rights: granted,
        } => {
            assert_eq!(rule.rights(), *granted);
            let anchor = match anchor {
                Anchor::Project => "project",
                Anchor::Slash => "/",
                Anchor::PrivateTemp => "temp",
            };
            format!("{anchor} {} {}", path.display(), rights(*granted))
        }
    };
    format!("{opening} {need} {:?}", rule.source)
}

/// The plan's rules, one line each, then what it skipped.
fn render(plan: &BoxPlan) -> Vec<String> {
    plan.rules()
        .iter()
        .map(line)
        .chain(
            plan.skipped()
                .iter()
                .map(|skipped| format!("skip {} {:?}", skipped.path.display(), skipped.why)),
        )
        .collect()
}

/// The lines every boxed run starts with, after the project's.
const SYSTEM: &[&str] = &[
    "project . r req Project",
    "real /elsewhere/pkg LINKED opt Linked",
    "real /usr rx req System",
    "real /etc r req System",
    "real /lib64 rx opt System",
    "real /dev/zero r opt Device",
    "real /dev/urandom r opt Device",
    "real /dev/random r opt Device",
    "/ dev/null rw req Device",
    "real /proc r req Proc",
];

/// The per-entry write set of [`facts`].
const ENTRIES: &[&str] = &[
    "project CLAUDE.md w req Entry",
    "project package.json w req Entry",
    "project src w req Entry",
];

/// What a run that may start a program gets, but its programs'.
const SPAWNING: &[&str] = &[
    "project node_modules rx opt NodeModules",
    "real /h/.gitconfig r opt GitHome",
    "temp . rwx req PrivateTemp",
];

/// What [`facts`]' plan skips, in the order it meets them.
const SKIPPED: &[&str] = &[
    "skip /bin Covered",
    "skip /lib Covered",
    "skip /p/AGENTS.md Symlink",
    "skip /p/fifo Other",
    "skip /p/linked.txt Linked",
];

/// `lines` joined from parts, the system's with the linked package's
/// rights filled in.
fn expected(spawns: bool, parts: &[&[&str]]) -> Vec<String> {
    let linked = if spawns { "rx" } else { "r" };
    let mut lines: Vec<String> = SYSTEM
        .iter()
        .map(|line| line.replace("LINKED", linked))
        .collect();
    for part in parts {
        lines.extend(part.iter().map(|&line| line.to_owned()));
    }
    let entries = parts.contains(&ENTRIES);
    lines.extend(
        SKIPPED
            .iter()
            .filter(|line| entries || line.contains("Covered"))
            .map(|&line| line.to_owned()),
    );
    lines
}

/// Each command's plan, whole: what it reads, writes, and executes, and
/// what it skips.
#[test]
fn each_command_plans_its_box() {
    let kit_vite = ["@sveltejs/kit", "vite"];
    let every = ["@sveltejs/kit", "vite", "vitest", "@fuzdev/fuz_css"];
    let git = ProgramFile::Direct {
        real: "/usr/bin/git".into(),
    };
    for (command, inputs, lines) in [
        (
            "--version",
            inputs(false, &[], &every, Vec::new()),
            expected(false, &[]),
        ),
        (
            "format",
            inputs(true, &[], &every, Vec::new()),
            expected(false, &[ENTRIES]),
        ),
        (
            "format --check",
            inputs(false, &[], &every, Vec::new()),
            expected(false, &[]),
        ),
        (
            "gen",
            inputs(true, &["svelte-kit"], &kit_vite, vec![node()]),
            expected(
                true,
                &[
                    ENTRIES,
                    &["real /h/node/bin/node rx req Node"],
                    SPAWNING,
                    &[
                        "project .svelte-kit w req ToolDir(SvelteKit)",
                        "project node_modules/$app w req ToolDir(App)",
                        "project node_modules/.vite w req ToolDir(Vite)",
                        "project node_modules/.vite-temp w req ToolDir(ViteTemp)",
                    ],
                ],
            ),
        ),
        (
            "gen --check",
            inputs(false, &["svelte-kit"], &kit_vite, vec![node()]),
            expected(
                true,
                &[
                    &["real /h/node/bin/node rx req Node"],
                    SPAWNING,
                    &[
                        "project .svelte-kit w req ToolDir(SvelteKit)",
                        "project node_modules/$app w req ToolDir(App)",
                        "project node_modules/.vite w req ToolDir(Vite)",
                        "project node_modules/.vite-temp w req ToolDir(ViteTemp)",
                    ],
                ],
            ),
        ),
        (
            "check",
            inputs(
                false,
                &["eslint", "svelte-check", "svelte-kit", "vitest"],
                &every,
                vec![node(), git],
            ),
            expected(
                true,
                &[
                    &[
                        "real /h/node/bin/node rx req Node",
                        "real /usr/bin/git rx req Program",
                    ],
                    SPAWNING,
                    &[
                        "project .svelte-kit w req ToolDir(SvelteKit)",
                        "project node_modules/$app w req ToolDir(App)",
                        "project node_modules/.vite w req ToolDir(Vite)",
                        "project node_modules/.vite-temp w req ToolDir(ViteTemp)",
                        "project .fuz/cache w req ToolDir(FuzCache)",
                    ],
                ],
            ),
        ),
        // eslint's plugins run no fuz_css transform, and vitest alone
        // declares `.vite`
        (
            "lint",
            inputs(
                false,
                &["eslint", "svelte-kit"],
                &["vitest", "@fuzdev/fuz_css"],
                vec![node()],
            ),
            expected(
                true,
                &[
                    &["real /h/node/bin/node rx req Node"],
                    SPAWNING,
                    &[
                        "project node_modules/.vite w req ToolDir(Vite)",
                        "project node_modules/.vite-temp w req ToolDir(ViteTemp)",
                    ],
                ],
            ),
        ),
        // a project declaring none of the tools' packages
        (
            "test",
            inputs(false, &["vitest"], &["zod"], vec![node()]),
            expected(true, &[&["real /h/node/bin/node rx req Node"], SPAWNING]),
        ),
    ] {
        let plan = BoxPlan::for_run(&inputs, &facts()).unwrap();
        assert_eq!(render(&plan), lines, "{command}");
        assert_eq!(plan.private_temp(), inputs.spawns, "{command}");
        assert_eq!(plan.note(), None, "{command}");
        assert_eq!(
            plan.tool_dirs().count(),
            lines.iter().filter(|line| line.contains("ToolDir")).count(),
            "{command}"
        );
    }
}

/// `check` with the grant flags `flags`, each resolved to its real path.
fn flagged(flags: &[(DirFlag, &str, &str)]) -> BoxInputs {
    BoxInputs {
        flags: flags
            .iter()
            .map(|&(flag, dir, real)| GrantedFlagDir {
                flag,
                dir: dir.to_owned(),
                real: real.into(),
            })
            .collect(),
        ..inputs(false, &[], &[], Vec::new())
    }
}

/// A read flag reads by real path, a run-in flag reads and executes, a
/// write flag writes beneath `/`, an excluded entry included, and one
/// naming the root by any spelling is the per-entry set.
#[test]
fn grant_flags_widen_the_box() {
    for (case, inputs, added) in [
        (
            "read",
            flagged(&[(DirFlag::Read, "../data", "/data")]),
            vec!["real /data r req Flag(Read)"],
        ),
        (
            "run-in",
            flagged(&[(DirFlag::RunIn, "/opt/x", "/opt/x")]),
            vec!["real /opt/x rx req Flag(RunIn)"],
        ),
        (
            "write beside",
            flagged(&[(DirFlag::Write, "../out", "/out")]),
            vec!["/ out rw req Flag(Write)"],
        ),
        (
            "write an excluded entry",
            flagged(&[(DirFlag::Write, ".git", "/p/.git")]),
            vec!["/ p/.git rw req Flag(Write)"],
        ),
    ] {
        let plan = BoxPlan::for_run(&inputs, &facts()).unwrap();
        let mut lines = expected(false, &[]);
        lines.splice(
            lines.len() - 2..lines.len() - 2,
            added.iter().map(|&line| line.to_owned()),
        );
        assert_eq!(render(&plan), lines, "{case}");
    }
    for spelling in [".", "./", "../p"] {
        let plan = BoxPlan::for_run(&flagged(&[(DirFlag::Write, spelling, "/p")]), &facts());
        assert_eq!(
            render(&plan.unwrap()),
            expected(false, &[ENTRIES]),
            "{spelling}"
        );
    }
}

/// `-A`, a write flag above the root, a script `node` no run-in flag
/// admits, and a `node_modules` symlink above the root are refused; a read
/// or run-in flag above the root isn't.
#[test]
fn the_plan_refuses_what_no_box_holds() {
    let shim = ProgramFile::Node {
        found: "/h/.asdf/shims/node".into(),
        real: "/h/.asdf/shims/node".into(),
        script: true,
    };
    let mut unconfined = inputs(false, &[], &[], Vec::new());
    unconfined.unconfined = true;
    let mut run_in = flagged(&[(DirFlag::RunIn, "~/.asdf", "/h/.asdf")]);
    run_in.programs = vec![shim.clone()];
    run_in.spawns = true;
    let mut read = flagged(&[(DirFlag::Read, "~/.asdf", "/h/.asdf")]);
    read.programs = vec![shim.clone()];
    read.spawns = true;
    // a `node` found through a link of its own, whose real path the
    // run-in flag covers
    let mut linked_shim = run_in.clone();
    linked_shim.programs = vec![ProgramFile::Node {
        found: "/h/bin/node".into(),
        real: "/h/.asdf/shims/node".into(),
        script: true,
    }];
    let mut deep = flagged(&[(DirFlag::Write, "~", "/h")]);
    deep.root = "/h/dev/p".into();
    let mut nested = inputs(false, &[], &[], Vec::new());
    nested.root = "/w/node_modules/p".into();
    let linked = |real: &str| {
        let mut linked = facts();
        linked.node_modules = NodeModules::Link { real: real.into() };
        linked
    };
    for (case, inputs, facts, refusal) in [
        ("-A", unconfined, facts(), Some(PlanRefusal::Unconfined)),
        (
            "write ..",
            flagged(&[(DirFlag::Write, "..", "/")]),
            facts(),
            Some(PlanRefusal::WriteAboveRoot {
                dir: "..".into(),
                real: "/".into(),
            }),
        ),
        (
            "read ..",
            flagged(&[(DirFlag::Read, "..", "/")]),
            facts(),
            None,
        ),
        (
            "run-in ..",
            flagged(&[(DirFlag::RunIn, "..", "/")]),
            facts(),
            None,
        ),
        (
            "a script node",
            inputs(false, &["vitest"], &[], vec![shim]),
            facts(),
            Some(PlanRefusal::ScriptNode {
                found: "/h/.asdf/shims/node".into(),
            }),
        ),
        ("a script node a run-in flag admits", run_in, facts(), None),
        (
            "a script node a read flag doesn't admit",
            read,
            facts(),
            Some(PlanRefusal::ScriptNode {
                found: "/h/.asdf/shims/node".into(),
            }),
        ),
        (
            "a script node found through a link, admitted by its real path",
            linked_shim,
            facts(),
            None,
        ),
        (
            "write an ancestor but /",
            deep,
            facts(),
            Some(PlanRefusal::WriteAboveRoot {
                dir: "~".into(),
                real: "/h".into(),
            }),
        ),
        (
            "node_modules to /",
            inputs(false, &[], &[], Vec::new()),
            linked("/"),
            Some(PlanRefusal::NodeModulesLink { real: "/".into() }),
        ),
        (
            "node_modules to a node_modules holding the project",
            nested,
            linked("/w/node_modules"),
            Some(PlanRefusal::NodeModulesLink {
                real: "/w/node_modules".into(),
            }),
        ),
        (
            "node_modules into the home directory",
            inputs(false, &[], &[], Vec::new()),
            linked("/h/.ssh"),
            Some(PlanRefusal::NodeModulesLink {
                real: "/h/.ssh".into(),
            }),
        ),
        (
            "node_modules into the project",
            inputs(false, &[], &[], Vec::new()),
            linked("/p/.git"),
            Some(PlanRefusal::NodeModulesLink {
                real: "/p/.git".into(),
            }),
        ),
        (
            "node_modules named so, inside the project",
            inputs(false, &[], &[], Vec::new()),
            linked("/p/src/node_modules"),
            Some(PlanRefusal::NodeModulesLink {
                real: "/p/src/node_modules".into(),
            }),
        ),
        (
            "another checkout's node_modules",
            inputs(false, &[], &[], Vec::new()),
            linked("/h/dev/q/node_modules"),
            None,
        ),
    ] {
        assert_eq!(BoxPlan::for_run(&inputs, &facts).err(), refusal, "{case}");
    }
}

/// A `node_modules` that is a symlink is followed: read, and read and
/// executed for a run that may start a program, at its real path, with a
/// note naming where it leads; its tool directories are made there, beneath
/// `/`, only when a write flag names it, and the note says which.
#[test]
fn a_symlinked_node_modules_is_followed() {
    let mut linked = facts();
    linked.node_modules = NodeModules::Link {
        real: "/shared/node_modules".into(),
    };
    let kit = || {
        inputs(
            false,
            &["svelte-kit"],
            &["@sveltejs/kit", "vite"],
            vec![node()],
        )
    };
    let in_target = [
        "/ shared/node_modules/$app w req ToolDir(App)",
        "/ shared/node_modules/.vite w req ToolDir(Vite)",
        "/ shared/node_modules/.vite-temp w req ToolDir(ViteTemp)",
    ];
    // no write flag: read and execute alone
    let plan = BoxPlan::for_run(&kit(), &linked).unwrap();
    let rendered = render(&plan);
    assert_eq!(rendered[1], "real /shared/node_modules rx req NodeModules");
    assert!(
        !rendered.iter().any(|line| line.contains("node_modules/")),
        "{rendered:#?}"
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("ToolDir(SvelteKit)"))
    );
    assert_eq!(
        plan.note().as_deref(),
        Some(
            "note: node_modules is a symlink to /shared/node_modules, which the box follows to \
             read and execute, and no tool directory is made in it: --allow-write= naming it lets \
             the tools write there"
        )
    );
    // a write flag naming it, or a directory holding it
    for (dir, real) in [
        ("../shared/node_modules", "/shared/node_modules"),
        ("/shared", "/shared"),
    ] {
        let mut flagged = kit();
        flagged.flags = vec![GrantedFlagDir {
            flag: DirFlag::Write,
            dir: dir.to_owned(),
            real: real.into(),
        }];
        let plan = BoxPlan::for_run(&flagged, &linked).unwrap();
        let rendered = render(&plan);
        for line in in_target {
            assert!(
                rendered.iter().any(|each| each == line),
                "{line}: {rendered:#?}"
            );
        }
        assert_eq!(
            plan.note().as_deref(),
            Some(
                "note: node_modules is a symlink to /shared/node_modules, which the box follows \
                 to read and execute, and --allow-write= naming it lets the tools write their \
                 directories there"
            ),
            "{dir}"
        );
    }
    // a write flag naming it on a run whose tools call for no directory
    // there: the note still names the flag as given, never asks for it
    let mut toolless = inputs(false, &[], &[], vec![node()]);
    toolless.flags = vec![GrantedFlagDir {
        flag: DirFlag::Write,
        dir: "/shared".to_owned(),
        real: "/shared".into(),
    }];
    let given = BoxPlan::for_run(&toolless, &linked).unwrap();
    assert!(given.tool_dirs().next().is_none());
    assert!(
        given
            .note()
            .is_some_and(|note| note.ends_with("lets the tools write their directories there")),
        "{:?}",
        given.note()
    );
    assert!(
        !render(&plan)
            .iter()
            .any(|line| line.starts_with("project node_modules")),
    );
    assert_eq!(
        plan.node_modules_link(),
        Some(std::path::Path::new("/shared/node_modules"))
    );
    // read alone for a run that starts nothing
    let plan = BoxPlan::for_run(&inputs(true, &[], &[], Vec::new()), &linked).unwrap();
    assert_eq!(
        render(&plan)[1],
        "real /shared/node_modules r req NodeModules"
    );
}

/// Without `node_modules`, no tool directory is planned in it, and nothing
/// reads or executes it.
#[test]
fn no_node_modules_plans_none_in_it() {
    let mut bare = facts();
    bare.node_modules = NodeModules::Absent;
    let plan = BoxPlan::for_run(
        &inputs(
            false,
            &["svelte-kit"],
            &["@sveltejs/kit", "vite"],
            vec![node()],
        ),
        &bare,
    )
    .unwrap();
    let rendered = render(&plan);
    assert!(
        !rendered.iter().any(|line| line.contains("node_modules")),
        "{rendered:#?}"
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("ToolDir(SvelteKit)"))
    );
    assert_eq!(
        plan.skipped()
            .iter()
            .map(|skipped| skipped.why)
            .collect::<Vec<_>>(),
        [Skip::Covered, Skip::Covered]
    );
    assert_eq!(plan.root(), PathBuf::from("/p"));
}

/// `--allow-temp` alone gets the run's private directory, as a run that
/// starts a program does.
#[test]
fn allow_temp_gets_the_private_directory() {
    let mut temp = inputs(false, &[], &[], Vec::new());
    temp.temp = true;
    let plan = BoxPlan::for_run(&temp, &facts()).unwrap();
    assert!(plan.private_temp());
    let mut lines = expected(false, &[]);
    lines.insert(lines.len() - 2, "temp . rwx req PrivateTemp".to_owned());
    assert_eq!(render(&plan), lines);
}
