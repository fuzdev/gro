use std::collections::BTreeSet;

use super::{
    Failure, Fault, GrantedProgram, PassEnv, ProgramOptions, SpawnNeed, check_forwarding,
    program_env,
};
use crate::{CHILD_BASE_ENV, EnvPattern, EnvPatternFault, EnvVars};

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
        forward: vec![EnvPattern::parse("CARGO_*").unwrap()],
    };
    let asking = |names: &[&str]| {
        let patterns = names
            .iter()
            .map(|name| EnvPattern::parse(name).unwrap())
            .collect();
        check_forwarding(
            "tool",
            &options(PassEnv::Patterns(patterns), None),
            &granted,
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
    assert!(matches!(
        check_forwarding(
            "tool",
            &options(PassEnv::All(BTreeSet::new()), None),
            &granted
        ),
        Err(Failure::NotGranted { needed: None, .. })
    ));
}
