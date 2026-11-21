//! Gro's own library as this binary embeds it, when built with the
//! `snapshot` feature (`gro_runtime::embed`), and `make-embed`, the internal
//! subcommand that writes it.

use std::path::Path;

#[cfg(test)]
use gro_runtime::embed::EmbedContents;
use gro_runtime::embed::{EMBED_FILE, Embed, make_embed as make_blob};

use crate::error::CliError;

/// The blob `fgro make-embed` wrote, in the directory `GRO_SNAPSHOT_DIR`
/// names: an array, not a reference to one, so the binary holds no pointer
/// to it for its loader to relocate at every start.
#[cfg(feature = "snapshot")]
mod embedded {
    pub static EMBED: [u8; include_bytes!(concat!(env!("GRO_SNAPSHOT_DIR"), "/embed.bin")).len()] =
        *include_bytes!(concat!(env!("GRO_SNAPSHOT_DIR"), "/embed.bin"));
}

/// The blob this binary was built with, undecoded: none without the
/// `snapshot` feature.
#[cfg(feature = "snapshot")]
pub const EMBEDDED_LIB: Option<&[u8]> = Some(&embedded::EMBED);

#[cfg(not(feature = "snapshot"))]
pub const EMBEDDED_LIB: Option<&[u8]> = None;

/// Writes the blob of the modules `<src>/embed.json` embeds, read from
/// `<src>/lib`, into `dir` as `embed.bin`, creating `dir` first. A file
/// already holding the same bytes is left as it is, so its unchanged
/// modification time doesn't rebuild what includes it.
///
/// A binary that embeds a blob ([`EMBEDDED_LIB`]) then holds its own to the
/// one it made, module by module, decoding both: one that differs was built
/// from other sources than `src` holds now, and fails naming the first
/// module that differs.
///
/// # Errors
///
/// Returns why the directory, the blob, or its file failed, or the
/// embedded blob's difference.
pub fn make_embed(src: &Path, dir: &Path) -> Result<(), CliError> {
    let made = make_blob(src)?;
    std::fs::create_dir_all(dir).map_err(|source| CliError::OutputDir {
        path: dir.to_owned(),
        source,
    })?;
    let file = dir.join(EMBED_FILE);
    if std::fs::read(&file).ok().as_deref() != Some(made.as_slice()) {
        std::fs::write(&file, &made).map_err(|source| CliError::EmbedWrite {
            path: file.clone(),
            source,
        })?;
    }
    match EMBEDDED_LIB {
        Some(embedded) => compare(&Embed::decode(embedded)?, &Embed::decode(&made)?),
        None => Ok(()),
    }
}

/// Fails unless `embedded` holds what `made` holds, naming the first
/// difference: the first path whose module differs or is in one and not the
/// other, else the first path left out of one and not the other, else
/// `../package.json` when the peers differ.
fn compare(embedded: &Embed<'_>, made: &Embed<'_>) -> Result<(), CliError> {
    fn modules<'a>(embed: &Embed<'a>) -> Vec<(&'a str, &'a str)> {
        embed.iter().collect()
    }
    fn left_out<'a>(embed: &Embed<'a>) -> Vec<(&'a str, &'a str)> {
        embed.left_out().map(|path| (path, "")).collect()
    }
    let difference = first_difference(&modules(embedded), &modules(made))
        .or_else(|| first_difference(&left_out(embedded), &left_out(made)))
        .or_else(|| {
            embedded
                .peers()
                .ne(made.peers())
                .then_some("../package.json")
        });
    difference.map_or(Ok(()), |path| {
        Err(CliError::EmbedStale {
            path: path.to_owned(),
        })
    })
}

/// The first key, in two lists of `(key, value)` each in its keys' order,
/// whose value differs or that one list holds and the other doesn't.
fn first_difference<'a>(a: &[(&'a str, &'a str)], b: &[(&'a str, &'a str)]) -> Option<&'a str> {
    a.iter()
        .zip(b)
        .find(|(a, b)| a != b)
        .map(|((a, _), (b, _))| *a.min(b))
        .or_else(|| {
            let (longer, shorter) = if a.len() > b.len() { (a, b) } else { (b, a) };
            longer.get(shorter.len()).map(|(key, _)| *key)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embed of the plain build `cargo test` runs: none, whatever the
    /// shell sets.
    #[test]
    fn a_plain_build_embeds_no_blob() {
        assert_eq!(EMBEDDED_LIB.is_some(), cfg!(feature = "snapshot"));
    }

    /// The file `embedded::EMBED` includes, a literal `concat!` requires:
    /// the one `make-embed` writes.
    #[test]
    fn the_included_file_is_the_one_make_embed_writes() {
        assert_eq!(EMBED_FILE, "embed.bin");
    }

    /// The path [`compare`] names: the first whose module differs, or that
    /// one blob holds and the other doesn't.
    #[test]
    fn compare_names_the_first_module_that_differs() {
        let blob = |modules: &[(&str, &str)]| {
            let modules: Vec<(String, String)> = modules
                .iter()
                .map(|(path, code)| ((*path).to_owned(), (*code).to_owned()))
                .collect();
            gro_runtime::embed::encode(&EmbedContents {
                modules,
                ..EmbedContents::default()
            })
            .unwrap()
        };
        let made = blob(&[("a.ts", "a"), ("b.ts", "b"), ("c.ts", "c")]);
        let made = Embed::decode(&made).unwrap();
        let stale = |modules: &[(&str, &str)]| {
            let embedded = blob(modules);
            match compare(&Embed::decode(&embedded).unwrap(), &made) {
                Ok(()) => None,
                Err(CliError::EmbedStale { path }) => Some(path),
                Err(error) => panic!("{error}"),
            }
        };
        assert_eq!(stale(&[("a.ts", "a"), ("b.ts", "b"), ("c.ts", "c")]), None);
        assert_eq!(
            stale(&[("a.ts", "a"), ("b.ts", "B"), ("c.ts", "c")]).as_deref(),
            Some("b.ts")
        );
        assert_eq!(
            stale(&[("a.ts", "a"), ("c.ts", "c")]).as_deref(),
            Some("b.ts")
        );
        assert_eq!(
            stale(&[("a.ts", "a"), ("b.ts", "b")]).as_deref(),
            Some("c.ts")
        );
        assert_eq!(
            stale(&[("a.ts", "a"), ("b.ts", "b"), ("c.ts", "c"), ("d.ts", "d")]).as_deref(),
            Some("d.ts")
        );
        assert_eq!(stale(&[]).as_deref(), Some("a.ts"));
    }

    /// Past the modules, [`compare`] names the first path left out of one
    /// and not the other, then the peers' `package.json`.
    #[test]
    fn compare_names_a_left_out_path_or_the_peers_that_differ() {
        let contents = |left_out: &[&str], peers: &[(&str, &str)]| EmbedContents {
            modules: vec![("a.ts".to_owned(), "a".to_owned())],
            left_out: left_out.iter().map(|path| (*path).to_owned()).collect(),
            peers: peers
                .iter()
                .map(|(name, range)| ((*name).to_owned(), (*range).to_owned()))
                .collect(),
        };
        let made =
            gro_runtime::embed::encode(&contents(&["b.ts", "c.ts"], &[("p", "^1")])).unwrap();
        let made = Embed::decode(&made).unwrap();
        let stale = |left_out: &[&str], peers: &[(&str, &str)]| {
            let embedded = gro_runtime::embed::encode(&contents(left_out, peers)).unwrap();
            match compare(&Embed::decode(&embedded).unwrap(), &made) {
                Ok(()) => None,
                Err(CliError::EmbedStale { path }) => Some(path),
                Err(error) => panic!("{error}"),
            }
        };
        assert_eq!(stale(&["b.ts", "c.ts"], &[("p", "^1")]), None);
        assert_eq!(stale(&["b.ts"], &[("p", "^1")]).as_deref(), Some("c.ts"));
        assert_eq!(stale(&["c.ts"], &[("p", "^1")]).as_deref(), Some("b.ts"));
        assert_eq!(
            stale(&["b.ts", "c.ts", "d.ts"], &[("p", "^1")]).as_deref(),
            Some("d.ts")
        );
        for peers in [&[("p", "^2")][..], &[], &[("p", "^1"), ("q", "^1")]] {
            assert_eq!(
                stale(&["b.ts", "c.ts"], peers).as_deref(),
                Some("../package.json"),
                "{peers:?}"
            );
        }
    }
}
