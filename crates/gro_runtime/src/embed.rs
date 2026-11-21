//! The embed: Gro's own library as the binary carries it.
//!
//! It's the modules of `src/lib` that `src/embed.json` lists as `embedded`,
//! each stripped of its types, in one blob the binary includes and hands a
//! run ([`LoadEmbed`]), whose loader serves them as they are when a
//! specifier of Gro's package names one (`resolver/embedded.rs`); a build
//! that includes none makes the blob from its build tree the first time a
//! run needs it. Beside the modules the blob names the ones `embed.json`
//! leaves out but the host modules, the ones that run only on Node, so a
//! specifier naming one is refused as that rather than as a module Gro
//! doesn't have, and carries the `peerDependencies` of Gro's
//! `package.json`, the packages the embedded modules import from the
//! project and the versions they need.
//!
//! [`make_embed`] makes the blob from a checkout's `src/` directory, which
//! holds `embed.json` and `lib/`, with the checkout's `package.json` beside
//! it. It refuses a peer whose range isn't one the resolver reads
//! (`resolver::semver`), so no range ships that a run couldn't
//! check, and an `embed.json` whose lists
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
//! - the count of modules, `n`, of modules left out, `m`, and of peers, `p`
//! - `2n + m + 2p + 1` offsets into the text, each span's start, the last
//!   offset the text's length: module `i`'s path at `2i` and its code at
//!   `2i + 1`, left-out module `j`'s path at `2n + j`, and peer `k`'s name
//!   at `2n + m + 2k` and its range at `2n + m + 2k + 1`
//! - the text: each module's path then its code, in the paths' byte order;
//!   each left-out module's path, in byte order; each peer's name then its
//!   range, in the names' byte order
//!
//! [`Embed::decode`] checks a blob whole before it reads anything from it:
//! the offsets in order from 0 and within the text, the text UTF-8 with every
//! offset on a character's boundary, the module paths, the left-out paths,
//! and the peers' names each strictly ascending, and no path both embedded
//! and left out. A module, a left-out path, or a peer is then found by a
//! binary search.

use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use url::Url;

use crate::loader::strip::strip_module;
use crate::resolver::semver::VersionRange;
use crate::resolver::{GroLibrary, host_modules};

/// The name of the blob's file, which the binary's build includes.
pub const EMBED_FILE: &str = "embed.bin";

/// The data file in the `src/` directory [`make_embed`] reads.
const SORT_FILE: &str = "embed.json";

/// The directory of Gro's library in the `src/` directory [`make_embed`]
/// reads, which the paths in [`SORT_FILE`] are relative to.
const LIB_DIR: &str = "lib";

/// The checkout's `package.json`, beside the `src/` directory
/// [`make_embed`] reads, whose `peerDependencies` the blob carries.
const PACKAGE_JSON: &str = "package.json";

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
    /// The `embedded` list, or a group it leaves out to run on Node, names
    /// something other than a `.ts` file inside `lib/`.
    #[error(
        "{} lists {path:?}, which isn't the relative path of a `.ts` file inside src/lib",
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
    /// A path both embedded and left out to run on Node.
    #[error("{} lists {path:?} as embedded and as left out", file.display())]
    ListedTwice {
        /// `embed.json`.
        file: PathBuf,
        /// The path.
        path: String,
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
    /// The checkout's `package.json` isn't JSON, or its `peerDependencies`
    /// aren't an object of strings.
    #[error("{}'s peerDependencies aren't an object of version ranges: {reason}", path.display())]
    Peers {
        /// The `package.json`.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// A peer's range isn't one the resolver reads, so a run couldn't check
    /// it.
    #[error(
        "{}'s peer dependency {name:?} has the range {range:?}, which the runtime doesn't \
         read: {}",
        path.display(),
        crate::resolver::semver::UnreadRange
    )]
    PeerRange {
        /// The `package.json`.
        path: PathBuf,
        /// The peer's name.
        name: String,
        /// Its range.
        range: String,
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

/// `embed.json` as the build reads it: its shape is checked whole, and the
/// groups' reasons are left to `src/test/package_module_closure.test.ts`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SortJson {
    #[serde(rename = "description")]
    _description: String,
    embedded: Vec<String>,
    excluded: ExcludedJson,
}

/// The groups `embed.json` leaves out: the host modules, which the runtime
/// serves, and the others, which run only on Node.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExcludedJson {
    host_modules: GroupJson,
    node_roots: GroupJson,
    node_loader: GroupJson,
    node_only: GroupJson,
    node_builtin_tasks: GroupJson,
    genfiles: GroupJson,
    reaches_node: GroupJson,
}

impl ExcludedJson {
    /// The groups of modules that run only on Node, every group but the
    /// host modules.
    const fn node_groups(&self) -> [&GroupJson; 6] {
        [
            &self.node_roots,
            &self.node_loader,
            &self.node_only,
            &self.node_builtin_tasks,
            &self.genfiles,
            &self.reaches_node,
        ]
    }
}

/// A group `embed.json` leaves out, with its reason.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupJson {
    #[serde(rename = "reason")]
    _reason: String,
    modules: Vec<String>,
}

/// Makes the blob of a checkout's `src/` directory, `src`.
///
/// It holds the modules `src/embed.json` lists as `embedded`, each read
/// from `src/lib` and stripped, the modules it leaves out to run on Node,
/// and the peers of the `package.json` beside `src`.
///
/// The strip recurses with the source's nesting, on the caller's stack,
/// which must hold tsv's contract (`tsv_ts::STACK_SIZE`), as a load's does.
///
/// # Errors
///
/// Returns the first refusal of `embed.json` the module doc lists, or why
/// a module couldn't be read or stripped, or the peers read.
pub fn make_embed(src: &Path) -> Result<Vec<u8>, EmbedError> {
    // absolute, so each module's URL names its file in a refusal
    let src = std::path::absolute(src).map_err(|source| EmbedError::Read {
        path: src.to_owned(),
        source,
    })?;
    let file = src.join(SORT_FILE);
    let Sorted { embedded, left_out } = read_sort(&file)?;
    let peers = read_peers(&src.parent().unwrap_or(&src).join(PACKAGE_JSON))?;
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
    encode(&EmbedContents {
        modules,
        left_out,
        peers,
    })
}

/// The paths `embed.json` sorts: the modules embedded, and those left out
/// to run on Node, each list in order.
struct Sorted {
    embedded: Vec<String>,
    left_out: Vec<String>,
}

/// Reads `embed.json` at `file`, returning its `embedded` list and the
/// modules it leaves out to run on Node once it's checked as [`make_embed`]
/// says.
fn read_sort(file: &Path) -> Result<Sorted, EmbedError> {
    let text = std::fs::read_to_string(file).map_err(|source| EmbedError::Read {
        path: file.to_owned(),
        source,
    })?;
    let sort: SortJson = serde_json::from_str(&text).map_err(|source| EmbedError::Json {
        path: file.to_owned(),
        source,
    })?;
    check_sort(file, &sort)?;
    let left_out: BTreeSet<String> = sort
        .excluded
        .node_groups()
        .into_iter()
        .flat_map(|group| group.modules.iter().cloned())
        .collect();
    Ok(Sorted {
        embedded: sort.embedded,
        left_out: left_out.into_iter().collect(),
    })
}

/// Reads the `peerDependencies` of the `package.json` at `path`, in their
/// names' order, refusing a range the resolver doesn't read.
fn read_peers(path: &Path) -> Result<Vec<(String, String)>, EmbedError> {
    let text = std::fs::read_to_string(path).map_err(|source| EmbedError::Read {
        path: path.to_owned(),
        source,
    })?;
    parse_peers(path, &text)
}

/// The `peerDependencies` of `text`, the `package.json` at `path`, as
/// [`read_peers`] reads them.
fn parse_peers(path: &Path, text: &str) -> Result<Vec<(String, String)>, EmbedError> {
    let fault = |reason: String| EmbedError::Peers {
        path: path.to_owned(),
        reason,
    };
    let package: serde_json::Value =
        serde_json::from_str(text).map_err(|error| fault(error.to_string()))?;
    let peers = match package.get("peerDependencies") {
        None => return Ok(Vec::new()),
        Some(serde_json::Value::Object(peers)) => peers,
        Some(_) => return Err(fault("not an object".to_owned())),
    };
    let mut read = Vec::with_capacity(peers.len());
    for (name, range) in peers {
        let Some(range) = range.as_str() else {
            return Err(fault(format!("{name:?}'s range isn't a string")));
        };
        if VersionRange::parse(range).is_err() {
            return Err(EmbedError::PeerRange {
                path: path.to_owned(),
                name: name.clone(),
                range: range.to_owned(),
            });
        }
        read.push((name.clone(), range.to_owned()));
    }
    read.sort();
    Ok(read)
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
    let left_out = sort
        .excluded
        .node_groups()
        .into_iter()
        .flat_map(|group| &group.modules);
    if let Some(path) = sort
        .embedded
        .iter()
        .chain(left_out)
        .find(|path| !is_lib_module_path(path))
    {
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
    if let Some(path) = sort.embedded.iter().find(|path| {
        sort.excluded
            .node_groups()
            .into_iter()
            .any(|group| group.modules.contains(path))
    }) {
        return Err(EmbedError::ListedTwice {
            file: file.to_owned(),
            path: path.clone(),
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

/// What a blob holds, as [`encode`] takes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbedContents {
    /// Each embedded module, `(path, code)`, in its paths' order.
    pub modules: Vec<(String, String)>,
    /// The modules of `src/lib` left out to run only on Node, by path, in
    /// order.
    pub left_out: Vec<String>,
    /// Gro's peer dependencies, `(name, range)`, in their names' order.
    pub peers: Vec<(String, String)>,
}

/// The blob of `contents`.
///
/// # Errors
///
/// Returns [`EmbedError::Malformed`] when a list isn't strictly ascending
/// or a path is both embedded and left out, and [`EmbedError::TooLarge`]
/// past what an offset names.
pub fn encode(contents: &EmbedContents) -> Result<Vec<u8>, EmbedError> {
    let EmbedContents {
        modules,
        left_out,
        peers,
    } = contents;
    if modules.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(EmbedError::Malformed("the paths aren't strictly ascending"));
    }
    if left_out.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(EmbedError::Malformed(
            "the left-out paths aren't strictly ascending",
        ));
    }
    if peers.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(EmbedError::Malformed(
            "the peers' names aren't strictly ascending",
        ));
    }
    if left_out.iter().any(|path| {
        modules
            .binary_search_by(|(module, _)| module.cmp(path))
            .is_ok()
    }) {
        return Err(EmbedError::Malformed(
            "a path is both embedded and left out",
        ));
    }
    let offset = |value: usize| u32::try_from(value).map_err(|_| EmbedError::TooLarge);
    let spans: Vec<&str> = modules
        .iter()
        .flat_map(|(path, code)| [path.as_str(), code.as_str()])
        .chain(left_out.iter().map(String::as_str))
        .chain(
            peers
                .iter()
                .flat_map(|(name, range)| [name.as_str(), range.as_str()]),
        )
        .collect();
    let mut blob = Vec::new();
    for count in [modules.len(), left_out.len(), peers.len()] {
        blob.extend_from_slice(&offset(count)?.to_le_bytes());
    }
    let mut text = String::new();
    let mut offsets = Vec::with_capacity(spans.len() + 1);
    for span in spans {
        offsets.push(offset(text.len())?);
        text.push_str(span);
    }
    offsets.push(offset(text.len())?);
    blob.reserve(offsets.len() * WORD + text.len());
    for value in offsets {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob.extend_from_slice(text.as_bytes());
    Ok(blob)
}

/// A decoded blob: the embedded modules, found by path, the modules left
/// out, and the peers.
#[derive(Clone, Copy)]
pub struct Embed<'a> {
    /// The offsets, `2n + m + 2p + 1` little-endian `u32`s.
    offsets: &'a [u8],
    /// The text the offsets index.
    text: &'a str,
    /// The count of modules, `n`.
    modules: usize,
    /// The count of modules left out, `m`.
    left_out: usize,
    /// The count of peers, `p`.
    peers: usize,
}

impl fmt::Debug for Embed<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Embed")
            .field("modules", &self.modules)
            .field("left_out", &self.left_out)
            .field("peers", &self.peers)
            .field("text", &self.text.len())
            .finish()
    }
}

/// How a run gets Gro's library: a function its embedder gives
/// ([`crate::RuntimeOptions::embed`]), called the first time the run needs
/// the library, so a run that never names it decodes nothing.
///
/// # Errors
///
/// Returns why the embedder's copy couldn't be had or decoded.
pub type LoadEmbed = fn() -> Result<Embed<'static>, EmbedError>;

/// Gro's library as a run holds it: the embedder's [`LoadEmbed`], called
/// at most once, when a resolution first needs the library.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunEmbed {
    /// The embedder's function, none when it handed the run no library.
    load: Option<LoadEmbed>,
    /// What it returned, once called: the embed, or why it failed.
    loaded: OnceCell<Result<Embed<'static>, Box<str>>>,
}

impl RunEmbed {
    /// The library `load` gives, not loaded yet.
    pub(crate) const fn new(load: Option<LoadEmbed>) -> Self {
        Self {
            load,
            loaded: OnceCell::new(),
        }
    }

    /// The run's library, loaded the first time it's asked for. `Err(None)`
    /// when the run holds none, and `Err` with the reason when it failed to
    /// load, every time it's asked for.
    pub(crate) fn get(&self) -> GroLibrary<'_> {
        let load = self.load.ok_or(None)?;
        match self
            .loaded
            .get_or_init(|| load().map_err(|error| error.to_string().into()))
        {
            Ok(embed) => Ok(*embed),
            Err(reason) => Err(Some(reason)),
        }
    }
}

/// The size of an offset, and of a count.
const WORD: usize = 4;

/// The counts that lead a blob.
const COUNTS: usize = 3;

impl<'a> Embed<'a> {
    /// Decodes `blob`, checking it whole first, as the module doc says.
    ///
    /// # Errors
    ///
    /// Returns [`EmbedError::Malformed`] naming what the check found.
    pub fn decode(blob: &'a [u8]) -> Result<Self, EmbedError> {
        let (counts, rest) = blob
            .split_first_chunk::<{ COUNTS * WORD }>()
            .ok_or(EmbedError::Malformed("it has no counts"))?;
        let count = |index: usize| {
            counts
                .get(index * WORD..(index + 1) * WORD)
                .and_then(|word| <[u8; WORD]>::try_from(word).ok())
                .and_then(|word| usize::try_from(u32::from_le_bytes(word)).ok())
        };
        let too_many = EmbedError::Malformed("its counts are past what it can hold");
        let (Some(modules), Some(left_out), Some(peers)) = (count(0), count(1), count(2)) else {
            return Err(too_many);
        };
        let words = modules
            .checked_mul(2)
            .and_then(|words| words.checked_add(left_out))
            .and_then(|words| words.checked_add(peers.checked_mul(2)?))
            .and_then(|words| words.checked_add(1))
            .ok_or(too_many)?;
        let offsets_len = words
            .checked_mul(WORD)
            .filter(|len| *len <= rest.len())
            .ok_or(EmbedError::Malformed("it's shorter than its offsets"))?;
        let (offsets, text) = rest.split_at(offsets_len);
        let text =
            std::str::from_utf8(text).map_err(|_| EmbedError::Malformed("its text isn't UTF-8"))?;
        let embed = Self {
            offsets,
            text,
            modules,
            left_out,
            peers,
        };
        // the first span starts the text, so no byte goes unread
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
        let ascending = |len: usize, key: &dyn Fn(usize) -> &'a str| {
            (1..len).all(|index| key(index - 1) < key(index))
        };
        if !ascending(modules, &|index| embed.path(index)) {
            return Err(EmbedError::Malformed("its paths aren't strictly ascending"));
        }
        if !ascending(left_out, &|index| embed.left_out_path(index)) {
            return Err(EmbedError::Malformed(
                "its left-out paths aren't strictly ascending",
            ));
        }
        if !ascending(peers, &|index| embed.peer_name(index)) {
            return Err(EmbedError::Malformed(
                "its peers' names aren't strictly ascending",
            ));
        }
        if embed.left_out().any(|path| embed.get(path).is_some()) {
            return Err(EmbedError::Malformed(
                "a path is both embedded and left out",
            ));
        }
        Ok(embed)
    }

    /// How many modules it holds.
    pub const fn len(&self) -> usize {
        self.modules
    }

    /// Whether it holds no module.
    pub const fn is_empty(&self) -> bool {
        self.modules == 0
    }

    /// The code of the module at `path`, relative to `lib/`, if it holds
    /// one.
    pub fn get(&self, path: &str) -> Option<&'a str> {
        self.module(path).map(|(_, code)| code)
    }

    /// The module at `path`, relative to `lib/`, as `(path, code)`, its
    /// path the blob's own, if it holds one.
    pub fn module(&self, path: &str) -> Option<(&'a str, &'a str)> {
        search(self.modules, |index| self.path(index), path)
            .map(|index| (self.path(index), self.code(index)))
    }

    /// Each module, `(path, code)`, in its paths' order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a str)> + '_ {
        (0..self.modules).map(|index| (self.path(index), self.code(index)))
    }

    /// Whether `path`, relative to `lib/`, is a module left out to run only
    /// on Node.
    pub fn is_left_out(&self, path: &str) -> bool {
        search(self.left_out, |index| self.left_out_path(index), path).is_some()
    }

    /// Each module left out to run only on Node, by path, in order.
    pub fn left_out(&self) -> impl Iterator<Item = &'a str> + '_ {
        (0..self.left_out).map(|index| self.left_out_path(index))
    }

    /// The range of Gro's peer dependency `name`, if it has one.
    pub fn peer_range(&self, name: &str) -> Option<&'a str> {
        search(self.peers, |index| self.peer_name(index), name)
            .map(|index| self.span(self.peer_start(index) + 1))
    }

    /// Each peer dependency, `(name, range)`, in its names' order.
    pub fn peers(&self) -> impl Iterator<Item = (&'a str, &'a str)> + '_ {
        (0..self.peers).map(|index| (self.peer_name(index), self.span(self.peer_start(index) + 1)))
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

    /// The path of left-out module `index`.
    fn left_out_path(&self, index: usize) -> &'a str {
        self.span(self.modules * 2 + index)
    }

    /// The offset index of peer `index`'s name.
    const fn peer_start(&self, index: usize) -> usize {
        self.modules * 2 + self.left_out + index * 2
    }

    /// The name of peer `index`.
    fn peer_name(&self, index: usize) -> &'a str {
        self.span(self.peer_start(index))
    }
}

/// The index below `len` whose key is `target`, by a binary search over
/// keys in strictly ascending order.
fn search<'a>(len: usize, key: impl Fn(usize) -> &'a str, target: &str) -> Option<usize> {
    let mut low = 0;
    let mut high = len;
    while low < high {
        let middle = low + (high - low) / 2;
        match key(middle).cmp(target) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Some(middle),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::{BUILTINS, UNBUILT_HOST_MODULES};

    fn modules(pairs: &[(&str, &str)]) -> EmbedContents {
        EmbedContents {
            modules: pairs
                .iter()
                .map(|(path, code)| ((*path).to_owned(), (*code).to_owned()))
                .collect(),
            ..EmbedContents::default()
        }
    }

    /// A run's copy is loaded once, the first time it's asked for, and a
    /// load that failed is that failure every time after; a run handed none
    /// holds none.
    #[test]
    fn a_run_s_copy_loads_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static LOADS: AtomicUsize = AtomicUsize::new(0);
        static FAILS: AtomicUsize = AtomicUsize::new(0);
        static BLOB: [u8; 16] = [0; 16];
        let embed = RunEmbed::new(Some(|| {
            LOADS.fetch_add(1, Ordering::SeqCst);
            Embed::decode(&BLOB)
        }));
        assert_eq!(LOADS.load(Ordering::SeqCst), 0);
        for _ in 0..3 {
            assert!(embed.get().is_ok_and(|embed| embed.is_empty()));
        }
        assert_eq!(LOADS.load(Ordering::SeqCst), 1);
        let failing = RunEmbed::new(Some(|| {
            FAILS.fetch_add(1, Ordering::SeqCst);
            Err(EmbedError::Malformed("broken"))
        }));
        for _ in 0..3 {
            assert_eq!(
                failing.get().err(),
                Some(Some("the embed is malformed: broken"))
            );
        }
        assert_eq!(FAILS.load(Ordering::SeqCst), 1);
        assert_eq!(RunEmbed::default().get().err(), Some(None));
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
        let empty = encode(&EmbedContents::default()).unwrap();
        assert_eq!(empty, [0; 16]);
        let embed = Embed::decode(&empty).unwrap();
        assert!(embed.is_empty());
        assert_eq!(embed.get("a.ts"), None);
        assert!(!embed.is_left_out("a.ts"));
        assert_eq!(embed.peer_range("a"), None);
    }

    /// The modules left out and the peers decode beside the modules, each
    /// found by its path or name, and nothing else is found as one.
    #[test]
    fn a_blob_decodes_to_its_left_out_modules_and_peers() {
        let contents = EmbedContents {
            left_out: vec!["b.ts".to_owned(), "internal/z.ts".to_owned()],
            peers: vec![
                ("@fuzdev/fuz_util".to_owned(), ">=0.83.0".to_owned()),
                ("typescript".to_owned(), "^6".to_owned()),
            ],
            ..modules(&[("a.ts", "a"), ("c.ts", "c")])
        };
        let blob = encode(&contents).unwrap();
        let embed = Embed::decode(&blob).unwrap();
        assert_eq!(embed.len(), 2);
        assert_eq!(embed.get("a.ts"), Some("a"));
        assert_eq!(embed.get("b.ts"), None);
        assert!(embed.is_left_out("b.ts"));
        assert!(embed.is_left_out("internal/z.ts"));
        for path in ["a.ts", "c.ts", "z.ts", "", "typescript"] {
            assert!(!embed.is_left_out(path), "{path}");
        }
        assert_eq!(
            embed.left_out().collect::<Vec<_>>(),
            ["b.ts", "internal/z.ts"]
        );
        assert_eq!(embed.peer_range("@fuzdev/fuz_util"), Some(">=0.83.0"));
        assert_eq!(embed.peer_range("typescript"), Some("^6"));
        for name in ["@fuzdev/fuz_schema", "a.ts", ">=0.83.0", ""] {
            assert_eq!(embed.peer_range(name), None, "{name}");
        }
        assert_eq!(
            embed.peers().collect::<Vec<_>>(),
            [("@fuzdev/fuz_util", ">=0.83.0"), ("typescript", "^6")]
        );
    }

    /// The layout the module doc gives: the counts, the offsets, the text.
    #[test]
    fn a_blob_is_the_counts_the_offsets_and_the_text() {
        let contents = EmbedContents {
            left_out: vec!["c.ts".to_owned()],
            peers: vec![("p".to_owned(), "^1".to_owned())],
            ..modules(&[("a.ts", "xy"), ("b.ts", "z")])
        };
        let blob = encode(&contents).unwrap();
        let mut expected = Vec::new();
        for count in [2_u32, 1, 1] {
            expected.extend_from_slice(&count.to_le_bytes());
        }
        for offset in [0_u32, 4, 6, 10, 11, 15, 16, 18] {
            expected.extend_from_slice(&offset.to_le_bytes());
        }
        expected.extend_from_slice(b"a.tsxyb.tszc.tsp^1");
        assert_eq!(blob, expected);
    }

    #[test]
    fn encode_refuses_lists_out_of_order() {
        for pairs in [[("b.ts", ""), ("a.ts", "")], [("a.ts", ""), ("a.ts", "")]] {
            assert!(matches!(
                encode(&modules(&pairs)),
                Err(EmbedError::Malformed(_))
            ));
        }
        let owned = |paths: &[&str]| paths.iter().map(|path| (*path).to_owned()).collect();
        for left_out in [&["b.ts", "a.ts"][..], &["a.ts", "a.ts"], &["m.ts"]] {
            let contents = EmbedContents {
                left_out: owned(left_out),
                ..modules(&[("m.ts", "")])
            };
            assert!(
                matches!(encode(&contents), Err(EmbedError::Malformed(_))),
                "{left_out:?}"
            );
        }
        for names in [["b", "a"], ["a", "a"]] {
            let contents = EmbedContents {
                peers: names
                    .iter()
                    .map(|name| ((*name).to_owned(), "^1".to_owned()))
                    .collect(),
                ..EmbedContents::default()
            };
            assert!(
                matches!(encode(&contents), Err(EmbedError::Malformed(_))),
                "{names:?}"
            );
        }
    }

    /// Every kind of malformed blob is refused before anything is read
    /// from it, never read past or panicked on.
    #[test]
    fn decode_refuses_a_malformed_blob() {
        let good = encode(&modules(&[("a.ts", "xy"), ("b.ts", "z")])).unwrap();
        // the first offset, after the three counts
        let first = COUNTS * WORD;
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
        split[first + 4..first + 8].copy_from_slice(&5_u32.to_le_bytes());
        let blob = |counts: [u32; 3], offsets: &[u32], text: &[u8]| {
            [
                counts
                    .iter()
                    .flat_map(|count| count.to_le_bytes())
                    .collect(),
                offsets
                    .iter()
                    .flat_map(|offset| offset.to_le_bytes())
                    .collect(),
                text.to_vec(),
            ]
            .concat()
        };
        let cases: [(&str, Vec<u8>); 17] = [
            ("empty", vec![]),
            ("short counts", vec![1, 0, 0, 0, 0, 0, 0, 0]),
            ("text before the first path", with(first, 1)),
            ("text in an empty blob", blob([0, 0, 0], &[1], b"a")),
            ("count past the offsets", with(0, 3)),
            ("count past usize", with(0, u32::MAX)),
            ("left-out count past usize", with(4, u32::MAX)),
            ("peer count past usize", with(8, u32::MAX)),
            ("an offset past the text", with(first + 4 * 4, 12)),
            ("offsets out of order", with(first + 4 * 2, 3)),
            ("last offset short", with(first + 4 * 4, 10)),
            ("text not UTF-8", not_utf8),
            ("an offset inside a character", split),
            (
                "paths out of order",
                blob([2, 0, 0], &[0, 4, 6, 10, 11], b"b.tsxya.tsz"),
            ),
            (
                "left-out paths out of order",
                blob([0, 2, 0], &[0, 4, 8], b"b.tsa.ts"),
            ),
            (
                "peers out of order",
                blob([0, 0, 2], &[0, 1, 3, 4, 6], b"b^1a^1"),
            ),
            (
                "a path embedded and left out",
                blob([1, 1, 0], &[0, 4, 5, 9], b"a.tsxa.ts"),
            ),
        ];
        for (name, blob) in cases {
            assert!(
                matches!(Embed::decode(&blob), Err(EmbedError::Malformed(_))),
                "{name}"
            );
        }
        assert!(Embed::decode(&good).is_ok());
        assert!(Embed::decode(&blob([1, 1, 1], &[0, 4, 5, 9, 10, 12], b"a.tsxb.tsp^1")).is_ok());
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
        sort_left_out(embedded, host_modules, &[])
    }

    /// A sort as [`sort`] reads, with `node_only` listing `left_out`.
    fn sort_left_out(embedded: &[&str], host_modules: &[&str], left_out: &[&str]) -> SortJson {
        let group = |modules: &[&str]| serde_json::json!({"reason": "r", "modules": modules});
        serde_json::from_value(serde_json::json!({
            "description": "d",
            "embedded": embedded,
            "excluded": {
                "host_modules": group(host_modules),
                "node_roots": group(&[]),
                "node_loader": group(&[]),
                "node_only": group(left_out),
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
        let refused_left_out = |embedded: &[&str], left_out: &[&str]| {
            check_sort(file, &sort_left_out(embedded, &host_modules, left_out)).unwrap_err()
        };
        assert!(matches!(
            refused_left_out(&["a.ts"], &["a.ts"]),
            EmbedError::ListedTwice { .. }
        ));
        assert!(matches!(
            refused_left_out(&[], &["a.js"]),
            EmbedError::Path { .. }
        ));
        check_sort(file, &sort_left_out(&["a.ts"], &host_modules, &["b.ts"])).unwrap();
    }

    /// The peers read, in their names' order, and each refusal of them.
    #[test]
    fn peers_are_read_in_order_and_refused_unread() {
        let file = Path::new("package.json");
        let peers = parse_peers(
            file,
            r#"{"name": "x", "peerDependencies": {"z": "^1", "@a/b": ">=0.83.0", "m": "^5.0.3 || ^6.0.0"}}"#,
        )
        .unwrap();
        assert_eq!(
            peers,
            [
                ("@a/b".to_owned(), ">=0.83.0".to_owned()),
                ("m".to_owned(), "^5.0.3 || ^6.0.0".to_owned()),
                ("z".to_owned(), "^1".to_owned()),
            ]
        );
        assert_eq!(parse_peers(file, r#"{"name": "x"}"#).unwrap(), []);
        for text in [
            "not json",
            r#"{"peerDependencies": []}"#,
            r#"{"peerDependencies": {"a": 1}}"#,
        ] {
            assert!(
                matches!(parse_peers(file, text), Err(EmbedError::Peers { .. })),
                "{text}"
            );
        }
        let Err(EmbedError::PeerRange { name, range, .. }) =
            parse_peers(file, r#"{"peerDependencies": {"a": "^1", "b": "~1.2.3"}}"#)
        else {
            panic!("an unread range was accepted");
        };
        assert_eq!((name.as_str(), range.as_str()), ("b", "~1.2.3"));
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
        let Sorted { embedded, left_out } = read_sort(&src.join(SORT_FILE)).unwrap();
        assert!(!embedded.is_empty());
        assert_eq!(
            embed.iter().map(|(path, _)| path).collect::<Vec<_>>(),
            embedded
        );
        assert!(!left_out.is_empty());
        assert_eq!(embed.left_out().collect::<Vec<_>>(), left_out);
        for path in ["invoke.ts", "loader.ts", "library_load.ts", "build.task.ts"] {
            assert!(embed.is_left_out(path), "{path}");
        }
        // the package's own peers, each read
        assert_eq!(
            embed
                .peers()
                .map(|(name, range)| (name.to_owned(), range.to_owned()))
                .collect::<Vec<_>>(),
            read_peers(&src.join("../package.json")).unwrap()
        );
        for name in ["@fuzdev/fuz_util", "@fuzdev/fuz_schema", "typescript"] {
            assert!(embed.peer_range(name).is_some(), "{name}");
        }
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
