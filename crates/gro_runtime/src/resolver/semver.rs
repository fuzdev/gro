//! The versions and ranges of a `package.json`, as far as Gro's peer
//! dependencies use them.
//!
//! A hand-written subset of npm's semver, which refuses any form it doesn't
//! read, so a range it can't check is never taken as met.
//!
//! A version is strict semver: `MAJOR.MINOR.PATCH`, numbers without leading
//! zeros, an optional prerelease after `-` and build metadata after `+`,
//! which precedence ignores. A range is comparator sets joined by `||`, a
//! version meeting any one set meeting the range, and a set is comparators
//! joined by spaces, each met. A comparator is `>=`, `>`, `<=`, `<`, or `=`
//! before a whole version, a whole version alone (`=`), or `^` before a
//! version whose minor and patch may be left off (`^6`, `^0.2`), read as
//! npm reads it: `^1.2.3` is `>=1.2.3 <2.0.0-0`, `^0.2.3` is
//! `>=0.2.3 <0.3.0-0`, `^0.0.3` is `>=0.0.3 <0.0.4-0`, a missing part 0
//! and the next part up the bound. Refused: `~`, `x` and `*` parts, hyphen
//! ranges, a space after an operator, an empty set, and anything else npm
//! reads loosely.
//!
//! A version with a prerelease meets a set only when some comparator of the
//! set names a prerelease of the same `MAJOR.MINOR.PATCH`, as npm decides
//! with its prereleases left out: `1.3.0-rc.1` meets `>=1.3.0-rc.0` and not
//! `^1.2.0`.

use std::cmp::Ordering;
use std::fmt;

/// A version, strict semver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// `MAJOR.MINOR.PATCH`.
    core: [u64; 3],
    /// The prerelease identifiers, none for a release.
    pre: Vec<Identifier>,
}

/// A prerelease identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Identifier {
    /// Digits only, compared by value, below every alphanumeric one.
    Numeric(u64),
    /// Any other, compared by its ASCII bytes.
    Alphanumeric(String),
}

impl Ord for Identifier {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::Numeric(_), Self::Alphanumeric(_)) => Ordering::Less,
            (Self::Alphanumeric(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Alphanumeric(a), Self::Alphanumeric(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Version {
    /// Parses a strict semver version, or `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let (rest, build) = match text.split_once('+') {
            Some((rest, build)) => (rest, Some(build)),
            None => (text, None),
        };
        if let Some(build) = build
            && !build.split('.').all(is_identifier)
        {
            return None;
        }
        let (core, pre) = match rest.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (rest, None),
        };
        let mut parts = core.split('.');
        let core = [
            parse_number(parts.next()?)?,
            parse_number(parts.next()?)?,
            parse_number(parts.next()?)?,
        ];
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            Some(pre) => pre
                .split('.')
                .map(parse_identifier)
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        Some(Self { core, pre })
    }

    /// A release, `MAJOR.MINOR.PATCH`.
    const fn release(core: [u64; 3]) -> Self {
        Self {
            core,
            pre: Vec::new(),
        }
    }

    /// The least version of `core`, its prerelease `0`: the exclusive upper
    /// bound npm gives a caret, below every prerelease of `core`.
    fn least(core: [u64; 3]) -> Self {
        Self {
            core,
            pre: vec![Identifier::Numeric(0)],
        }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            // a release ranks above its prereleases
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A number of a version: digits, no leading zero but `0` itself.
fn parse_number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None;
    }
    text.parse().ok()
}

/// Whether `text` is a build or prerelease identifier's characters: one or
/// more of `[0-9A-Za-z-]`.
fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// A prerelease identifier, a numeric one without a leading zero.
fn parse_identifier(text: &str) -> Option<Identifier> {
    if !is_identifier(text) {
        return None;
    }
    if text.bytes().all(|byte| byte.is_ascii_digit()) {
        return parse_number(text).map(Identifier::Numeric);
    }
    Some(Identifier::Alphanumeric(text.to_owned()))
}

/// How a comparator compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// `>=`.
    AtLeast,
    /// `>`.
    Above,
    /// `<=`.
    AtMost,
    /// `<`.
    Below,
    /// `=`, or a version alone.
    Exactly,
}

/// One comparison a version must pass.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Comparator {
    op: Op,
    version: Version,
}

impl Comparator {
    fn passes(&self, version: &Version) -> bool {
        let order = version.cmp(&self.version);
        match self.op {
            Op::AtLeast => order != Ordering::Less,
            Op::Above => order == Ordering::Greater,
            Op::AtMost => order != Ordering::Greater,
            Op::Below => order == Ordering::Less,
            Op::Exactly => order == Ordering::Equal,
        }
    }
}

/// A range of versions, from a `package.json`'s dependency field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRange {
    /// The comparator sets, any one of which a version meets the range by.
    sets: Vec<Vec<Comparator>>,
}

/// A range the subset doesn't read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadRange;

impl fmt::Display for UnreadRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "a range is comparator sets joined by '||', each of '>=', '>', '<=', '<', or '=' \
             before a whole version, a whole version alone, or '^' before a version",
        )
    }
}

impl VersionRange {
    /// Parses a range of the subset the module doc gives.
    ///
    /// # Errors
    ///
    /// Returns [`UnreadRange`] for any other form.
    pub fn parse(text: &str) -> Result<Self, UnreadRange> {
        let sets = text
            .split("||")
            .map(|set| {
                let comparators: Vec<&str> = set.split_ascii_whitespace().collect();
                if comparators.is_empty() {
                    return Err(UnreadRange);
                }
                let mut parsed = Vec::with_capacity(comparators.len() + 1);
                for comparator in comparators {
                    parse_comparator(comparator, &mut parsed)?;
                }
                Ok(parsed)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { sets })
    }

    /// Whether `version` is in the range.
    pub fn contains(&self, version: &Version) -> bool {
        self.sets.iter().any(|set| {
            set.iter().all(|comparator| comparator.passes(version))
                && (version.pre.is_empty()
                    || set.iter().any(|comparator| {
                        !comparator.version.pre.is_empty()
                            && comparator.version.core == version.core
                    }))
        })
    }
}

/// Parses one comparator into `set`: a caret's two, any other's one.
fn parse_comparator(text: &str, set: &mut Vec<Comparator>) -> Result<(), UnreadRange> {
    if let Some(partial) = text.strip_prefix('^') {
        let (lower, upper) = caret(partial).ok_or(UnreadRange)?;
        set.push(Comparator {
            op: Op::AtLeast,
            version: lower,
        });
        set.push(Comparator {
            op: Op::Below,
            version: upper,
        });
        return Ok(());
    }
    let (op, rest) = [
        (">=", Op::AtLeast),
        ("<=", Op::AtMost),
        (">", Op::Above),
        ("<", Op::Below),
        ("=", Op::Exactly),
    ]
    .iter()
    .find_map(|(prefix, op)| text.strip_prefix(prefix).map(|rest| (*op, rest)))
    .unwrap_or((Op::Exactly, text));
    let version = Version::parse(rest).ok_or(UnreadRange)?;
    set.push(Comparator { op, version });
    Ok(())
}

/// The bounds of `^partial`: the least version it allows, and the
/// exclusive bound above, as npm reads a caret.
fn caret(partial: &str) -> Option<(Version, Version)> {
    if let Some(version) = Version::parse(partial) {
        let [major, minor, patch] = version.core;
        // a part at its type's limit has no bound above, so the range is refused
        let upper = if major > 0 {
            [major.checked_add(1)?, 0, 0]
        } else if minor > 0 {
            [0, minor.checked_add(1)?, 0]
        } else {
            [0, 0, patch.checked_add(1)?]
        };
        return Some((version, Version::least(upper)));
    }
    let mut parts = partial.split('.');
    let major = parse_number(parts.next()?)?;
    let minor = parts.next().map(parse_number);
    if parts.next().is_some() {
        return None;
    }
    match minor {
        None => Some((
            Version::release([major, 0, 0]),
            Version::least([major.checked_add(1)?, 0, 0]),
        )),
        Some(minor) => {
            let minor = minor?;
            let upper = if major > 0 {
                [major.checked_add(1)?, 0, 0]
            } else {
                [0, minor.checked_add(1)?, 0]
            };
            Some((Version::release([major, minor, 0]), Version::least(upper)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("{text}"))
    }

    fn contains(range: &str, text: &str) -> bool {
        VersionRange::parse(range)
            .unwrap_or_else(|_| panic!("{range}"))
            .contains(&version(text))
    }

    #[test]
    fn versions_parse_strictly() {
        for text in [
            "0.0.0",
            "1.2.3",
            "10.20.30",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-0.3.7",
            "1.0.0-x-y.0",
            "1.0.0+build.5",
            "1.0.0-rc.1+build.1",
        ] {
            assert!(Version::parse(text).is_some(), "{text}");
        }
        for text in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "v1.2.3",
            "=1.2.3",
            " 1.2.3",
            "1.2.3 ",
            "1.2.3-",
            "1.2.3-01",
            "1.2.3-a..b",
            "1.2.3-a_b",
            "1.2.3+",
            "1.2.3+a..b",
            "1.x.3",
            "a.b.c",
            "18446744073709551616.0.0",
        ] {
            assert_eq!(Version::parse(text), None, "{text}");
        }
    }

    /// The precedence semver gives, build metadata ignored.
    #[test]
    fn versions_order_by_precedence() {
        let ordered = [
            "1.0.0-0",
            "1.0.0-1",
            "1.0.0-2",
            "1.0.0-10",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "1.10.0",
            "2.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(version(pair[0]) < version(pair[1]), "{pair:?}");
        }
        assert_eq!(version("1.0.0+a").cmp(&version("1.0.0+b")), Ordering::Equal);
    }

    /// Each range gro's `package.json` declares, read as npm reads it.
    #[test]
    fn the_declared_ranges_read_as_npm_reads_them() {
        let cases: [(&str, &[&str], &[&str]); 9] = [
            (
                ">=0.83.0",
                &["0.83.0", "0.83.1", "0.84.0", "1.0.0", "99.0.0"],
                &["0.82.9", "0.0.1", "0.84.0-next.1", "0.83.0-rc.1"],
            ),
            (">=0.5.7", &["0.5.7", "0.6.1"], &["0.5.6", "0.5.7-0"]),
            (
                "^6",
                &["6.0.0", "6.0.3", "6.9.9"],
                &["5.9.9", "7.0.0", "7.0.0-0", "6.1.0-rc.1", "6.0.0-beta"],
            ),
            ("^0.2.0", &["0.2.0", "0.2.9"], &["0.3.0", "0.1.9", "1.0.0"]),
            ("^3", &["3.0.0", "3.5.1"], &["4.0.0", "2.9.9"]),
            (">=0.18.1", &["0.18.1", "1.2.0"], &["0.18.0"]),
            (
                "^5.0.3 || ^6.0.0 || ^7.0.0 || ^8.0.0",
                &["5.0.3", "5.4.0", "6.0.0", "7.3.1", "8.0.0"],
                &["5.0.2", "4.9.0", "9.0.0", "8.0.0-beta.1"],
            ),
            (
                "^3 || ^4 || ^5",
                &["3.0.0", "4.2.0", "5.9.0"],
                &["2.0.0", "6.0.0"],
            ),
            ("^5", &["5.0.0", "5.55.2"], &["6.0.0"]),
        ];
        for (range, inside, outside) in cases {
            for text in inside {
                assert!(contains(range, text), "{text} in {range}");
            }
            for text in outside {
                assert!(!contains(range, text), "{text} outside {range}");
            }
        }
    }

    /// The caret's bounds for each part left off or zero, and a
    /// prerelease met only by a comparator naming one of its release.
    #[test]
    fn carets_and_prereleases_read_as_npm_reads_them() {
        let cases: [(&str, &[&str], &[&str]); 9] = [
            (
                "^1.2.3",
                &["1.2.3", "1.9.0"],
                &["1.2.2", "2.0.0", "2.0.0-0"],
            ),
            ("^0.2.3", &["0.2.3", "0.2.9"], &["0.2.2", "0.3.0"]),
            ("^0.0.3", &["0.0.3"], &["0.0.4", "0.0.2"]),
            ("^1.2", &["1.2.0", "1.9.9"], &["1.1.9", "2.0.0"]),
            ("^0.2", &["0.2.0", "0.2.9"], &["0.3.0", "0.1.0"]),
            ("^0.0", &["0.0.0", "0.0.9"], &["0.1.0"]),
            ("^0", &["0.0.0", "0.9.9"], &["1.0.0"]),
            (
                "^1.2.3-beta.2",
                &["1.2.3-beta.2", "1.2.3-beta.10", "1.2.3", "1.5.0"],
                &["1.2.3-beta.1", "1.2.4-beta.1", "2.0.0"],
            ),
            (
                ">=1.3.0-rc.0 <2.0.0",
                &["1.3.0-rc.1", "1.3.0", "1.9.0"],
                &["1.4.0-rc.1", "1.2.0", "2.0.0"],
            ),
        ];
        for (range, inside, outside) in cases {
            for text in inside {
                assert!(contains(range, text), "{text} in {range}");
            }
            for text in outside {
                assert!(!contains(range, text), "{text} outside {range}");
            }
        }
        assert!(contains("=1.2.3", "1.2.3"));
        assert!(contains("1.2.3", "1.2.3+build"));
        assert!(!contains("1.2.3", "1.2.4"));
        assert!(contains(">1.2.3", "1.2.4") && !contains(">1.2.3", "1.2.3"));
        assert!(contains("<=1.2.3", "1.2.3") && !contains("<=1.2.3", "1.2.4"));
        assert!(contains("<1.2.3", "1.2.2") && !contains("<1.2.3", "1.2.3"));
    }

    #[test]
    fn forms_outside_the_subset_are_refused() {
        for range in [
            "",
            " ",
            "||",
            "^1 ||",
            "|| ^1",
            "~1.2.3",
            "1.x",
            "1.2.x",
            "*",
            "x",
            "latest",
            "1.2.3 - 2.0.0",
            ">= 1.2.3",
            ">=1.2",
            "<2",
            "=1",
            "1",
            "^1.2.3.4",
            "^01",
            "^1.x",
            "^",
            "^v1",
            "^1.2.3-",
            "!=1.2.3",
            "=>1.2.3",
            ">==1.2.3",
            "workspace:*",
            "npm:foo@1",
            // a caret whose bound above would pass the largest number
            "^18446744073709551615",
            "^18446744073709551615.0.0",
            "^0.18446744073709551615",
            "^0.0.18446744073709551615",
        ] {
            assert_eq!(VersionRange::parse(range), Err(UnreadRange), "{range:?}");
        }
    }
}
