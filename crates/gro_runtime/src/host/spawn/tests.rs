use std::collections::{BTreeMap, BTreeSet};

use super::{
    Failure, Fault, GrantedProgram, PassEnv, ProgramOptions, SpawnNeed, check_forwarding, launch,
    program_env,
};
use crate::{CHILD_BASE_ENV, EnvPattern, EnvPatternFault, EnvVars, PassEnvGrant, ProgramGrant};

/// What the embedder read for children, in order.
fn read(vars: &[(&str, &str)]) -> EnvVars {
    vars.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// A program's options of `pass` and `values`.
fn options(pass: PassEnv, values: Option<&[(&str, &str)]>) -> ProgramOptions {
    ProgramOptions {
        pass,
        values: values.map(|values| {
            values
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect()
        }),
    }
}

/// A child's environment is the base set in the Node module's order,
/// `LC_ALL=C` among it, `PATH` kept to its absolute entries, then what
/// `pass_env` names, a glob each name it covers, then `env`'s values, a
/// later value in an earlier one's place; `'all'` passes everything but
/// what `omit_env` names.
#[test]
fn a_child_s_environment_is_the_base_set_then_pass_env_then_env() {
    let env = read(&[
        ("FORCE_COLOR", "1"),
        ("CARGO_HOME", "/c"),
        ("PATH", "/usr/bin::rel:/bin/"),
        ("HOME", "/h"),
        ("CARGO_TARGET", "/t"),
        ("SECRET", "s"),
        ("TERM", "xterm"),
    ]);
    let pairs = |made: Vec<(String, String)>| -> Vec<String> {
        made.into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect()
    };
    assert_eq!(
        pairs(program_env(
            &env,
            &options(PassEnv::Patterns(Vec::new()), None)
        )),
        [
            "PATH=/usr/bin:/bin/",
            "HOME=/h",
            "TERM=xterm",
            "LC_ALL=C",
            "FORCE_COLOR=1"
        ]
    );
    let patterns = vec![
        EnvPattern::Prefix("CARGO_".to_owned()),
        EnvPattern::Exact("MISSING".to_owned()),
    ];
    assert_eq!(
        pairs(program_env(
            &env,
            &options(
                PassEnv::Patterns(patterns),
                Some(&[("HOME", "/other"), ("X", "")])
            )
        )),
        [
            "PATH=/usr/bin:/bin/",
            "HOME=/other",
            "TERM=xterm",
            "LC_ALL=C",
            "FORCE_COLOR=1",
            "CARGO_HOME=/c",
            "CARGO_TARGET=/t",
            "X=",
        ]
    );
    assert_eq!(
        pairs(program_env(
            &env,
            &options(PassEnv::All(BTreeSet::from(["SECRET".to_owned()])), None)
        )),
        [
            "FORCE_COLOR=1",
            "CARGO_HOME=/c",
            "PATH=/usr/bin:/bin/",
            "HOME=/h",
            "CARGO_TARGET=/t",
            "TERM=xterm",
        ]
    );
}

/// A pattern names one variable or the names its prefix starts: a glob
/// covers no name holding its prefix past the start, no shorter name, and
/// no other case, and holds a pattern only when every name that one
/// covers, it covers too.
#[test]
fn a_pattern_covers_and_holds_only_what_its_prefix_starts() {
    let parse = |text: &str| EnvPattern::parse(text).unwrap();
    let glob = parse("CARGO_*");
    assert_eq!(glob, EnvPattern::Prefix("CARGO_".to_owned()));
    assert!(glob.covers("CARGO_HOME"));
    assert!(glob.covers("CARGO_"));
    for name in [
        "XCARGO_HOME",
        "MY_CARGO_TOKEN",
        "CARGO",
        "cargo_home",
        "CARGOX",
    ] {
        assert!(!glob.covers(name), "{name}");
    }
    let exact = parse("CARGO_HOME");
    assert!(exact.covers("CARGO_HOME"));
    assert!(!exact.covers("CARGO_HOME_X"));
    assert!(!exact.covers("X_CARGO_HOME"));
    assert!(glob.holds(&exact));
    assert!(glob.holds(&parse("CARGO_H*")));
    assert!(glob.holds(&glob));
    for wider in ["CARGO*", "C*", "XCARGO_*", "MY_CARGO_*"] {
        assert!(!glob.holds(&parse(wider)), "{wider}");
    }
    assert!(!exact.holds(&parse("CARGO_HOME*")));
    assert!(!exact.holds(&parse("CARGO_HOM")));
    assert_eq!(EnvPattern::parse("*"), Err(EnvPatternFault::GlobAlone));
    assert_eq!(EnvPattern::parse("A*B"), Err(EnvPatternFault::GlobNotLast));
    assert_eq!(EnvPattern::parse("A**"), Err(EnvPatternFault::GlobNotLast));
}

/// A granted program's `pass_env` may name a variable of the base set
/// exactly, which every child is given, with no grant to forward it; any
/// other name, a glob over the base set's included, is refused for its
/// first pattern the grant doesn't hold, needing that pattern, as on every
/// root, since `--allow-run-env=` widens the program's grant whichever
/// root passed it on.
#[test]
fn a_base_set_name_is_held_and_any_other_needs_its_forwarding() {
    let granted = GrantedProgram {
        pin: Err(Fault::NotFound),
        grant: ProgramGrant::Flag {
            forward: vec![EnvPattern::parse("CARGO_*").unwrap()],
        },
    };
    let asking = |names: &[&str]| {
        let patterns = names
            .iter()
            .map(|name| EnvPattern::parse(name).unwrap())
            .collect();
        check_forwarding(
            "tool",
            &options(PassEnv::Patterns(patterns), None),
            &granted.grant,
        )
    };
    for base in CHILD_BASE_ENV {
        assert!(asking(&[base, "CARGO_HOME"]).is_ok(), "{base}");
    }
    assert!(asking(&CHILD_BASE_ENV).is_ok());
    for refused in ["LC_ALL", "PATHX", "PAT*", "HOME_DIR", "path", "CARGO*"] {
        match asking(&["HOME", refused, "OTHER"]) {
            Err(Failure::NotGranted {
                program,
                needed: Some(SpawnNeed::Forward(needed)),
                ..
            }) => {
                assert_eq!(program, "tool");
                assert_eq!(needed, refused);
            }
            _ => panic!("{refused} isn't refused needing itself"),
        }
    }
    match check_forwarding(
        "tool",
        &options(PassEnv::All(BTreeSet::new()), None),
        &granted.grant,
    ) {
        Err(Failure::NotGranted {
            cause,
            needed: None,
            ..
        }) => assert_eq!(
            cause,
            "tool can't be passed the whole environment: only a run granted everything passes it"
        ),
        _ => panic!("'all' isn't refused a program granted a list"),
    }
}

/// The cause of a refusal that needs no grant, or a panic naming `what`.
fn refused_needing_nothing(result: Result<(), Failure>, what: &str) -> String {
    match result {
        Err(Failure::NotGranted {
            program,
            cause,
            needed: None,
        }) => {
            assert_eq!(program, "tool", "{what}");
            cause
        }
        _ => panic!("{what} isn't refused needing nothing"),
    }
}

/// A program granted the whole environment but what its grant omits may
/// be passed `'all'` with an `omit_env` leaving each of those out, more
/// beside them included, or a list passing none of them, its child given
/// the base set and `LC_ALL=C` as every list's is; anything passing an
/// omitted name, by `'all'`, the base set, a name, or a glob, is refused,
/// naming it, needing nothing, since no flag widens the policy; and its
/// `env` may set only the table's values, under `'all'` too.
#[test]
fn the_whole_environment_is_passed_but_what_the_grant_omits() {
    let all_but = |omit_env: &[&str]| ProgramGrant::Table {
        pass_env: PassEnvGrant::All {
            omit_env: omit_env.iter().map(|&name| name.to_owned()).collect(),
        },
        env: BTreeMap::new(),
    };
    let grant = all_but(&["NODE_ENV", "npm_config_omit"]);
    let all = |omitted: &[&str]| {
        let omitted = omitted.iter().map(|&name| name.to_owned()).collect();
        check_forwarding("tool", &options(PassEnv::All(omitted), None), &grant)
    };
    let listing_for = |grant: &ProgramGrant, names: &[&str]| {
        let patterns = names
            .iter()
            .map(|name| EnvPattern::parse(name).unwrap())
            .collect();
        check_forwarding("tool", &options(PassEnv::Patterns(patterns), None), grant)
    };
    let listing = |names: &[&str]| listing_for(&grant, names);
    let refusal = |variable: &str| {
        format!(
            "tool can't be passed {variable}: its grant passes the whole environment without it"
        )
    };
    assert!(all(&["NODE_ENV", "npm_config_omit"]).is_ok());
    assert!(all(&["npm_config_omit", "SECRET", "NODE_ENV"]).is_ok());
    assert!(listing(&[]).is_ok());
    assert!(listing(&["HOME", "SECRET", "GIT_*", "npm_config_o", "NODE_ENVX"]).is_ok());
    for omitted in [&[][..], &["NODE_ENV"], &["npm_config_omit", "node_env"]] {
        let cause = refused_needing_nothing(all(omitted), &format!("'all' omitting {omitted:?}"));
        assert!(
            cause.starts_with("tool can't be passed ")
                && cause.ends_with(": its grant passes the whole environment without it"),
            "{cause}"
        );
    }
    assert_eq!(
        refused_needing_nothing(all(&["npm_config_omit"]), "'all' passing NODE_ENV"),
        refusal("NODE_ENV")
    );
    for (names, passed) in [
        (&["NODE_ENV"][..], "NODE_ENV"),
        (&["SECRET", "npm_config_*"], "npm_config_omit"),
        (&["npm_*"], "npm_config_omit"),
        (&["NODE_*", "HOME"], "NODE_ENV"),
    ] {
        assert_eq!(
            refused_needing_nothing(listing(names), &format!("{names:?}")),
            refusal(passed)
        );
    }
    // a list's child is given the base set, so a grant omitting one of it
    // refuses every list, the default call's empty one included
    let no_home = all_but(&["HOME"]);
    for names in [&[][..], &["SECRET"]] {
        assert_eq!(
            refused_needing_nothing(listing_for(&no_home, names), &format!("{names:?}")),
            refusal("HOME")
        );
    }
    // as does a grant omitting any one of them, or `LC_ALL`, which every
    // list's child is set to `C`
    for omitted in CHILD_BASE_ENV.into_iter().chain(["LC_ALL"]) {
        assert_eq!(
            refused_needing_nothing(
                listing_for(&all_but(&[omitted]), &[]),
                &format!("a list omitting {omitted}")
            ),
            refusal(omitted)
        );
    }
    let no_omit = all_but(&[]);
    assert!(listing_for(&no_omit, &[]).is_ok());
    assert!(listing_for(&no_omit, &["HOME", "GIT_*"]).is_ok());
    let setting = |values: &[(&str, &str)]| {
        let omitted = BTreeSet::from(["NODE_ENV".to_owned(), "npm_config_omit".to_owned()]);
        check_forwarding(
            "tool",
            &options(PassEnv::All(omitted), Some(values)),
            &grant,
        )
    };
    assert_eq!(
        refused_needing_nothing(
            setting(&[("NODE_ENV", "production")]),
            "'all' setting NODE_ENV"
        ),
        "tool can't be set NODE_ENV to that value: its grant sets only the values it holds"
    );
}

/// A program whose grant fixes its values may be set only those, each to
/// its own value, or fewer, and one that doesn't may be set any; a value
/// refused is named by its variable alone, needing nothing.
#[test]
fn a_program_s_env_values_are_only_those_its_grant_sets() {
    let only = |held: &[(&str, &str)]| ProgramGrant::Table {
        pass_env: PassEnvGrant::Names(Vec::new()),
        env: held
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    };
    let setting = |grant: &ProgramGrant, values: Option<&[(&str, &str)]>| {
        check_forwarding(
            "tool",
            &options(PassEnv::Patterns(Vec::new()), values),
            grant,
        )
    };
    let color = only(&[("FORCE_COLOR", "1")]);
    assert!(setting(&color, None).is_ok());
    assert!(setting(&color, Some(&[])).is_ok());
    assert!(setting(&color, Some(&[("FORCE_COLOR", "1")])).is_ok());
    for (values, variable) in [
        (&[("FORCE_COLOR", "0")][..], "FORCE_COLOR"),
        (
            &[("FORCE_COLOR", "1"), ("LD_PRELOAD", "/x.so")],
            "LD_PRELOAD",
        ),
        (&[("force_color", "1")], "force_color"),
    ] {
        let cause = refused_needing_nothing(setting(&color, Some(values)), &format!("{values:?}"));
        assert_eq!(
            cause,
            format!(
                "tool can't be set {variable} to that value: its grant sets only the values it \
                 holds"
            )
        );
    }
    let none = only(&[]);
    assert!(setting(&none, None).is_ok());
    refused_needing_nothing(
        setting(&none, Some(&[("NODE_OPTIONS", "--require=x")])),
        "a value under a grant that sets none",
    );
    let any = ProgramGrant::Flag {
        forward: Vec::new(),
    };
    assert!(
        setting(
            &any,
            Some(&[("NODE_OPTIONS", "--require=x"), ("FORCE_COLOR", "0")])
        )
        .is_ok()
    );
}

/// A child in a program's box has each temporary variable pointed into its
/// box's directory, over any value it had, `TMP` and `TEMP` only where its
/// environment has them; a child in no box keeps its environment.
#[test]
fn a_boxed_child_s_temporary_variables_name_its_box_s_directory() {
    let pair = |name: &str, value: &str| (name.to_owned(), value.to_owned());
    let env = [
        pair("PATH", "/usr/bin"),
        pair("TMPDIR", "/run"),
        pair("TMP", "/run"),
        pair("XDG_DATA_HOME", "/elsewhere"),
    ];
    assert_eq!(launch::child_env(&env, None), env);
    assert_eq!(
        launch::child_env(&env, Some(std::path::Path::new("/run/program_2"))),
        [
            pair("PATH", "/usr/bin"),
            pair("TMPDIR", "/run/program_2"),
            pair("TMP", "/run/program_2"),
            pair("XDG_DATA_HOME", "/run/program_2/xdg_data"),
            pair("XDG_CACHE_HOME", "/run/program_2/xdg_cache"),
            pair("XDG_STATE_HOME", "/run/program_2/xdg_state"),
        ]
    );
}
