//! Tests for the box's plan: each command's rules, whole, and each flag's,
//! from inputs and facts written here, every rule rendered as one line.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::confine::plan::{
    Anchor, BoxFacts, BoxInputs, BoxPlan, DirFlag, EntryKind, GRO_CODE, GrantedFlagDir,
    NodeModules, Opening, PlanRefusal, PlannedRule, ProgramFile, RootWriter, RuleSource,
    RunWriteDir, RunWriteGrant, Skip, ToolDir, ToolDirs, ToolWrites, TopEntry,
};
use crate::confine::program::ProgramWrites;
use crate::confine::rules::{Need, RealRights, Rights};
use crate::verb::table::BuiltinGrantsTable;

/// `names` as a set.
fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

/// The programs of the grants table named `names`, as the closure grants
/// each that writes tool directories, by its name in the run, with the
/// `write` the carried table names for it.
pub(super) fn table_tools(names: &[&str]) -> BTreeMap<String, ToolWrites> {
    let table = BuiltinGrantsTable::carried().unwrap();
    names
        .iter()
        .filter_map(|&name| {
            let grant = table
                .builtins
                .values()
                .flatten()
                .flat_map(|grants| grants.programs.values())
                .find(|grant| grant.name == name)
                .unwrap();
            (!grant.write.is_empty()).then(|| {
                let tool = ToolWrites {
                    name: name.to_owned(),
                    tool_dirs: grant.write.iter().copied().collect(),
                };
                (format!("node_modules/.bin/{name}"), tool)
            })
        })
        .collect()
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
        git_dirs: vec!["/m/.git".into()],
        git_fault: None,
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
/// grants the grants table's `tools`, and pins `programs`, in a project declaring
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
        tools: table_tools(tools),
        spawns: !programs.is_empty(),
        git: !programs.is_empty(),
        declared: set(declared),
        programs,
        home: Some("/h".into()),
        ..BoxInputs::default()
    }
}

/// `rights` as `r`, `w`, and `x`, a plain write (`Rights::WRITE_PLAIN`)
/// as `p`.
fn rights(rights: Rights) -> &'static str {
    let (r, w, p, x) = (
        Rights::READ,
        Rights::WRITE,
        Rights::WRITE_PLAIN,
        Rights::EXECUTE,
    );
    [
        (r, "r"),
        (w, "w"),
        (p, "p"),
        (x, "x"),
        (r | w, "rw"),
        (r | p, "rp"),
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
    "real /m/.git r opt GitDir",
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
    let every = ["@sveltejs/kit", "vite", "vitest"];
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
                        "project .svelte-kit p req ToolDir(SvelteKit)",
                        "project node_modules/$app p req ToolDir(App)",
                        "project node_modules/.vite-temp p req ToolDir(ViteTemp)",
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
                        "project .svelte-kit p req ToolDir(SvelteKit)",
                        "project node_modules/$app p req ToolDir(App)",
                        "project node_modules/.vite-temp p req ToolDir(ViteTemp)",
                    ],
                ],
            ),
        ),
        (
            "check",
            inputs(
                false,
                &["eslint", "svelte-check", "svelte-kit", "tsc", "vitest"],
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
                        "project .svelte-kit p req ToolDir(SvelteKit)",
                        "project .svelte-kit/generated/dev p req ToolDir(SvelteKitDev)",
                        "project .svelte-kit/types p req ToolDir(SvelteKitTypes)",
                        "project node_modules/$app p req ToolDir(App)",
                        "project node_modules/.vite-temp p req ToolDir(ViteTemp)",
                        "project node_modules/.vite/vitest p req ToolDir(Vitest)",
                    ],
                ],
            ),
        ),
        // svelte-check's and svelte-kit's own, never vitest's
        (
            "typecheck",
            inputs(
                false,
                &["svelte-check", "svelte-kit", "tsc"],
                &every,
                vec![node()],
            ),
            expected(
                true,
                &[
                    &["real /h/node/bin/node rx req Node"],
                    SPAWNING,
                    &[
                        "project .svelte-kit p req ToolDir(SvelteKit)",
                        "project .svelte-kit/generated/dev p req ToolDir(SvelteKitDev)",
                        "project node_modules/$app p req ToolDir(App)",
                        "project node_modules/.vite-temp p req ToolDir(ViteTemp)",
                    ],
                ],
            ),
        ),
        // vitest's own alone: none of `.svelte-kit` whole, nor `$app`
        (
            "test",
            inputs(false, &["vitest"], &every, vec![node()]),
            expected(
                true,
                &[
                    &["real /h/node/bin/node rx req Node"],
                    SPAWNING,
                    &[
                        "project .svelte-kit/generated/dev p req ToolDir(SvelteKitDev)",
                        "project .svelte-kit/types p req ToolDir(SvelteKitTypes)",
                        "project node_modules/.vite-temp p req ToolDir(ViteTemp)",
                        "project node_modules/.vite/vitest p req ToolDir(Vitest)",
                    ],
                ],
            ),
        ),
        // eslint writes none, and of svelte-kit's own the project declares
        // only what `.vite-temp` takes
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
                    &["project node_modules/.vite-temp p req ToolDir(ViteTemp)"],
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
        assert_eq!(plan.note(check_command), None, "{command}");
        assert_eq!(
            plan.tool_dirs().count(),
            lines.iter().filter(|line| line.contains("ToolDir")).count(),
            "{command}"
        );
    }
}

/// Each program of the grants table writes the tool directories its
/// `write` names and no other's, beside what `--allow-run-write=` names for
/// it; a program of the table that names none writes nothing of its own;
/// and a run that starts no program has no write set at all.
#[test]
fn each_program_writes_its_own_tool_directories() {
    let mut check = inputs(
        false,
        &["eslint", "svelte-check", "svelte-kit", "tsc", "vitest"],
        &["@sveltejs/kit", "vite", "vitest"],
        vec![node()],
    );
    check.run_writes = vec![vitest_writes(&[("src", "/p/src")])];
    let plan = BoxPlan::for_run(&check, &facts()).unwrap();
    let dirs = |dirs: &[ToolDir]| dirs.iter().copied().collect::<ToolDirs>();
    let own = |dirs: ToolDirs| ProgramWrites {
        tool_dirs: dirs,
        ..ProgramWrites::NONE
    };
    let kit = dirs(&[ToolDir::SvelteKit, ToolDir::App, ToolDir::ViteTemp]);
    let svelte_check = dirs(&[ToolDir::SvelteKitDev, ToolDir::ViteTemp]);
    let vitest = dirs(&[
        ToolDir::SvelteKitDev,
        ToolDir::SvelteKitTypes,
        ToolDir::ViteTemp,
        ToolDir::Vitest,
    ]);
    assert_eq!(
        plan.writes_by_name(),
        BTreeMap::from([
            (
                "node_modules/.bin/svelte-check".to_owned(),
                own(svelte_check)
            ),
            ("node_modules/.bin/svelte-kit".to_owned(), own(kit)),
            (
                "node_modules/.bin/vitest".to_owned(),
                ProgramWrites {
                    tool_dirs: vitest,
                    root: false,
                    run_write: Some(0),
                }
            ),
        ])
    );
    assert_eq!(
        plan.program_writes(),
        [
            ProgramWrites::NONE,
            own(svelte_check),
            own(kit),
            ProgramWrites {
                tool_dirs: vitest,
                root: false,
                run_write: Some(0),
            },
        ]
    );
    // no program but vitest writes its cache, and none writes what `dev`
    // optimizes beside it
    let writers = |dir: ToolDir| -> Vec<String> {
        plan.writes_by_name()
            .into_iter()
            .filter(|(_, writes)| writes.tool_dirs.contains(dir))
            .map(|(program, _)| program)
            .collect()
    };
    assert_eq!(writers(ToolDir::Vitest), ["node_modules/.bin/vitest"]);
    assert_eq!(
        writers(ToolDir::SvelteKit),
        ["node_modules/.bin/svelte-kit"]
    );
    assert_eq!(
        writers(ToolDir::SvelteKitTypes),
        ["node_modules/.bin/vitest"]
    );
    assert_eq!(writers(ToolDir::App), ["node_modules/.bin/svelte-kit"]);
    check.spawns = false;
    let still = BoxPlan::for_run(&check, &facts()).unwrap();
    assert!(still.writes_by_name().is_empty() && still.program_writes().is_empty());
}

/// A directory `--allow-run-write=` names is written plain, as the tool
/// directories are, where it is one, lies in one or in where a linked
/// `node_modules` leads, or holds one, by whole components; every other
/// keeps the whole write.
#[test]
fn a_run_write_flag_in_a_tool_directory_writes_plain() {
    let run_write = |dirs: &[(&str, &str)], facts: &BoxFacts| -> Vec<String> {
        let mut named = inputs(
            false,
            &["svelte-kit", "vitest"],
            &["@sveltejs/kit", "vite", "vitest"],
            vec![node()],
        );
        named.run_writes = vec![vitest_writes(dirs)];
        render(&BoxPlan::for_run(&named, facts).unwrap())
            .into_iter()
            .filter(|line| line.contains("RunWrite"))
            .collect()
    };
    assert_eq!(
        run_write(
            &[
                (".svelte-kit/generated/dev", "/p/.svelte-kit/generated/dev"),
                (
                    ".svelte-kit/generated/dev/x",
                    "/p/.svelte-kit/generated/dev/x"
                ),
                (".svelte-kit/generated", "/p/.svelte-kit/generated"),
                ("node_modules", "/p/node_modules"),
                (".svelte-kit2", "/p/.svelte-kit2"),
                ("src", "/p/src"),
                ("/elsewhere", "/elsewhere"),
            ],
            &facts()
        ),
        [
            "/ p/.svelte-kit/generated/dev rp req RunWrite(0)",
            "/ p/.svelte-kit/generated/dev/x rp req RunWrite(0)",
            "/ p/.svelte-kit/generated rp req RunWrite(0)",
            "/ p/node_modules rp req RunWrite(0)",
            "/ p/.svelte-kit2 rw req RunWrite(0)",
            "/ p/src rw req RunWrite(0)",
            "/ elsewhere rw req RunWrite(0)",
        ]
    );
    // where a linked `node_modules` leads, in it or holding it, and not a
    // sibling of it
    let mut linked = facts();
    linked.node_modules = NodeModules::Link {
        real: "/shared/node_modules".into(),
        present: ToolDirs::EMPTY,
        checkout: true,
    };
    assert_eq!(
        run_write(
            &[
                ("/shared/node_modules/$app", "/shared/node_modules/$app"),
                ("/shared", "/shared"),
                ("/shared/out", "/shared/out"),
            ],
            &linked
        ),
        [
            "/ shared/node_modules/$app rp req RunWrite(0)",
            "/ shared rp req RunWrite(0)",
            "/ shared/out rw req RunWrite(0)",
        ]
    );
}

/// Both write flags take one rule: `--allow-write=` on, in, or holding a
/// tool directory is written plain as `--allow-run-write=` is, and every
/// tool directory's path counts, made or not, so a closure with no tool
/// that writes, in a project declaring none, still writes a flag on
/// `.svelte-kit` plain; every other flag directory keeps the whole write.
#[test]
fn both_write_flags_write_a_tool_directory_plain() {
    let mut none_writes = flagged(&[
        (
            DirFlag::Write,
            ".svelte-kit/generated/dev",
            "/p/.svelte-kit/generated/dev",
        ),
        (DirFlag::Write, "node_modules", "/p/node_modules"),
        (
            DirFlag::Write,
            ".svelte-kit/output",
            "/p/.svelte-kit/output",
        ),
        (DirFlag::Write, "node_modules/$app", "/p/node_modules/$app"),
        (
            DirFlag::Write,
            "node_modules/.vite-temp",
            "/p/node_modules/.vite-temp",
        ),
        (
            DirFlag::Write,
            "node_modules/.vite",
            "/p/node_modules/.vite",
        ),
        (DirFlag::Write, ".svelte-kit2", "/p/.svelte-kit2"),
        (DirFlag::Write, "src", "/p/src"),
    ]);
    none_writes.programs = vec![ProgramFile::Direct {
        real: "/usr/bin/git".into(),
    }];
    none_writes.spawns = true;
    none_writes.git = true;
    none_writes.run_writes = vec![named_writes(
        "git",
        "git",
        &[(".svelte-kit", "/p/.svelte-kit"), ("out", "/p/out")],
    )];
    let plan = BoxPlan::for_run(&none_writes, &facts()).unwrap();
    assert_eq!(plan.tool_dirs().count(), 0);
    let flagged: Vec<String> = render(&plan)
        .into_iter()
        .filter(|line| line.contains("Flag(Write)") || line.contains("RunWrite"))
        .collect();
    assert_eq!(
        flagged,
        [
            "/ p/.svelte-kit/generated/dev rp req Flag(Write)",
            "/ p/node_modules rp req Flag(Write)",
            "/ p/.svelte-kit/output rp req Flag(Write)",
            "/ p/node_modules/$app rp req Flag(Write)",
            "/ p/node_modules/.vite-temp rp req Flag(Write)",
            "/ p/node_modules/.vite rp req Flag(Write)",
            "/ p/.svelte-kit2 rw req Flag(Write)",
            "/ p/src rw req Flag(Write)",
            "/ p/.svelte-kit rp req RunWrite(0)",
            "/ p/out rw req RunWrite(0)",
        ]
    );
}

/// vitest's own cache is made only where the project declares vitest, not
/// `vite` alone, though `.vite-temp` is.
#[test]
fn vitest_s_cache_is_made_only_for_a_declared_vitest() {
    let plan = BoxPlan::for_run(
        &inputs(false, &["vitest"], &["vite"], vec![node()]),
        &facts(),
    )
    .unwrap();
    let dirs: Vec<ToolDir> = plan.tool_dirs().map(|(dir, _)| dir).collect();
    assert_eq!(dirs, [ToolDir::ViteTemp]);
}

/// Vitest's module cache is in its own tool directory, by absolute path,
/// only where Vitest's box is the table's alone: the run starts vitest,
/// made `node_modules/.vite/vitest` in the project's own `node_modules`, and
/// no `--allow-read=`, `--allow-run-in=`, `--allow-run=` of a program the
/// table doesn't grant, or `--allow-run-write=vitest:` widens its box; a
/// write flag, gro's code's alone, and another program's
/// `--allow-run-write=` leave it.
#[test]
fn vitest_s_module_cache_is_given_only_to_the_table_s_box() {
    let cache = Some(PathBuf::from(
        "/p/node_modules/.vite/vitest/boxed-module-cache",
    ));
    // `test` in a project declaring vitest, with `flags` and `run_writes`
    let test = |flags: &[(DirFlag, &str, &str)], run_writes: Vec<RunWriteGrant>| BoxInputs {
        declared: set(&["vitest"]),
        ..run_writing(false, flags, run_writes)
    };
    let mut linked = facts();
    linked.node_modules = NodeModules::Link {
        real: "/shared/node_modules".into(),
        present: vitest_dirs(),
        checkout: true,
    };
    let python = named_writes("/usr/bin/python3", "python3", &[("../out", "/out")]);
    for (case, inputs, facts, expected) in [
        ("test", test(&[], Vec::new()), facts(), cache.clone()),
        (
            "a write flag",
            test(&[(DirFlag::Write, "../out", "/out")], Vec::new()),
            facts(),
            cache.clone(),
        ),
        (
            "another program's write flag",
            test(&[], vec![python]),
            facts(),
            cache,
        ),
        (
            "vitest's write flag",
            test(&[], vec![vitest_writes(&[("src", "/p/src")])]),
            facts(),
            None,
        ),
        (
            "a read flag",
            test(&[(DirFlag::Read, "../data", "/data")], Vec::new()),
            facts(),
            None,
        ),
        (
            "a run-in flag",
            test(&[(DirFlag::RunIn, "/opt/x", "/opt/x")], Vec::new()),
            facts(),
            None,
        ),
        (
            "a program a flag grants",
            BoxInputs {
                flag_programs: true,
                ..test(&[], Vec::new())
            },
            facts(),
            None,
        ),
        ("a linked node_modules", test(&[], Vec::new()), linked, None),
        (
            "vitest undeclared",
            BoxInputs {
                declared: set(&["vite"]),
                ..test(&[], Vec::new())
            },
            facts(),
            None,
        ),
        (
            "no vitest",
            BoxInputs {
                tools: table_tools(&["svelte-kit"]),
                ..test(&[], Vec::new())
            },
            facts(),
            None,
        ),
        (
            "no program",
            BoxInputs {
                spawns: false,
                programs: Vec::new(),
                ..test(&[], Vec::new())
            },
            facts(),
            None,
        ),
    ] {
        let plan = BoxPlan::for_run(&inputs, &facts).unwrap();
        assert_eq!(plan.vitest_module_cache(), expected, "{case}");
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
    run_in.git = true;
    let mut read = flagged(&[(DirFlag::Read, "~/.asdf", "/h/.asdf")]);
    read.programs = vec![shim.clone()];
    read.spawns = true;
    read.git = true;
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
        linked.node_modules = NodeModules::Link {
            real: real.into(),
            present: ToolDirs::EMPTY,
            checkout: true,
        };
        linked
    };
    for (case, inputs, facts, refusal) in [
        ("-A", unconfined, facts(), Some(PlanRefusal::Unconfined)),
        (
            "write ..",
            flagged(&[(DirFlag::Write, "..", "/")]),
            facts(),
            Some(PlanRefusal::WriteAboveRoot {
                program: None,
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
                program: None,
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
/// note naming where it leads. No tool directory is made there, whatever
/// flag names it: `--allow-write=` naming it, or a directory holding it, is
/// gro's own code's alone, and a program writes there only by a rule
/// `--allow-run-write=` makes for it, which the note names.
#[test]
fn a_symlinked_node_modules_is_followed() {
    let mut linked = facts();
    linked.node_modules = NodeModules::Link {
        real: "/shared/node_modules".into(),
        present: ToolDirs::EMPTY,
        checkout: true,
    };
    let kit = || {
        inputs(
            false,
            &["svelte-kit", "vitest"],
            &["@sveltejs/kit", "vite", "vitest"],
            vec![node()],
        )
    };
    // neither tool's directories are there yet, so each flag names the
    // target itself
    let none_writes = "note: node_modules is a symlink to /shared/node_modules, which the box \
                       follows to read and execute, and no tool writes in it; that checkout's \
                       own commands run what a tool writes there, and this command's tools \
                       write there with: fgro check \
                       '--allow-run-write=svelte-kit:/shared/node_modules' \
                       '--allow-run-write=vitest:/shared/node_modules' --no-lint";
    // no flag: read and execute alone, the project's own tool directories
    // made, SvelteKit's and vitest's own beneath `.svelte-kit`, none in the
    // link's target
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
    assert_eq!(plan.note(check_command).as_deref(), Some(none_writes));
    // a write flag naming it, or a directory holding it: its rule, gro's
    // own code's, written plain, and no tool directory there
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
        assert!(
            rendered
                .iter()
                .any(|line| *line == format!("/ {} rp req Flag(Write)", &real[1..])),
            "{dir}: {rendered:#?}"
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("ToolDir(App)") || line.contains("ToolDir(Vite")),
            "{dir}: {rendered:#?}"
        );
        assert_eq!(plan.tool_dirs().count(), 3, "{dir}: {rendered:#?}");
        assert_eq!(
            plan.note(check_command).as_deref(),
            Some(none_writes),
            "{dir}"
        );
    }
    // `--allow-run-write=` naming it, a directory in it, or one holding it,
    // for one program: that program's rule, no tool directory, and the note
    // names each program writing there, once and sorted, then the flags for
    // the rest, by the program's identity, whatever name the flag gave it
    let rest = "; that checkout's own commands run what a tool writes there, and the rest of \
                this command's tools write there with: fgro check \
                '--allow-run-write=svelte-kit:/shared/node_modules' --no-lint";
    for (case, run_writes, writers) in [
        (
            "its target",
            vec![vitest_writes(&[(
                "/shared/node_modules",
                "/shared/node_modules",
            )])],
            Some(("vitest writes", rest)),
        ),
        (
            "a directory holding it",
            vec![vitest_writes(&[("/shared", "/shared")])],
            Some(("vitest writes", rest)),
        ),
        (
            "directories in it, two programs",
            vec![
                vitest_writes(&[
                    ("/shared/node_modules/.vite", "/shared/node_modules/.vite"),
                    (
                        "/shared/node_modules/.vite-temp",
                        "/shared/node_modules/.vite-temp",
                    ),
                ]),
                named_writes(
                    "node_modules/.bin/svelte-kit",
                    "svelte-kit",
                    &[("/shared/node_modules/$app", "/shared/node_modules/$app")],
                ),
            ],
            Some(("svelte-kit and vitest write", "")),
        ),
        (
            "its target as typed through a path that isn't its real one",
            vec![vitest_writes(&[(
                "../shared/node_modules",
                "/shared/node_modules",
            )])],
            Some(("vitest writes", rest)),
        ),
        (
            "one program by two names, by the first",
            vec![RunWriteGrant {
                dirs: vec![
                    RunWriteDir {
                        named: "node_modules/.bin/vitest".to_owned(),
                        dir: "/shared/node_modules/.vite".to_owned(),
                        real: "/shared/node_modules/.vite".into(),
                    },
                    RunWriteDir {
                        named: "vitest".to_owned(),
                        dir: "/shared/node_modules/.vite-temp".to_owned(),
                        real: "/shared/node_modules/.vite-temp".into(),
                    },
                ],
                ..vitest_writes(&[])
            }],
            Some(("node_modules/.bin/vitest writes", rest)),
        ),
        (
            "a directory elsewhere",
            vec![vitest_writes(&[("out", "/p/out")])],
            None,
        ),
        (
            "a sibling of its target",
            vec![vitest_writes(&[("/shared/out", "/shared/out")])],
            None,
        ),
    ] {
        let mut named = kit();
        named.run_writes = run_writes;
        let plan = BoxPlan::for_run(&named, &linked).unwrap();
        let rendered = render(&plan);
        assert!(
            rendered.iter().any(|line| line.contains("RunWrite(")),
            "{case}: {rendered:#?}"
        );
        assert_eq!(plan.tool_dirs().count(), 3, "{case}: {rendered:#?}");
        let expected = writers.map_or_else(
            || none_writes.to_owned(),
            |(writers, rest)| {
                format!(
                    "note: node_modules is a symlink to /shared/node_modules, which the box \
                     follows to read and execute, and of the tools only {writers} in it, where \
                     --allow-run-write= names{rest}"
                )
            },
        );
        assert_eq!(plan.note(check_command), Some(expected), "{case}");
    }
    // under a command whose closure writes the root, a program's directory
    // where the link leads is outside the project, never the corner's
    let mut gen_named = kit();
    gen_named.writes_root = true;
    gen_named.run_writes = vec![vitest_writes(&[(
        "/shared/node_modules",
        "/shared/node_modules",
    )])];
    let named_plan = BoxPlan::for_run(&gen_named, &linked).unwrap();
    let rendered = render(&named_plan);
    assert!(
        rendered.contains(&"/ shared/node_modules rp req RunWrite(0)".to_owned()),
        "{rendered:#?}"
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line.starts_with("project node_modules")),
        "{rendered:#?}"
    );
    assert_eq!(
        plan.node_modules_link(),
        Some(std::path::Path::new("/shared/node_modules"))
    );
    // read alone for a run that starts nothing, as its note says
    let plan = BoxPlan::for_run(&inputs(true, &[], &[], Vec::new()), &linked).unwrap();
    assert_eq!(
        render(&plan)[1],
        "real /shared/node_modules r req NodeModules"
    );
    assert_eq!(
        plan.note(check_command).as_deref(),
        Some(
            "note: node_modules is a symlink to /shared/node_modules, which the box follows to \
             read"
        )
    );
    // and for one whose realm has a temporary directory but starts nothing
    let mut temp_only = inputs(true, &[], &[], Vec::new());
    temp_only.temp = true;
    let plan = BoxPlan::for_run(&temp_only, &linked).unwrap();
    assert_eq!(
        plan.note(check_command).as_deref(),
        Some(
            "note: node_modules is a symlink to /shared/node_modules, which the box follows to \
             read"
        )
    );
}

/// Where a linked `node_modules` leads into a checkout, the note names the
/// command again with a flag for each of the command's tools that writes a
/// tool directory there the declared dependencies call for, quoted for a
/// shell: its directories where all of them are there, else the target
/// itself; a target that's no checkout's, one a shell or the flag would
/// read otherwise, a command with no such tool, and a command line with no
/// command word get the flag's shape alone.
#[test]
fn a_linked_node_modules_note_names_each_tool_s_flag() {
    let note_in = |target: &str,
                   present: ToolDirs,
                   checkout: bool,
                   declared: &[&str],
                   tools: &[&str],
                   command: &dyn Fn(&[String]) -> Option<String>| {
        let mut linked = facts();
        linked.node_modules = NodeModules::Link {
            real: target.into(),
            present,
            checkout,
        };
        let inputs = inputs(false, tools, declared, vec![node()]);
        BoxPlan::for_run(&inputs, &linked)
            .unwrap()
            .note(command)
            .unwrap()
    };
    let note = |target: &str, present: ToolDirs, declared: &[&str], tools: &[&str]| {
        note_in(target, present, true, declared, tools, &check_command)
    };
    let all = [ToolDir::App, ToolDir::ViteTemp, ToolDir::Vitest]
        .into_iter()
        .collect::<ToolDirs>();
    let declared = ["@sveltejs/kit", "vite", "vitest"];
    let tools = ["svelte-check", "svelte-kit", "vitest"];
    let head = |target: &str| {
        format!(
            "note: node_modules is a symlink to {target}, which the box follows to read and \
             execute, and no tool writes in it"
        )
    };
    let with = "; that checkout's own commands run what a tool writes there, and this command's \
                tools write there with: fgro check ";
    let t = "/w/main/node_modules";
    assert_eq!(
        note(t, all, &declared, &tools),
        format!(
            "{}{with}'--allow-run-write=svelte-check:{t}/.vite-temp' \
             '--allow-run-write=svelte-kit:{t}/$app,{t}/.vite-temp' \
             '--allow-run-write=vitest:{t}/.vite-temp,{t}/.vite/vitest' --no-lint",
            head(t)
        )
    );
    // one of a tool's directories missing: the target, which exists
    let app = std::iter::once(ToolDir::App).collect::<ToolDirs>();
    assert_eq!(
        note(t, app, &declared, &tools),
        format!(
            "{}{with}'--allow-run-write=svelte-check:{t}' '--allow-run-write=svelte-kit:{t}' \
             '--allow-run-write=vitest:{t}' --no-lint",
            head(t)
        )
    );
    // only what the declared dependencies call for
    assert_eq!(
        note(t, all, &["@sveltejs/kit"], &tools),
        format!(
            "{}{with}'--allow-run-write=svelte-kit:{t}/$app' --no-lint",
            head(t)
        )
    );
    let shape = ": --allow-run-write=<program>:<dirs> naming where it leads lets that tool write \
                 there";
    for target in [
        "/w/it's/node_modules",
        "/w/a,b/node_modules",
        "/w/a\\b/node_modules",
        "/w/caf\u{e9}/node_modules",
        "/w/a\u{200b}b/node_modules",
        "/w/a\tb/node_modules",
    ] {
        assert_eq!(
            note(target, all, &declared, &tools),
            format!("{}{shape}", head(target)),
            "{target:?}"
        );
    }
    assert_eq!(note(t, all, &declared, &[]), format!("{}{shape}", head(t)));
    // no `package.json` beside it, like a global npm prefix's
    // `lib/node_modules`: no checkout, so no flags to paste
    assert_eq!(
        note_in(t, all, false, &declared, &tools, &check_command),
        format!("{}{shape}", head(t))
    );
    // no command word to put the flags after
    assert_eq!(
        note_in(t, all, true, &declared, &tools, &|_| None),
        format!("{}{shape}", head(t))
    );
}

/// A command line `check --no-lint` rendered with `flags`, the grant flags
/// after its command word, as the CLI renders the whole command.
#[expect(clippy::unnecessary_wraps, reason = "the shape `BoxPlan::note` takes")]
fn check_command(flags: &[String]) -> Option<String> {
    Some(format!("fgro check {} --no-lint", flags.join(" ")))
}

/// The program `program`, named `named` on the command line, with the
/// directories `dirs` lists for it, each as written and its real path.
fn named_writes(program: &str, named: &str, dirs: &[(&str, &str)]) -> RunWriteGrant {
    RunWriteGrant {
        program: program.to_owned(),
        dirs: dirs
            .iter()
            .map(|&(dir, real)| RunWriteDir {
                named: named.to_owned(),
                dir: dir.to_owned(),
                real: real.into(),
            })
            .collect(),
    }
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

/// The rule that holds the temporary directory a spawning run makes its
/// private directory in: a flag on its parent, on the directory itself, on
/// `/`, a read or run-in flag, never a write flag, whose directory no
/// program takes, and the project's own read, compared by components, so `/tmpx`
/// holds nothing of `/tmp`, nor `/tmp/a` of `/tmp`; the system's rules hold
/// none of `/tmp`'s, and the private directory's own rule is never one.
#[test]
fn a_rule_holding_the_temporary_directory_is_found() {
    let spawning = |flags: &[(DirFlag, &str, &str)]| BoxInputs {
        spawns: true,
        git: true,
        programs: vec![node()],
        ..flagged(flags)
    };
    let holding = |flags: &[(DirFlag, &str, &str)], dir: &str| {
        BoxPlan::for_run(&spawning(flags), &facts())
            .unwrap()
            .holding(std::path::Path::new(dir))
    };
    let flag = |flag| Some((PathBuf::from("/tmp"), RuleSource::Flag(flag)));
    assert_eq!(
        holding(&[(DirFlag::Read, "/tmp", "/tmp")], "/tmp/base"),
        flag(DirFlag::Read)
    );
    assert_eq!(
        holding(&[(DirFlag::Read, "/tmp", "/tmp")], "/tmp"),
        flag(DirFlag::Read)
    );
    assert_eq!(
        holding(&[(DirFlag::RunIn, "/tmp", "/tmp")], "/tmp"),
        flag(DirFlag::RunIn)
    );
    // a write flag's directory is gro's own code's, which no program takes
    assert_eq!(holding(&[(DirFlag::Write, "/tmp", "/tmp")], "/tmp"), None);
    assert_eq!(
        holding(&[(DirFlag::Read, "/", "/")], "/tmp"),
        Some((PathBuf::from("/"), RuleSource::Flag(DirFlag::Read)))
    );
    assert_eq!(holding(&[(DirFlag::Read, "/tmpx", "/tmpx")], "/tmp"), None);
    assert_eq!(holding(&[(DirFlag::Read, "/tmp", "/tmp")], "/tmpx"), None);
    assert_eq!(
        holding(&[(DirFlag::Read, "/tmp/a", "/tmp/a")], "/tmp"),
        None
    );
    assert_eq!(holding(&[], "/tmp"), None);
    assert_eq!(
        holding(&[], "/p/src"),
        Some((PathBuf::from("/p/."), RuleSource::Project))
    );
    // `--allow-temp`'s private directory is the run's alone
    let temp = BoxInputs {
        temp: true,
        ..spawning(&[])
    };
    let plan = BoxPlan::for_run(&temp, &facts()).unwrap();
    assert!(plan.private_temp());
    assert_eq!(plan.holding(std::path::Path::new("/tmp")), None);
}

/// The program `node_modules/.bin/vitest`, a tool of the table, with the
/// directories `dirs` lists for it, each as written and its real path.
fn vitest_writes(dirs: &[(&str, &str)]) -> RunWriteGrant {
    RunWriteGrant {
        program: "node_modules/.bin/vitest".to_owned(),
        dirs: dirs
            .iter()
            .map(|&(dir, real)| RunWriteDir {
                named: "vitest".to_owned(),
                dir: dir.to_owned(),
                real: real.into(),
            })
            .collect(),
    }
}

/// A run of `test`, or of a command whose closure writes the root when
/// `writes`, with the write flags `flags` and `--allow-run-write=` naming
/// `run_writes`.
fn run_writing(
    writes: bool,
    flags: &[(DirFlag, &str, &str)],
    run_writes: Vec<RunWriteGrant>,
) -> BoxInputs {
    BoxInputs {
        writes_root: writes,
        spawns: true,
        git: true,
        programs: vec![node()],
        run_writes,
        tools: table_tools(&["vitest"]),
        ..flagged(flags)
    }
}

/// The tool directories the carried table's vitest writes.
fn vitest_dirs() -> ToolDirs {
    table_tools(&["vitest"])["node_modules/.bin/vitest"].tool_dirs
}

/// `--allow-run-write=` widens the run's box alone: each directory read
/// and written beneath `/`, a rule of its own beside any other source's on
/// the same path, an excluded entry honored where gro's code doesn't write
/// the root, and the root the per-entry set; its program and spelling are
/// the plan's to name, by the rule's index.
#[test]
fn a_run_write_flag_widens_the_box() {
    let plan = |inputs: &BoxInputs| BoxPlan::for_run(inputs, &facts()).unwrap();
    let added = |plan: &BoxPlan| -> Vec<String> {
        render(plan)
            .into_iter()
            .filter(|line| {
                line.contains("RunWrite") || line.contains("Flag(") || line.contains("Entry")
            })
            .collect()
    };
    let one = plan(&run_writing(
        false,
        &[],
        vec![vitest_writes(&[("src/x", "/p/src/x"), ("../out", "/out")])],
    ));
    assert_eq!(
        added(&one),
        ["/ p/src/x rw req RunWrite(0)", "/ out rw req RunWrite(0)"]
    );
    assert_eq!(
        one.run_write(0),
        Some(&vitest_writes(&[("src/x", "/p/src/x"), ("../out", "/out")]))
    );
    assert_eq!(one.run_write(1), None);
    // the same directory under a write flag, and one an entry's rule
    // names: each its own rule
    let both = plan(&run_writing(
        true,
        &[(DirFlag::Write, "out", "/p/src")],
        vec![vitest_writes(&[("src", "/p/src")])],
    ));
    assert_eq!(
        added(&both),
        [
            "project CLAUDE.md w req Entry",
            "project package.json w req Entry",
            "project src w req Entry",
            "/ p/src rw req Flag(Write)",
            "/ p/src rw req RunWrite(0)",
        ]
    );
    // an excluded entry, under a command that doesn't write the root
    let git = RunWriteGrant {
        program: "git".to_owned(),
        dirs: vec![RunWriteDir {
            named: "git".to_owned(),
            dir: ".git".to_owned(),
            real: "/p/.git".into(),
        }],
    };
    assert_eq!(
        added(&plan(&run_writing(
            false,
            &[],
            vec![vitest_writes(&[]), git]
        ))),
        ["/ p/.git rw req RunWrite(1)"]
    );
    // the root, by any spelling, is the per-entry set, planned as a write
    // on the root is
    for spelling in [".", "./", "../p"] {
        let rooted = plan(&run_writing(
            false,
            &[],
            vec![vitest_writes(&[(spelling, "/p")])],
        ));
        assert_eq!(
            added(&rooted),
            [
                "project CLAUDE.md w req Entry",
                "project package.json w req Entry",
                "project src w req Entry",
            ],
            "{spelling}"
        );
        assert!(render(&rooted).contains(&"skip /p/AGENTS.md Symlink".to_owned()));
    }
}

/// A directory `--allow-run-write=` names that holds the root is refused,
/// naming the flag as written; where gro's own code writes the whole
/// project, by the closure or a write flag naming the root, one under a
/// dot-entry or `node_modules` by its real path is refused unless a write
/// flag names it or a directory holding it short of the root.
#[test]
fn the_plan_refuses_a_run_write_flag_gro_s_code_would_write_through() {
    let refused = |inputs: &BoxInputs| BoxPlan::for_run(inputs, &facts()).err();
    assert_eq!(
        refused(&run_writing(
            false,
            &[],
            vec![vitest_writes(&[("..", "/")])]
        )),
        Some(PlanRefusal::WriteAboveRoot {
            program: Some("vitest".to_owned()),
            dir: "..".to_owned(),
            real: "/".into(),
        })
    );
    let above = refused(&run_writing(
        false,
        &[],
        vec![vitest_writes(&[("../..", "/")])],
    ))
    .unwrap();
    assert_eq!(
        above.to_string(),
        "--allow-run-write=vitest:../.. names /, which holds the project, and no kernel rule \
         beneath it can leave the project's .git and node_modules out: name directories inside \
         the project or beside it"
    );
    let corner = |dir: &str| PlanRefusal::RealmWritesProgramDir {
        program: "vitest".to_owned(),
        dir: dir.to_owned(),
        cause: RootWriter::Closure,
    };
    for (dir, real) in [
        (".git", "/p/.git"),
        (".git/hooks", "/p/.git/hooks"),
        ("node_modules", "/p/node_modules"),
        ("node_modules/.cache", "/p/node_modules/.cache"),
        (".svelte-kit", "/p/.svelte-kit"),
        // a top-level symlink to `.git`, by its real path
        ("out", "/p/.git"),
    ] {
        assert_eq!(
            refused(&run_writing(true, &[], vec![vitest_writes(&[(dir, real)])])),
            Some(corner(dir)),
            "{dir}"
        );
        // with no write on the root by gro's code it's honored
        assert_eq!(
            refused(&run_writing(
                false,
                &[],
                vec![vitest_writes(&[(dir, real)])]
            )),
            None,
            "{dir}"
        );
        // a write flag naming it, or a directory holding it short of the
        // root, puts both on the command line
        assert_eq!(
            refused(&run_writing(
                true,
                &[(DirFlag::Write, dir, real)],
                vec![vitest_writes(&[(dir, real)])]
            )),
            None,
            "{dir}"
        );
    }
    assert_eq!(
        refused(&run_writing(
            true,
            &[(DirFlag::Write, "node_modules", "/p/node_modules")],
            vec![vitest_writes(&[(
                "node_modules/.cache",
                "/p/node_modules/.cache"
            )])]
        )),
        None
    );
    // the root itself, an ordinary entry, and a directory outside the
    // project aren't the corner's
    for (dir, real) in [
        (".", "/p"),
        ("src", "/p/src"),
        ("../.x", "/.x"),
        ("lib", "/o/node_modules/x"),
    ] {
        assert_eq!(
            refused(&run_writing(true, &[], vec![vitest_writes(&[(dir, real)])])),
            None,
            "{dir}"
        );
    }
    // a write flag naming the root makes gro's code write it, under any
    // command, and is named as written
    assert_eq!(
        refused(&run_writing(
            false,
            &[(DirFlag::Write, "./", "/p")],
            vec![vitest_writes(&[(".git", "/p/.git")])]
        )),
        Some(PlanRefusal::RealmWritesProgramDir {
            program: "vitest".to_owned(),
            dir: ".git".to_owned(),
            cause: RootWriter::Flag("./".to_owned()),
        })
    );
    // the message, with and without the command's name
    let refusal = corner(".git");
    assert_eq!(
        refusal.message(Some("fgro gen"), GRO_CODE),
        "--allow-run-write=vitest:.git would let gro's own code write .git as well, since fgro \
         gen writes the project and only the kernel box keeps it out of .git, node_modules, and \
         the dot-entries: add --allow-write=.git to grant it to both"
    );
    assert!(
        refusal
            .to_string()
            .contains("since the command writes the project")
    );
}

/// A rule `--allow-run-write=` makes on the OS's temporary directory, or
/// one holding it, is found holding it, since its program takes it; the
/// program boxes of the run are one writing nothing and each program's own.
#[test]
fn a_run_write_flag_holding_the_temporary_directory_is_found() {
    let plan = BoxPlan::for_run(
        &run_writing(false, &[], vec![vitest_writes(&[("/tmp", "/tmp")])]),
        &facts(),
    )
    .unwrap();
    assert_eq!(
        plan.holding(std::path::Path::new("/tmp/base")),
        Some((PathBuf::from("/tmp"), RuleSource::RunWrite(0)))
    );
    assert_eq!(
        plan.program_writes(),
        [
            ProgramWrites::NONE,
            ProgramWrites {
                tool_dirs: vitest_dirs(),
                root: false,
                run_write: Some(0),
            }
        ]
    );
    // one naming the root writes its entries; a run that starts nothing
    // has no program to write anything
    let rooted = BoxPlan::for_run(
        &run_writing(false, &[], vec![vitest_writes(&[(".", "/p")])]),
        &facts(),
    )
    .unwrap();
    assert_eq!(
        rooted.program_writes()[1],
        ProgramWrites {
            tool_dirs: vitest_dirs(),
            root: true,
            run_write: Some(0),
        }
    );
    let still = BoxInputs {
        spawns: false,
        programs: Vec::new(),
        ..run_writing(false, &[], vec![vitest_writes(&[("/tmp", "/tmp")])])
    };
    assert_eq!(
        BoxPlan::for_run(&still, &facts()).unwrap().program_writes(),
        []
    );
}

/// A spawning run that may not start git reads none of git's files outside
/// the project and isn't refused when they can't be read; type markers
/// above the root are read, file-only, by their real paths, after the
/// link closure.
#[test]
fn git_s_files_hang_on_git_and_markers_are_read_file_only() {
    let mut cargo = inputs(
        false,
        &[],
        &[],
        vec![ProgramFile::Direct {
            real: "/usr/bin/cargo".into(),
        }],
    );
    cargo.git = false;
    cargo.markers = vec!["/m/package.json".into()];
    let plan = BoxPlan::for_run(&cargo, &facts()).unwrap();
    let lines = render(&plan);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("GitHome") || line.contains("GitDir")),
        "{lines:#?}"
    );
    let marker = lines
        .iter()
        .position(|line| line == "real /m/package.json r opt Marker")
        .unwrap_or_else(|| panic!("{lines:#?}"));
    let linked = lines
        .iter()
        .position(|line| line.starts_with("real /elsewhere/pkg"))
        .unwrap();
    assert_eq!(marker, linked + 1, "{lines:#?}");
    let faulty = BoxFacts {
        git_fault: Some(crate::confine::git::GitFault {
            path: "/h/.gitconfig".into(),
            why: "unreadable".to_owned(),
        }),
        ..facts()
    };
    assert!(BoxPlan::for_run(&cargo, &faulty).is_ok());
    cargo.git = true;
    assert!(matches!(
        BoxPlan::for_run(&cargo, &faulty),
        Err(PlanRefusal::Git(_))
    ));
}

/// The root of an entry in no project is the run's loader's: its rule
/// reaches no program's box, so nothing a spawning run starts reads it,
/// unless a read or run-in flag names it, or `--allow-run-write=` names it
/// for that program, which reads it then; a project's root reaches every
/// program's box.
#[test]
fn an_unbounded_root_reaches_no_program_unless_a_flag_names_it() {
    let loose = |flags: &[(DirFlag, &str, &str)]| BoxInputs {
        unbounded: true,
        spawns: true,
        programs: vec![ProgramFile::Direct {
            real: "/usr/bin/cat".into(),
        }],
        ..flagged(flags)
    };
    let holding = |inputs: &BoxInputs| {
        BoxPlan::for_run(inputs, &facts())
            .unwrap()
            .holding(std::path::Path::new("/p/src"))
    };
    let plan = BoxPlan::for_run(&loose(&[]), &facts()).unwrap();
    assert!(
        render(&plan).contains(&"project . r req UnboundedRoot".to_owned()),
        "{:#?}",
        render(&plan)
    );
    assert_eq!(holding(&loose(&[])), None);
    for flag in [DirFlag::Read, DirFlag::RunIn] {
        assert_eq!(
            holding(&loose(&[(flag, ".", "/p")])),
            Some((PathBuf::from("/p"), RuleSource::Flag(flag)))
        );
    }
    // a write flag's directory is gro's own code's
    assert_eq!(holding(&loose(&[(DirFlag::Write, ".", "/p")])), None);
    let writes_root = ProgramWrites {
        root: true,
        run_write: Some(0),
        ..ProgramWrites::NONE
    };
    for (source, writes, granted) in [
        (RuleSource::UnboundedRoot, ProgramWrites::NONE, None),
        (RuleSource::UnboundedRoot, writes_root, Some(Rights::READ)),
        (RuleSource::Project, ProgramWrites::NONE, Some(Rights::READ)),
    ] {
        assert_eq!(
            crate::confine::program::program_rights(source, Rights::READ, writes),
            granted,
            "{source:?} {writes:?}"
        );
    }
    let bounded = BoxInputs {
        unbounded: false,
        ..loose(&[])
    };
    assert_eq!(
        holding(&bounded),
        Some((PathBuf::from("/p/."), RuleSource::Project))
    );
}
