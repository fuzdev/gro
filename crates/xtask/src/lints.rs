//! A check that the crates allowed `unsafe` keep the workspace's lints.
//!
//! Cargo replaces a crate's `[lints]` table whole, so `gro_sys` and `gro_v8`,
//! which need `unsafe_code` at `deny` rather than the workspace's `forbid`,
//! re-declare every lint, and their tables must change with the workspace's.

use std::collections::BTreeMap;

/// A lint table: each `(group, lint)` (`rust`, `rustdoc`, `clippy`) with its
/// level as written.
type Lints = BTreeMap<(String, String), String>;

/// The lints in the manifest `text` under the sections `[<prefix>.<group>]`,
/// read line by line: a `key = value` line, comments and blank lines skipped,
/// any other section ending the table.
fn lints_in(text: &str, prefix: &str) -> Lints {
    let mut lints = Lints::new();
    let mut group = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            group = section
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('.'))
                .map(str::to_owned);
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(group) = &group
            && let Some((lint, level)) = line.split_once('=')
        {
            lints.insert(
                (group.clone(), lint.trim().to_owned()),
                level.trim().to_owned(),
            );
        }
    }
    lints
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `gro_sys`'s and `gro_v8`'s re-declared tables equal the workspace's,
    /// but `unsafe_code` at `deny` and `clippy::undocumented_unsafe_blocks`
    /// added.
    #[test]
    fn unsafe_crates_keep_the_workspace_lints() {
        let mut expected = lints_in(include_str!("../../../Cargo.toml"), "workspace.lints");
        assert!(
            expected.len() > 1,
            "the workspace's lints read as {expected:?}"
        );
        let rust = |lint: &str| ("rust".to_owned(), lint.to_owned());
        assert_eq!(
            expected.insert(rust("unsafe_code"), "\"deny\"".to_owned()),
            Some("\"forbid\"".to_owned())
        );
        expected.insert(
            ("clippy".to_owned(), "undocumented_unsafe_blocks".to_owned()),
            "\"warn\"".to_owned(),
        );
        for (name, manifest) in [
            ("gro_sys", include_str!("../../gro_sys/Cargo.toml")),
            ("gro_v8", include_str!("../../gro_v8/Cargo.toml")),
        ] {
            assert_eq!(lints_in(manifest, "lints"), expected, "{name}");
        }
    }

    #[test]
    fn lints_are_read_by_section() {
        let text = "[workspace.lints.rust]\n# a comment\nx = \"warn\"\n\n\
                    [workspace.lints.clippy]\nall = { level = \"warn\", priority = -1 }\n\
                    [profile.release]\nlto = true\n";
        assert_eq!(
            lints_in(text, "workspace.lints"),
            Lints::from([
                (("rust".into(), "x".into()), "\"warn\"".into()),
                (
                    ("clippy".into(), "all".into()),
                    "{ level = \"warn\", priority = -1 }".into()
                ),
            ])
        );
        assert!(lints_in(text, "lints").is_empty());
    }
}
