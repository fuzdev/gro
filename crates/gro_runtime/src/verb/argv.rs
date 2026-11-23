//! The command's args as Node's Gro reads them, as far as the closure needs
//! them: which keys a command line sets, in what order, and to `true`,
//! `false`, or anything else, and its positionals.
//!
//! [`parse_argv`] is `fuz_util`'s `argv_parse` (`@fuzdev/fuz_util/args.ts`),
//! the parse every Gro command line and forwarded section goes through, and
//! [`ArgvKeys::flag`] and [`ArgvKeys::fits`] are `args_parse`'s reading of
//! them against a builtin's `Args`, as the grants table records them
//! ([`BuiltinArgs`]): a boolean's value given or its default, synced with
//! its `no-` pair where both are booleans, and whether Node refuses the args
//! for a key the schema doesn't hold or a value given a boolean. A value
//! `argv_parse` coerces (`--x=1`, `--x v`) is never a boolean, so it's
//! [`ArgValue::Other`], and a repeated valued flag an array, `Other` too.
//!
//! The positionals are `argv_parse`'s `_` key, so a flag keyed `_` reaches
//! them: a value given it (`--_=check`, `--_ check`) joins them in place,
//! and a boolean (`--_`, `--no-_`, `-_`) replaces them, after which Node
//! names no builtin, refusing them or reading a list led by `true` or
//! `false` ([`ParsedArgv::replaced`]). A coerced number given `_` is kept
//! as its text, which names no builtin, `task`, or task path either way.
//! The first positional names the command, and any after it stay the
//! command's `_` arg ([`ParsedArgv::command_keys`]), which `fits` reads
//! as it reads any key.
//!
//! Two departures, neither reaching a builtin's boolean arg: JS lists an
//! integer-like key (`--5`) before every other, where here it keeps its place,
//! and a short flag's keys are its characters, where JS takes UTF-16 code
//! units.

use super::table::BuiltinArgs;

/// A key's value, as far as the closure reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgValue {
    /// `true`: a boolean flag, `--x` or a short `-x`.
    True,
    /// `false`: a negated flag, `--no-x`.
    False,
    /// A string, a number, or an array: a flag given a value, or given one
    /// more than once.
    Other,
}

/// The keys a command line sets, each once, in the order of their last
/// appearance, as `argv_parse` orders them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArgvKeys(Vec<(String, ArgValue)>);

impl ArgvKeys {
    /// The value `key` holds, and its place.
    fn get(&self, key: &str) -> Option<(usize, ArgValue)> {
        self.0
            .iter()
            .position(|(k, _)| k == key)
            .map(|index| (index, self.0[index].1))
    }

    /// Sets `key` to `value` as its last key, `argv_parse`'s `set_last`.
    fn set_last(&mut self, key: &str, value: ArgValue) {
        self.0.retain(|(k, _)| k != key);
        self.0.push((key.to_owned(), value));
    }

    /// Sets `key` to a value, an array when it has one already, as its last
    /// key, `argv_parse`'s `set_arg`.
    fn set_arg(&mut self, key: &str) {
        self.set_last(key, ArgValue::Other);
    }

    /// These keys with `forwarded`'s merged over them, Gro's
    /// `merge_forwarded_args`: each forwarded key after every key of these,
    /// by spelling.
    pub fn merge(&self, forwarded: &Self) -> Self {
        let mut merged: Vec<(String, ArgValue)> = self
            .0
            .iter()
            .filter(|(key, _)| forwarded.get(key).is_none())
            .cloned()
            .collect();
        merged.extend(forwarded.0.iter().cloned());
        Self(merged)
    }

    /// The boolean arg `key` as `args_parse` reads it against `args`, the
    /// builtin's `Args` (`None` for one with none, whose args Node passes
    /// unparsed): its value given, or its default when none is (`false`
    /// with no `Args`), synced with `no-<key>` when `args` holds both as
    /// booleans, the later given of the two winning and `no-<key>` setting
    /// the opposite of its value. A value given it is no `true`.
    pub fn flag(&self, key: &str, args: Option<&BuiltinArgs>) -> bool {
        let dual = format!("no-{key}");
        let default = args.and_then(|args| args.boolean_default(key));
        let synced =
            default.is_some() && args.is_some_and(|args| args.boolean_default(&dual).is_some());
        let negated = synced.then(|| self.get(&dual)).flatten();
        match (self.get(key), negated) {
            (None, None) => default.unwrap_or(false),
            (Some((_, value)), None) => value == ArgValue::True,
            (Some((index, value)), Some((negated_index, _))) if index > negated_index => {
                value == ArgValue::True
            }
            (_, Some((_, negated))) => negated == ArgValue::False,
        }
    }

    /// Whether these keys pass the two checks of `args_parse` the table
    /// records enough to make against `args`, the builtin's `Args`: each is a
    /// key it holds, and none it holds as a boolean is given a value. A
    /// non-boolean arg's type isn't checked, so keys that fit can still be
    /// refused by `Args`. With no `Args`, Node passes any.
    pub fn fits(&self, args: Option<&BuiltinArgs>) -> bool {
        args.is_none_or(|args| {
            self.0.iter().all(|(key, value)| {
                args.holds(key)
                    && (args.boolean_default(key).is_none() || *value != ArgValue::Other)
            })
        })
    }
}

/// The key that holds `argv_parse`'s positionals.
const POSITIONALS_KEY: &str = "_";

/// A command line or a forwarded section, parsed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedArgv {
    /// The keys it sets, `_` never among them.
    pub keys: ArgvKeys,
    /// Its positionals, as written, values given `_` among them in place.
    pub positionals: Vec<String>,
    /// Whether a boolean given `_` replaced the positionals, so Node names
    /// no builtin from them; those after it aren't kept.
    pub replaced: bool,
}

impl ParsedArgv {
    /// Gives `key` the value `value`, `argv_parse`'s `set_arg`: a value
    /// given `_` joins the positionals.
    fn set_arg(&mut self, key: &str, value: &str) {
        if key == POSITIONALS_KEY {
            self.push_positional(value);
        } else {
            self.keys.set_arg(key);
        }
    }

    /// Sets `key` to a boolean, `argv_parse`'s `set_last`: a boolean given
    /// `_` replaces the positionals.
    fn set_last(&mut self, key: &str, value: ArgValue) {
        if key == POSITIONALS_KEY {
            self.replaced = true;
        } else {
            self.keys.set_last(key, value);
        }
    }

    /// Adds a positional, unless they were replaced, where `argv_parse`
    /// pushes it onto the array the `_` key no longer holds.
    fn push_positional(&mut self, positional: &str) {
        if !self.replaced {
            self.positionals.push(positional.to_owned());
        }
    }

    /// The keys of the command its first positional names, as Gro's
    /// `to_task_args` and `to_forwarded_args_by_command` leave them once the
    /// name is taken off: `_` among them, first, as `argv_parse` sets it,
    /// when positionals remain after the name, which the builtin's `Args`
    /// must then hold, and not when none do.
    pub fn command_keys(&self) -> ArgvKeys {
        let mut keys = self.keys.clone();
        if self.positionals.len() > 1 {
            keys.0
                .insert(0, (POSITIONALS_KEY.to_owned(), ArgValue::Other));
        }
        keys
    }
}

/// Parses `argv` as `argv_parse` does: `--x` is `true` and `--no-x` sets
/// `x` `false`, `--x=v` and `--x v` give `x` a value when `v` doesn't start
/// with `-`, a short flag group `-abc` sets each, its last taking a value
/// as a long flag does, `-` alone is skipped, and everything after `--` is
/// positional.
pub fn parse_argv(argv: &[String]) -> ParsedArgv {
    let mut parsed = ParsedArgv::default();
    let mut args = argv.iter().peekable();
    while let Some(arg) = args.next() {
        // the next arg when it's a value: it exists and doesn't start with `-`
        let mut next_value = || args.next_if(|next| !next.starts_with('-'));
        if arg == "--" {
            for positional in args {
                parsed.push_positional(positional);
            }
            break;
        }
        if let Some(rest) = arg.strip_prefix("--") {
            if let Some((key, value)) = rest.split_once('=') {
                parsed.set_arg(key, value);
            } else if let Some(key) = rest.strip_prefix("no-") {
                parsed.set_last(key, ArgValue::False);
            } else if let Some(value) = next_value() {
                parsed.set_arg(rest, value);
            } else {
                parsed.set_last(rest, ArgValue::True);
            }
        } else if arg == "-" {
        } else if let Some(chars) = arg.strip_prefix('-') {
            // `-abc=v` sets `a` and `b` and gives `c` the value; `-abc v` the same
            let (group, value) = match chars.split_once('=') {
                Some((group, value)) => (group, Some(value)),
                None => (chars, None),
            };
            let mut group = group.chars().map(String::from);
            let last = group.next_back();
            for key in group {
                parsed.set_last(&key, ArgValue::True);
            }
            if let Some(last) = last {
                if let Some(value) = value.or_else(|| next_value().map(String::as_str)) {
                    parsed.set_arg(&last, value);
                } else {
                    parsed.set_last(&last, ArgValue::True);
                }
            }
        } else {
            parsed.push_positional(arg);
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::{ArgValue, ArgvKeys, ParsedArgv, parse_argv};
    use crate::verb::table::BuiltinArgs;

    /// `Args` holding each of `booleans` with its default, and `others`.
    fn args(booleans: &[(&str, bool)], others: &[&str]) -> BuiltinArgs {
        BuiltinArgs(
            booleans
                .iter()
                .map(|&(key, default)| (key.to_owned(), Some(default)))
                .chain(others.iter().map(|&key| (key.to_owned(), None)))
                .collect(),
        )
    }

    fn parse(argv: &[&str]) -> ParsedArgv {
        parse_argv(&argv.iter().map(|&arg| arg.to_owned()).collect::<Vec<_>>())
    }

    fn keys(keys: &[(&str, ArgValue)]) -> ArgvKeys {
        ArgvKeys(
            keys.iter()
                .map(|&(key, value)| (key.to_owned(), value))
                .collect(),
        )
    }

    #[test]
    fn argv_is_parsed_as_argv_parse_parses_it() {
        use ArgValue::{False, Other, True};
        let parsed = parse(&[
            "check", "--a", "--b=1", "--no-c", "--d", "v", "-", "-ef", "-g=x", "-hi", "w", "--j",
            "-k", "--", "--l", "m",
        ]);
        assert_eq!(
            parsed.keys,
            keys(&[
                ("a", True),
                ("b", Other),
                ("c", False),
                ("d", Other),
                ("e", True),
                ("f", True),
                ("g", Other),
                ("h", True),
                ("i", Other),
                ("j", True),
                ("k", True),
            ])
        );
        assert_eq!(parsed.positionals, ["check", "--l", "m"]);
        // keys move to their last appearance; a repeated value is an array, and
        // a boolean flag after it replaces it
        assert_eq!(
            parse(&["--a", "--b", "--a"]).keys,
            keys(&[("b", True), ("a", True)])
        );
        assert_eq!(
            parse(&["--a=1", "--b", "--a=2"]).keys,
            keys(&[("b", True), ("a", Other)])
        );
        assert_eq!(parse(&["--a=1", "--a"]).keys, keys(&[("a", True)]));
        // `--x=` is the empty string, and `--no-` sets the empty key
        assert_eq!(
            parse(&["--x=", "--no-"]).keys,
            keys(&[("x", Other), ("", False)])
        );
        // a flag before a word takes it as its value
        assert_eq!(parse(&["--x", "check"]).positionals, Vec::<String>::new());
        assert_eq!(parse(&["-x", "check"]).positionals, Vec::<String>::new());
        assert_eq!(parse(&["--x=1", "check"]).positionals, ["check"]);
        assert_eq!(parse(&["--no-x", "check"]).positionals, ["check"]);
        assert_eq!(parse(&["-x=1", "check"]).positionals, ["check"]);
    }

    #[test]
    fn the_positionals_are_the_underscore_key() {
        let positionals = |argv: &[&str]| {
            let parsed = parse(argv);
            assert!(parsed.keys.get("_").is_none(), "{argv:?}");
            (parsed.positionals, parsed.replaced)
        };
        // a value given `_` joins them in place
        let joined = |items: &[&str]| (items.iter().map(|&i| i.to_owned()).collect(), false);
        assert_eq!(positionals(&["--_=check", "--build"]), joined(&["check"]));
        assert_eq!(positionals(&["--_", "check", "x"]), joined(&["check", "x"]));
        assert_eq!(positionals(&["a", "--_=b", "c"]), joined(&["a", "b", "c"]));
        assert_eq!(positionals(&["-_=a", "-x_", "b"]), joined(&["a", "b"]));
        assert_eq!(
            positionals(&["a", "--", "--_", "b"]),
            joined(&["a", "--_", "b"])
        );
        // a boolean given `_` replaces them, keeping none after it
        let replaced = |items: &[&str]| (items.iter().map(|&i| i.to_owned()).collect(), true);
        assert_eq!(positionals(&["a", "--_", "--x", "b"]), replaced(&["a"]));
        assert_eq!(positionals(&["--no-_", "check"]), replaced(&[]));
        assert_eq!(positionals(&["-_", "--", "check"]), replaced(&[]));
        assert_eq!(positionals(&["-_x", "check"]), replaced(&[]));
        assert_eq!(positionals(&["check", "-_"]), replaced(&["check"]));
        assert_eq!(positionals(&["--_", "--_=check"]), replaced(&[]));
    }

    #[test]
    fn positionals_after_the_name_are_the_command_s_underscore_key() {
        use ArgValue::{Other, True};
        let command_keys = |argv: &[&str]| parse(argv).command_keys();
        assert_eq!(command_keys(&["check", "--x"]), keys(&[("x", True)]));
        assert_eq!(command_keys(&[]), keys(&[]));
        // `_` first, as `argv_parse` sets it before any flag
        assert_eq!(
            command_keys(&["--x=1", "check", "a", "--y"]),
            keys(&[("_", Other), ("x", Other), ("y", True)])
        );
        assert_eq!(command_keys(&["--y", "check", "a"]), keys(&[("y", Other)]));
        assert_eq!(
            command_keys(&["check", "--y", "a", "b"]),
            keys(&[("_", Other), ("y", Other)])
        );
        assert_eq!(command_keys(&["check", "--", "--y"]), keys(&[("_", Other)]));
        assert_eq!(command_keys(&["--_=check", "a"]), keys(&[("_", Other)]));
        let gen_args = args(&[("check", false)], &["_"]);
        let check_args = args(&[("check", false)], &[]);
        assert!(command_keys(&["gen", "src"]).fits(Some(&gen_args)));
        assert!(!command_keys(&["check", "src"]).fits(Some(&check_args)));
        assert!(command_keys(&["check", "--check"]).fits(Some(&check_args)));
    }

    #[test]
    fn a_boolean_arg_is_set_as_args_parse_syncs_its_no_pair() {
        let paired = args(&[("build", false), ("no-build", true)], &[]);
        let set = |argv: &[&str]| parse(argv).keys.flag("build", Some(&paired));
        assert!(set(&["--build"]));
        assert!(!set(&[]));
        assert!(!set(&["--no-build"]));
        assert!(!set(&["--build", "--no-build"]));
        assert!(set(&["--no-build", "--build"]));
        // `no-build` given `false` sets `build`
        assert!(set(&["--no-no-build"]));
        assert!(!set(&["--no-build=1"]));
        // a later `no-build` given a value is no boolean's `false`
        assert!(!set(&["--build", "--no-build=x"]));
        // `=` is read before `no-`: this sets the key `no-no-build`
        assert!(set(&["--build", "--no-no-build=x"]));
        assert!(set(&["--no-no-build", "--build"]));
        // a value is never a boolean's `true`
        assert!(!set(&["--build=true"]));
        assert!(!set(&["--build", "true"]));
        assert!(!set(&["--build=1", "--build=2"]));
    }

    #[test]
    fn a_boolean_arg_defaults_and_pairs_as_its_args_declare() {
        let on = args(&[("test", true), ("no-test", false)], &[]);
        let flag = |argv: &[&str], args: Option<&BuiltinArgs>| parse(argv).keys.flag("test", args);
        // the default when the command line gives neither
        assert!(flag(&[], Some(&on)));
        assert!(!flag(&["--no-test"], Some(&on)));
        assert!(!flag(&["--no-no-test=x", "--no-test"], Some(&on)));
        assert!(flag(&["--no-test", "--test"], Some(&on)));
        // `no-test` given `false` turns it on, and given a value, which `Args` refuses, never
        assert!(!flag(&["--no-test=1"], Some(&on)));
        assert!(flag(&["--no-no-test"], Some(&on)));
        // no pair without both booleans: `no-test` is then an arg of its own
        let unpaired = args(&[("test", true)], &["no-test"]);
        assert!(flag(&["--no-no-test"], Some(&unpaired)));
        assert!(flag(&["--no-test=1"], Some(&unpaired)));
        assert!(!flag(&["--no-test"], Some(&unpaired)));
        // with no `Args`, `false` unless given `true`, and no pair
        assert!(!flag(&[], None));
        assert!(flag(&["--test"], None));
        assert!(flag(&["--test", "--no-no-test"], None));
        assert!(!flag(&["--no-no-test"], None));
    }

    #[test]
    fn args_fit_as_args_parse_takes_them() {
        let gen_args = args(&[("check", false)], &["_", "root_dirs"]);
        let fits = |argv: &[&str], args: Option<&BuiltinArgs>| parse(argv).keys.fits(args);
        assert!(fits(&["--check", "--no-check"], Some(&gen_args)));
        assert!(fits(
            &["--root_dirs", "a", "--root_dirs=b"],
            Some(&gen_args)
        ));
        // an unknown key, a boolean's `no-` key it doesn't hold, a value given a boolean
        assert!(!fits(&["--no-check", "--x"], Some(&gen_args)));
        assert!(!fits(&["--no-no-check"], Some(&gen_args)));
        assert!(!fits(&["--check=1"], Some(&gen_args)));
        assert!(!fits(&["--check", "false"], Some(&gen_args)));
        // with no `Args`, any
        assert!(fits(&["--x=1", "--check=1"], None));
    }

    #[test]
    fn forwarded_keys_merge_after_every_direct_one() {
        use ArgValue::{False, True};
        let direct = keys(&[("build", True), ("no-build", True), ("x", True)]);
        let forwarded = keys(&[("build", False)]);
        let merged = direct.merge(&forwarded);
        assert_eq!(
            merged,
            keys(&[("no-build", True), ("x", True), ("build", False)])
        );
        let paired = args(&[("build", false), ("no-build", true)], &[]);
        assert!(!merged.flag("build", Some(&paired)));
        // a forwarded `--build` comes after a direct `--no-build`
        let merged = keys(&[("build", False)]).merge(&keys(&[("build", True)]));
        assert!(merged.flag("build", Some(&paired)));
        let merged = keys(&[("no-build", False)]).merge(&keys(&[("x", True)]));
        assert!(merged.flag("build", Some(&paired)));
    }
}
