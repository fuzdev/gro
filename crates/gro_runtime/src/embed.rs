//! The embed: Gro's own library as the binary carries it.
//!
//! It's the modules of `src/lib` that `src/embed.json` lists as `embedded`,
//! each stripped of its types, in one blob the binary includes, which no
//! command loads yet.
//!
//! [`make_embed`] makes the blob from a checkout's `src/` directory, which
//! holds `embed.json` and `lib/`. It refuses an `embed.json` whose lists
//! aren't sorted, whose `embedded` list names something other than a `.ts`
//! file inside `lib/` or names a host module's file, or whose
//! `host_modules` group isn't exactly the runtime's host modules: the
//! builtin table's rows and the ones it reserves unbuilt
//! ([`crate::resolver::BUILTINS`], [`crate::resolver::UNBUILT_HOST_MODULES`]).
//! So a builtin that lands, or a host module added to Gro, fails the build
//! until the data names it, and the embed never holds a host module's Node
//! file, which the runtime serves as its builtin: a host module's file is
//! the builtin table's ([`crate::resolver::HostModule::file`]), the one
//! mapping the resolver reads too. Each module is stripped by
//! the loader's own strip, which blanks the types in place, every line
//! keeping its length in UTF-16 units, so a frame's line and column point
//! into the source as written; a module the strip refuses fails the build
//! rather than the first command that loads it. The same sources make the
//! same blob, byte for byte: the order is the sorted list's, never a
//! directory's, and the blob holds no path but the modules' own, relative to
//! `lib/`.
//!
//! The blob is a table of `u32` offsets before the text it indexes, never a
//! table of string references, which a binary would hold as pointers its
//! loader relocates at every start, embed used or not. Little-endian
//! throughout:
//!
//! - the count of modules, `n`
//! - `2n + 1` offsets into the text: module `i`'s path starts at offset
//!   `2i`, its code at `2i + 1`, and the next module at `2i + 2`, the last
//!   offset the text's length
//! - the text: each module's path, then its code, in the paths' byte order
//!
//! [`Embed::decode`] checks a blob whole before it reads anything from it:
//! the offsets in order from 0 and within the text, the text UTF-8 with every
//! offset on a character's boundary, and the paths strictly ascending. A
//! module is then found by its path in a binary search.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use url::Url;

use crate::loader::strip::strip_module;
use crate::resolver::host_modules;

/// The name of the blob's file, which the binary's build includes.
pub const EMBED_FILE: &str = "embed.bin";

/// The data file in the `src/` directory [`make_embed`] reads.
const SORT_FILE: &str = "embed.json";

/// The directory of Gro's library in the `src/` directory [`make_embed`]
/// reads, which the paths in [`SORT_FILE`] are relative to.
const LIB_DIR: &str = "lib";

/// Why the embed couldn't be made, or a blob doesn't decode.
#[derive(Debug, Error)]
pub enum EmbedError {
    /// A file couldn't be read.
    #[error("failed to read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// The failure reading it.
        source: std::io::Error,
    },
    /// `embed.json` isn't the data the build reads: not JSON, or a shape
    /// with a field missing or one it doesn't know.
    #[error("{} isn't the embed's data: {source}", path.display())]
    Json {
        /// `embed.json`.
        path: PathBuf,
        /// What the parse found.
        source: serde_json::Error,
    },
    /// A list of `embed.json` isn't sorted, or names a path twice.
    #[error(
        "{} lists {list} out of order: {path:?} after {previous:?}, where each list is sorted \
         and names a path once",
        file.display()
    )]
    Unsorted {
        /// `embed.json`.
        file: PathBuf,
        /// The list.
        list: &'static str,
        /// The path before.
        previous: String,
        /// The path out of order.
        path: String,
    },
    /// The `embedded` list names something other than a `.ts` file inside
    /// `lib/`.
    #[error(
        "{} embeds {path:?}, which isn't the relative path of a `.ts` file inside src/lib",
        file.display()
    )]
    Path {
        /// `embed.json`.
        file: PathBuf,
        /// The path.
        path: String,
    },
    /// The `host_modules` group isn't the runtime's host modules.
    #[error(
        "{}'s host_modules must be the runtime's host modules, its builtins and the ones it \
         reserves unbuilt: {}",
        file.display(),
        host_modules_difference(missing, extra)
    )]
    HostModules {
        /// `embed.json`.
        file: PathBuf,
        /// The runtime's host modules the group doesn't list.
        missing: Vec<String>,
        /// What the group lists that is no host module of the runtime's.
        extra: Vec<String>,
    },
    /// The `embedded` list names a host module's file, which the runtime
    /// serves itself.
    #[error(
        "{} embeds {path:?}, a host module's file, which the runtime serves as its builtin",
        file.display()
    )]
    EmbedsHostModule {
        /// `embed.json`.
        file: PathBuf,
        /// The path.
        path: String,
    },
    /// A module's source isn't UTF-8.
    #[error("{} isn't UTF-8", path.display())]
    NotUtf8 {
        /// The module's file.
        path: PathBuf,
    },
    /// The strip refused a module, with the loader's message, which names
    /// its file and the position.
    #[error("{0}")]
    Strip(String),
    /// The text is past what a `u32` offset names.
    #[error("the embed's modules are larger than 4 GiB, the most its offsets name")]
    TooLarge,
    /// A blob that isn't one [`make_embed`] makes.
    #[error("the embed is malformed: {0}")]
    Malformed(&'static str),
}

/// The missing and extra host modules, as [`EmbedError::HostModules`]
/// names them.
fn host_modules_difference(missing: &[String], extra: &[String]) -> String {
    let list = |paths: &[String]| paths.join(", ");
    match (missing.is_empty(), extra.is_empty()) {
        (false, false) => format!(
            "it lacks {} and lists {}, which aren't",
            list(missing),
            list(extra)
        ),
        (false, true) => format!("it lacks {}", list(missing)),
        _ => format!("it lists {}, which aren't", list(extra)),
    }
}

/// `embed.json` as the build reads it: its shape is checked whole, but
/// only the `embedded` list and the `host_modules` group decide anything
/// here; `src/test/package_module_closure.test.ts` holds the rest.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SortJson {
    #[serde(rename = "description")]
    _description: String,
    embedded: Vec<String>,
    excluded: ExcludedJson,
}

/// The groups `embed.json` leaves out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExcludedJson {
    host_modules: GroupJson,
    #[serde(rename = "node_roots")]
    _node_roots: GroupJson,
    #[serde(rename = "node_loader")]
    _node_loader: GroupJson,
    #[serde(rename = "node_only")]
    _node_only: GroupJson,
    #[serde(rename = "node_builtin_tasks")]
    _node_builtin_tasks: GroupJson,
    #[serde(rename = "genfiles")]
    _genfiles: GroupJson,
    #[serde(rename = "reaches_node")]
    _reaches_node: GroupJson,
}

/// A group `embed.json` leaves out, with its reason.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupJson {
    #[serde(rename = "reason")]
    _reason: String,
    modules: Vec<String>,
}

/// Makes the blob of the modules `src/embed.json` lists as `embedded`,
/// each read from `src/lib` and stripped, where `src` is a checkout's `src/`
/// directory.
///
/// The strip recurses with the source's nesting, on the caller's stack,
/// which must hold tsv's contract (`tsv_ts::STACK_SIZE`), as a load's does.
///
/// # Errors
///
/// Returns the first refusal of `embed.json` the module doc lists, or why
/// a module couldn't be read or stripped.
pub fn make_embed(src: &Path) -> Result<Vec<u8>, EmbedError> {
    // absolute, so each module's URL names its file in a refusal
    let src = std::path::absolute(src).map_err(|source| EmbedError::Read {
        path: src.to_owned(),
        source,
    })?;
    let file = src.join(SORT_FILE);
    let embedded = read_sort(&file)?;
    let lib = src.join(LIB_DIR);
    let mut modules = Vec::with_capacity(embedded.len());
    for path in embedded {
        let module_file = lib.join(&path);
        let bytes = std::fs::read(&module_file).map_err(|source| EmbedError::Read {
            path: module_file.clone(),
            source,
        })?;
        let source = String::from_utf8(bytes).map_err(|_| EmbedError::NotUtf8 {
            path: module_file.clone(),
        })?;
        // as a load's would, the refusal names the file's URL
        let url = Url::from_file_path(&module_file).map_err(|()| EmbedError::Path {
            file: file.clone(),
            path: path.clone(),
        })?;
        let code =
            strip_module(&source, &url).map_err(|refusal| EmbedError::Strip(refusal.message))?;
        modules.push((path, code));
    }
    encode(&modules)
}

/// Reads `embed.json` at `file`, returning its `embedded` list once it's
/// checked as [`make_embed`] says.
fn read_sort(file: &Path) -> Result<Vec<String>, EmbedError> {
    let text = std::fs::read_to_string(file).map_err(|source| EmbedError::Read {
        path: file.to_owned(),
        source,
    })?;
    let sort: SortJson = serde_json::from_str(&text).map_err(|source| EmbedError::Json {
        path: file.to_owned(),
        source,
    })?;
    check_sort(file, &sort)?;
    Ok(sort.embedded)
}

/// Checks `sort`, read from `file`, as [`make_embed`] says.
fn check_sort(file: &Path, sort: &SortJson) -> Result<(), EmbedError> {
    let host_modules = &sort.excluded.host_modules.modules;
    for (list, paths) in [("embedded", &sort.embedded), ("host_modules", host_modules)] {
        if let Some(pair) = paths.windows(2).find(|pair| pair[0] >= pair[1]) {
            return Err(EmbedError::Unsorted {
                file: file.to_owned(),
                list,
                previous: pair[0].clone(),
                path: pair[1].clone(),
            });
        }
    }
    if let Some(path) = sort.embedded.iter().find(|path| !is_lib_module_path(path)) {
        return Err(EmbedError::Path {
            file: file.to_owned(),
            path: path.clone(),
        });
    }
    let expected = host_module_paths();
    let listed: BTreeSet<String> = host_modules.iter().cloned().collect();
    let missing: Vec<String> = expected.difference(&listed).cloned().collect();
    let extra: Vec<String> = listed.difference(&expected).cloned().collect();
    if !missing.is_empty() || !extra.is_empty() {
        return Err(EmbedError::HostModules {
            file: file.to_owned(),
            missing,
            extra,
        });
    }
    if let Some(path) = sort.embedded.iter().find(|path| expected.contains(*path)) {
        return Err(EmbedError::EmbedsHostModule {
            file: file.to_owned(),
            path: path.clone(),
        });
    }
    Ok(())
}

/// The runtime's host modules, each by its file's path in `lib/`: every
/// row of the builtin table, and each module it reserves unbuilt. A
/// specifier outside Gro's package, which no row has, stays whole, so it
/// can't match a path and fails the check naming itself.
fn host_module_paths() -> BTreeSet<String> {
    host_modules()
        .map(|module| module.file().into_owned())
        .collect()
}

/// Whether `path` is the relative path of a `.ts` file inside `lib/`: its
/// segments, split at `/`, none empty, `.`, or `..`, nor holding a `\`.
fn is_lib_module_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "ts")
        && path
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | "..") && !segment.contains('\\'))
}

/// The blob of `modules`, `(path, code)` pairs in their paths' order.
///
/// # Errors
///
/// Returns [`EmbedError::Malformed`] when the paths aren't strictly
/// ascending, and [`EmbedError::TooLarge`] past what an offset names.
pub fn encode(modules: &[(String, String)]) -> Result<Vec<u8>, EmbedError> {
    if modules.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(EmbedError::Malformed("the paths aren't strictly ascending"));
    }
    let offset = |value: usize| u32::try_from(value).map_err(|_| EmbedError::TooLarge);
    let count = offset(modules.len())?;
    let mut offsets = Vec::with_capacity(modules.len() * 2 + 1);
    let mut text = String::new();
    for (path, code) in modules {
        offsets.push(offset(text.len())?);
        text.push_str(path);
        offsets.push(offset(text.len())?);
        text.push_str(code);
    }
    offsets.push(offset(text.len())?);
    let mut blob = Vec::with_capacity(4 + offsets.len() * 4 + text.len());
    blob.extend_from_slice(&count.to_le_bytes());
    for value in offsets {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob.extend_from_slice(text.as_bytes());
    Ok(blob)
}

/// A decoded blob: the embedded modules, found by path.
#[derive(Clone, Copy)]
pub struct Embed<'a> {
    /// The offsets, `2n + 1` little-endian `u32`s.
    offsets: &'a [u8],
    /// The text the offsets index.
    text: &'a str,
}

impl fmt::Debug for Embed<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Embed")
            .field("modules", &self.len())
            .field("text", &self.text.len())
            .finish()
    }
}

/// The size of an offset, and of the count.
const WORD: usize = 4;

impl<'a> Embed<'a> {
    /// Decodes `blob`, checking it whole first, as the module doc says.
    ///
    /// # Errors
    ///
    /// Returns [`EmbedError::Malformed`] naming what the check found.
    pub fn decode(blob: &'a [u8]) -> Result<Self, EmbedError> {
        let (count, rest) = blob
            .split_first_chunk::<WORD>()
            .ok_or(EmbedError::Malformed("it has no count"))?;
        let words = usize::try_from(u32::from_le_bytes(*count))
            .ok()
            .and_then(|count| count.checked_mul(2))
            .and_then(|words| words.checked_add(1))
            .ok_or(EmbedError::Malformed("its count is past what it can hold"))?;
        let offsets_len = words
            .checked_mul(WORD)
            .filter(|len| *len <= rest.len())
            .ok_or(EmbedError::Malformed("it's shorter than its offsets"))?;
        let (offsets, text) = rest.split_at(offsets_len);
        let text =
            std::str::from_utf8(text).map_err(|_| EmbedError::Malformed("its text isn't UTF-8"))?;
        let embed = Self { offsets, text };
        // the first path starts the text, so no byte goes unread
        if embed.offset(0) != 0 {
            return Err(EmbedError::Malformed("its first offset isn't 0"));
        }
        let mut previous = 0;
        for index in 0..words {
            let offset = embed.offset(index);
            if offset < previous {
                return Err(EmbedError::Malformed("its offsets aren't in order"));
            }
            if !text.is_char_boundary(offset) {
                return Err(EmbedError::Malformed(
                    "an offset is past its text or inside a character",
                ));
            }
            previous = offset;
        }
        if previous != text.len() {
            return Err(EmbedError::Malformed(
                "its last offset isn't its text's length",
            ));
        }
        for index in 1..embed.len() {
            if embed.path(index - 1) >= embed.path(index) {
                return Err(EmbedError::Malformed("its paths aren't strictly ascending"));
            }
        }
        Ok(embed)
    }

    /// How many modules it holds.
    pub const fn len(&self) -> usize {
        self.offsets.len() / WORD / 2
    }

    /// Whether it holds no module.
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The code of the module at `path`, relative to `lib/`, if it holds
    /// one.
    pub fn get(&self, path: &str) -> Option<&'a str> {
        let mut low = 0;
        let mut high = self.len();
        while low < high {
            let middle = low + (high - low) / 2;
            match self.path(middle).cmp(path) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(self.code(middle)),
            }
        }
        None
    }

    /// Each module, `(path, code)`, in its paths' order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a str)> + '_ {
        (0..self.len()).map(|index| (self.path(index), self.code(index)))
    }

    /// The offset at `index`, below the count of offsets, which
    /// [`Self::decode`] checked.
    fn offset(&self, index: usize) -> usize {
        let start = index * WORD;
        self.offsets
            .get(start..start + WORD)
            .and_then(|word| <[u8; WORD]>::try_from(word).ok())
            .and_then(|word| usize::try_from(u32::from_le_bytes(word)).ok())
            .unwrap_or(usize::MAX)
    }

    /// The text from the offset at `start` to the next, which
    /// [`Self::decode`] checked are in order and on characters' boundaries.
    fn span(&self, start: usize) -> &'a str {
        self.text
            .get(self.offset(start)..self.offset(start + 1))
            .unwrap_or_default()
    }

    /// The path of module `index`.
    fn path(&self, index: usize) -> &'a str {
        self.span(index * 2)
    }

    /// The code of module `index`.
    fn code(&self, index: usize) -> &'a str {
        self.span(index * 2 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::{BUILTINS, UNBUILT_HOST_MODULES};

    fn modules(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(path, code)| ((*path).to_owned(), (*code).to_owned()))
            .collect()
    }

    /// A blob decodes to the modules it was made of, each found by its
    /// path, and to none it wasn't.
    #[test]
    fn a_blob_decodes_to_its_modules() {
        let pairs = [
            ("a.ts", "export const a = 1;\n"),
            ("b.ts", ""),
            ("internal/c.ts", "// \u{2013} a dash\nexport {};\n"),
            ("internal/d.ts", "x"),
        ];
        let blob = encode(&modules(&pairs)).unwrap();
        let embed = Embed::decode(&blob).unwrap();
        assert_eq!(embed.len(), pairs.len());
        assert_eq!(embed.iter().collect::<Vec<_>>(), pairs);
        for (path, code) in pairs {
            assert_eq!(embed.get(path), Some(code));
        }
        for absent in ["", "a", "a.tsx", "c.ts", "internal/", "z.ts"] {
            assert_eq!(embed.get(absent), None, "{absent}");
        }
        let empty = encode(&[]).unwrap();
        assert_eq!(empty, [0, 0, 0, 0, 0, 0, 0, 0]);
        let embed = Embed::decode(&empty).unwrap();
        assert!(embed.is_empty());
        assert_eq!(embed.get("a.ts"), None);
    }

    /// The layout the module doc gives: the count, the offsets, the text.
    #[test]
    fn a_blob_is_the_count_the_offsets_and_the_text() {
        let blob = encode(&modules(&[("a.ts", "xy"), ("b.ts", "z")])).unwrap();
        let mut expected = vec![2, 0, 0, 0];
        for offset in [0_u32, 4, 6, 10, 11] {
            expected.extend_from_slice(&offset.to_le_bytes());
        }
        expected.extend_from_slice(b"a.tsxyb.tsz");
        assert_eq!(blob, expected);
    }

    #[test]
    fn encode_refuses_paths_out_of_order() {
        for pairs in [[("b.ts", ""), ("a.ts", "")], [("a.ts", ""), ("a.ts", "")]] {
            assert!(matches!(
                encode(&modules(&pairs)),
                Err(EmbedError::Malformed(_))
            ));
        }
    }

    /// Every kind of malformed blob is refused before anything is read
    /// from it, never read past or panicked on.
    #[test]
    fn decode_refuses_a_malformed_blob() {
        let good = encode(&modules(&[("a.ts", "xy"), ("b.ts", "z")])).unwrap();
        let with = |at: usize, value: u32| {
            let mut blob = good.clone();
            blob[at..at + 4].copy_from_slice(&value.to_le_bytes());
            blob
        };
        let mut not_utf8 = good.clone();
        let last = not_utf8.len() - 1;
        not_utf8[last] = 0xff;
        let mut split = encode(&modules(&[("a.ts", "\u{e9}")])).unwrap();
        // the path's end moved inside the code's two-byte character
        split[8..12].copy_from_slice(&5_u32.to_le_bytes());
        let cases: [(&str, Vec<u8>); 12] = [
            ("empty", vec![]),
            ("short count", vec![1, 0]),
            ("text before the first path", with(4, 1)),
            (
                "text in an empty blob",
                [0, 0, 0, 0, 1, 0, 0, 0, b'a'].to_vec(),
            ),
            ("count past the offsets", with(0, 3)),
            ("count past usize", with(0, u32::MAX)),
            ("an offset past the text", with(4 + 4 * 4, 12)),
            ("offsets out of order", with(4 + 4 * 2, 3)),
            ("last offset short", with(4 + 4 * 4, 10)),
            ("text not UTF-8", not_utf8),
            ("an offset inside a character", split),
            (
                "paths out of order",
                [
                    vec![2, 0, 0, 0],
                    [0_u32, 4, 6, 10, 11]
                        .iter()
                        .flat_map(|offset| offset.to_le_bytes())
                        .collect(),
                    b"b.tsxya.tsz".to_vec(),
                ]
                .concat(),
            ),
        ];
        for (name, blob) in cases {
            assert!(
                matches!(Embed::decode(&blob), Err(EmbedError::Malformed(_))),
                "{name}"
            );
        }
        assert!(Embed::decode(&good).is_ok());
    }

    /// The runtime's host modules, as the check reads them: a row's and an
    /// unbuilt one's file each.
    #[test]
    fn host_module_paths_are_the_rows_and_the_unbuilt() {
        let paths = host_module_paths();
        assert_eq!(paths.len(), BUILTINS.len() + UNBUILT_HOST_MODULES.len());
        for path in [
            "fs.ts",
            "fs_error.ts",
            "tsv.ts",
            "net.ts",
            "watch.ts",
            "stdin.ts",
        ] {
            assert!(paths.contains(path), "{path}");
        }
        assert!(
            paths
                .iter()
                .all(|path| is_lib_module_path(path) && !path.contains('@')),
            "{paths:?}"
        );
    }

    /// A sort read from JSON, `embedded` and `host_modules` given and every
    /// other group empty.
    fn sort(embedded: &[&str], host_modules: &[&str]) -> SortJson {
        let group = |modules: &[&str]| serde_json::json!({"reason": "r", "modules": modules});
        serde_json::from_value(serde_json::json!({
            "description": "d",
            "embedded": embedded,
            "excluded": {
                "host_modules": group(host_modules),
                "node_roots": group(&[]),
                "node_loader": group(&[]),
                "node_only": group(&[]),
                "node_builtin_tasks": group(&[]),
                "genfiles": group(&[]),
                "reaches_node": group(&[]),
            }
        }))
        .unwrap()
    }

    #[test]
    fn check_sort_accepts_the_runtime_s_host_modules() {
        let paths = host_module_paths();
        let host_modules: Vec<&str> = paths.iter().map(String::as_str).collect();
        let file = Path::new("embed.json");
        check_sort(file, &sort(&["a.ts", "internal/b.ts"], &host_modules)).unwrap();
        check_sort(file, &sort(&[], &host_modules)).unwrap();
    }

    /// Each refusal of the module doc, each by its kind.
    #[test]
    fn check_sort_refuses_what_the_build_cant_embed() {
        let paths = host_module_paths();
        let host_modules: Vec<&str> = paths.iter().map(String::as_str).collect();
        let file = Path::new("embed.json");
        let refused = |embedded: &[&str], host_modules: &[&str]| {
            check_sort(file, &sort(embedded, host_modules)).unwrap_err()
        };
        for embedded in [&["b.ts", "a.ts"][..], &["a.ts", "a.ts"]] {
            assert!(matches!(
                refused(embedded, &host_modules),
                EmbedError::Unsorted {
                    list: "embedded",
                    ..
                }
            ));
        }
        let mut reversed = host_modules.clone();
        reversed.reverse();
        assert!(matches!(
            refused(&[], &reversed),
            EmbedError::Unsorted {
                list: "host_modules",
                ..
            }
        ));
        for path in [
            "a.js",
            "/a.ts",
            "../a.ts",
            "./a.ts",
            "a/../b.ts",
            "a//b.ts",
            "a\\b.ts",
            "",
        ] {
            assert!(
                matches!(refused(&[path], &host_modules), EmbedError::Path { .. }),
                "{path}"
            );
        }
        let without_fs: Vec<&str> = host_modules
            .iter()
            .copied()
            .filter(|path| *path != "fs.ts")
            .collect();
        let EmbedError::HostModules { missing, extra, .. } = refused(&[], &without_fs) else {
            panic!("not a host-module refusal");
        };
        assert_eq!((missing, extra), (vec!["fs.ts".to_owned()], vec![]));
        let mut with_extra = host_modules.clone();
        with_extra.push("zz.ts");
        let EmbedError::HostModules { missing, extra, .. } = refused(&[], &with_extra) else {
            panic!("not a host-module refusal");
        };
        assert_eq!((missing, extra), (vec![], vec!["zz.ts".to_owned()]));
        assert!(matches!(
            refused(&["fs.ts"], &host_modules),
            EmbedError::EmbedsHostModule { .. }
        ));
    }

    /// The JSON's shape is checked whole: a group missing, or a field it
    /// doesn't know, is refused.
    #[test]
    fn the_sort_s_shape_is_checked() {
        let parse = |value: serde_json::Value| serde_json::from_value::<SortJson>(value).is_err();
        let group = serde_json::json!({"reason": "r", "modules": []});
        let groups = serde_json::json!({
            "host_modules": group, "node_roots": group, "node_loader": group,
            "node_only": group, "node_builtin_tasks": group, "genfiles": group,
            "reaches_node": group,
        });
        let whole = serde_json::json!({"description": "d", "embedded": [], "excluded": groups});
        assert!(!parse(whole.clone()));
        let mut extra = whole.clone();
        extra["other"] = serde_json::json!([]);
        assert!(parse(extra));
        let mut missing = whole.clone();
        missing["excluded"]
            .as_object_mut()
            .unwrap()
            .remove("genfiles");
        assert!(parse(missing));
        let mut extra_group = whole;
        extra_group["excluded"]["other"] = group;
        assert!(parse(extra_group));
    }

    /// The checkout's own: `src/embed.json` makes a blob, each module's
    /// code its source in `src/lib` stripped in place.
    #[test]
    fn the_checkout_s_embed_is_its_stripped_modules() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src");
        let blob = tsv_ts::run_with_stack(|| make_embed(&src).unwrap()).unwrap();
        let embed = Embed::decode(&blob).unwrap();
        let embedded = read_sort(&src.join(SORT_FILE)).unwrap();
        assert!(!embedded.is_empty());
        assert_eq!(
            embed.iter().map(|(path, _)| path).collect::<Vec<_>>(),
            embedded
        );
        // blanked in place: each line keeps its count of UTF-16 units
        let lines = |text: &str| -> Vec<usize> {
            text.split('\n')
                .map(|line| line.encode_utf16().count())
                .collect()
        };
        let mut stripped = 0;
        for (path, code) in embed.iter() {
            let source = std::fs::read_to_string(src.join(LIB_DIR).join(path)).unwrap();
            assert_eq!(lines(code), lines(&source), "{path}");
            stripped += usize::from(code != source);
        }
        assert!(stripped > 0);
    }
}
