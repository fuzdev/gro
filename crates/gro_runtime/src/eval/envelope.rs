//! The envelope: the one JSON object `fgro eval` writes, success or failure.

use std::io::{self, Write};

use serde::Serialize;

use super::dump::GraphDump;
use super::limits::format_size;
use super::{DEFAULT_EXPORT, EvalOptions, MAX_ENVELOPE_SIZE};
use crate::loader::graph::{FailureReason, Graph};
use crate::resolver::Resolution;

/// The envelope's protocol version, its first field. It moves with any change
/// to the envelope that a reader must know about: its shape, or a value a
/// closed set takes, such as a new error kind.
pub const EVAL_PROTOCOL: u32 = 2;

/// The `deno_core` version the runtime is built on, as `Cargo.lock` pins it,
/// which a test checks.
pub const DENO_CORE_VERSION: &str = "0.412.0";

/// The tsv commit the type strip is built from: the one the `tsv_ts` release
/// at [`TSV_VERSION`] was published from (its crate's `.cargo_vcs_info.json`).
pub const TSV_REV: &str = "2c9119cd2773de89299cdf077fed0c8d40995ead";

/// The `tsv_ts` version the type strip is built from, as `Cargo.lock` pins it,
/// which a test checks; [`TSV_REV`] moves with it.
pub const TSV_VERSION: &str = "0.8.1";

/// What `fgro eval` wrote: the runtime and entry, the value or the error, and
/// what the run tried to reach and was refused.
#[derive(Debug, Clone)]
pub struct Envelope {
    /// The runtime that evaluated, for display.
    pub runtime: RuntimeInfo,
    /// What was evaluated.
    pub entry: Entry,
    /// The value as compact JSON text, or why there is none.
    pub result: Result<String, EvalError>,
    /// What the run tried to reach and was refused; unset only in the envelope
    /// the out-of-memory backstop writes, prepared before the run and written
    /// without it, and in one whose text would pass [`MAX_ENVELOPE_SIZE`]: a
    /// value's, replaced by a small failure, or a failure's, which drops it.
    pub access: Option<Access>,
    /// The module graph, when the caller asked for it.
    pub graph: GraphField,
}

/// Whether an envelope carries the module graph, and what it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphField {
    /// Not asked for: the envelope has no `graph` field.
    Omitted,
    /// Asked for, with no graph to write: `null`. The run never reached its
    /// module graph (the entry didn't resolve, the box didn't go up, or the
    /// process ran out of memory and wrote its prepared envelope), or the graph
    /// names a path no id can: one that isn't UTF-8, or a module URL with no
    /// path ([`super::DumpError`]). A graph the host failed to record fails the
    /// eval as `internal`, and is `null` too, as is one in an envelope that
    /// would pass [`MAX_ENVELOPE_SIZE`], which drops it.
    Unavailable,
    /// The graph, from the run as far as it got.
    Dumped(Box<GraphDump>),
}

impl GraphField {
    /// The field with its graph dropped, as an envelope written without it
    /// carries: not asked for stays [`GraphField::Omitted`], and anything
    /// else is [`GraphField::Unavailable`].
    #[must_use]
    pub(super) const fn dropped(&self) -> Self {
        match self {
            Self::Omitted => Self::Omitted,
            Self::Unavailable | Self::Dumped(_) => Self::Unavailable,
        }
    }
}

/// The runtime that evaluated, for display only: two binaries can share
/// every field, so none identifies one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeInfo {
    /// The binary's version.
    pub fgro_version: String,
    /// The git commit it was built from, unset when the build embedded none.
    pub commit: Option<String>,
    /// Whether the working tree differed from the commit, unset with it.
    pub dirty: Option<bool>,
    /// The V8 version, as V8 reports it.
    pub v8: String,
    /// The `deno_core` version.
    pub deno_core: String,
    /// The tsv commit the type strip is built from.
    pub tsv_rev: String,
}

impl RuntimeInfo {
    /// The runtime of the binary at `fgro_version`, built from `commit` (a
    /// SHA, with `-dirty` appended when the working tree differed), or from
    /// no known commit.
    pub fn new(fgro_version: &str, commit: Option<&str>) -> Self {
        let (commit, dirty) = commit.map_or((None, None), |commit| {
            commit.strip_suffix("-dirty").map_or_else(
                || (Some(commit.to_owned()), Some(false)),
                |sha| (Some(sha.to_owned()), Some(true)),
            )
        });
        Self {
            fgro_version: fgro_version.to_owned(),
            commit,
            dirty,
            v8: crate::v8_version().to_owned(),
            deno_core: DENO_CORE_VERSION.to_owned(),
            tsv_rev: TSV_REV.to_owned(),
        }
    }
}

/// What was evaluated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// The entry module: its canonical path once it resolved, else the path
    /// as given.
    pub id: String,
    /// The export serialized, with no driver; unset with one, which is handed
    /// the whole namespace.
    pub export: Option<String>,
    /// The BLAKE3 hash of the driver's bytes as read from stdin, in hex;
    /// unset with no driver.
    pub driver_blake3: Option<String>,
}

/// The kind of failure, the contract a reader branches on.
///
/// A kind is derived from the step that failed and the failures the
/// loader recorded, never from a message, and a failure past the
/// envelope's cap keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The entry didn't resolve or isn't a project's own module (inside
    /// `node_modules`, or with no project root), or a static import didn't
    /// resolve: the first failure of the graph's static load was a
    /// resolution's (an import from the driver among them).
    Resolve,
    /// The graph didn't form with every import resolved: a syntax error, an
    /// import of a name a module doesn't export, TypeScript the strip
    /// refuses, import attributes, or a module past
    /// [`MAX_MODULE_SIZE`](crate::resolver::MAX_MODULE_SIZE).
    Link,
    /// A module's evaluation threw or rejected, or left a top-level `await`
    /// unsettled, or the driver's call did.
    ModuleThrew,
    /// The export the host reads is absent: with no driver, the one named
    /// (`default` unless `--export` names another); with one, the driver's
    /// `default`, or an `after_drain` that isn't a function.
    ExportMissing,
    /// The value isn't JSON data by the host serializer's rules, or the
    /// envelope holding it would pass [`MAX_ENVELOPE_SIZE`]. A failure past the
    /// cap keeps its kind.
    Serialize,
    /// After the value was serialized, the event loop's remaining work
    /// threw or rejected, or the driver's `after_drain` threw.
    LateRejection,
    /// The heap can grow no more under a data limit that leaves room for its
    /// cap: the run was stopped, or, out of memory, the process wrote a
    /// prepared envelope and exited.
    Heap,
    /// The process ran out of memory outside the heap's cap, at its cap on
    /// memory ([`MEMORY_LIMIT`](super::MEMORY_LIMIT)) or a lower one its
    /// caller set: an array buffer's allocation failed, or the heap stopped
    /// growing under a limit with no room for its cap (the message names
    /// both), and the run was stopped; or, out of memory with the heap not at
    /// fault (V8 ran out outside its heap, or on the heap's pages before it
    /// stopped the heap near its cap), the process wrote a prepared envelope
    /// and exited.
    Memory,
    /// The run passed its timeout and was stopped.
    Timeout,
    /// The process's environment can't give the same result everywhere, or
    /// can't hold the eval: its time zone isn't UTC, the descriptors it
    /// inherited can't be closed, its environment can't be cleared or its
    /// memory capped, or the host can't hold the box.
    Environment,
    /// The host failed: a module replaced on disk between resolving and
    /// loading, a read failed, a read outside the box's roots was refused,
    /// the build has no snapshot, the graph names a `package.json` with no
    /// read of it kept, or the host broke one of its own invariants.
    Internal,
}

/// Why there is no value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvalError {
    /// The kind of failure.
    pub kind: ErrorKind,
    /// What failed, for a person: an uncaught error's report with its frames,
    /// or the host's message.
    pub message: String,
    /// Where: the JS path to the value that didn't serialize (`$.a[2].b`),
    /// or the `url:line:column` of a load failure when one is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// The specifier of the import that failed: as written for a resolution,
    /// the resolved URL for a load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specifier: Option<String>,
    /// The URL of the module that imported it; for an entry that didn't
    /// resolve, the working directory it was resolved from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importer: Option<String>,
    /// What to change, when the host can tell more than the message says:
    /// a fix for a person, apart from the message, which stays the error's
    /// own report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl EvalError {
    /// An error of `kind` with `message` and nothing else.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            at: None,
            specifier: None,
            importer: None,
            hint: None,
        }
    }
}

/// What the run tried to reach and was refused, listed whether or not the
/// code caught the refusal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Access {
    /// Every `import()` the config's graph made, each refused, in
    /// specifier order, then importer order, one entry per specifier and
    /// importer. The driver's are refused as imports from the driver, and
    /// not listed.
    pub dynamic: Vec<DynamicImport>,
}

/// An `import()`, refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DynamicImport {
    /// The specifier as written.
    pub specifier: String,
    /// The URL of the module that called it.
    pub importer: String,
}

impl Envelope {
    /// The envelope of an eval of `options` that failed with `error` before
    /// its entry resolved, such as when the driver couldn't be read: the
    /// entry as given.
    pub fn failed(options: &EvalOptions, error: EvalError) -> Self {
        Self {
            runtime: options.runtime.clone(),
            entry: Entry {
                id: options.entry.to_string_lossy().into_owned(),
                export: options.driver.is_none().then(|| {
                    options
                        .export
                        .clone()
                        .unwrap_or_else(|| DEFAULT_EXPORT.to_owned())
                }),
                driver_blake3: options
                    .driver
                    .as_ref()
                    .map(|driver| blake3::hash(driver).to_hex().to_string()),
            },
            result: Err(error),
            access: Some(Access::default()),
            graph: if options.graph {
                GraphField::Unavailable
            } else {
                GraphField::Omitted
            },
        }
    }

    /// The envelope of an eval of `options` failed with `error` once its
    /// entry resolved to `entry`, named by its canonical path.
    pub(super) fn failed_resolved(
        options: &EvalOptions,
        entry: &Resolution,
        error: EvalError,
    ) -> Self {
        let mut envelope = Self::failed(options, error);
        envelope.entry.id = entry_id(entry);
        envelope
    }

    /// Whether there is a value.
    pub const fn is_ok(&self) -> bool {
        self.result.is_ok()
    }

    /// The envelope as one line of compact JSON, its fields in this order:
    /// `eval_protocol`, `ok`, `runtime`, `entry`, then `value` or `error`,
    /// then `access` (`null` when unset), then `graph` when it was asked
    /// for (`null` when unavailable). The value is spliced in as the
    /// serializer wrote it.
    ///
    /// # Errors
    ///
    /// Returns serde's error when a part fails to serialize, which none of
    /// these types can.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let value = self.result.as_deref().map_or(0, str::len);
        let mut out = Vec::with_capacity(value + 1024);
        self.write_json(&mut out)?;
        // serde writes UTF-8, and the value is a `String`
        String::from_utf8(out).map_err(|error| {
            serde_json::Error::io(io::Error::new(io::ErrorKind::InvalidData, error))
        })
    }

    /// The length of [`Envelope::to_json`]'s text in bytes, counted as
    /// it's written, so no part of it is held: an envelope too large to
    /// write costs no memory to measure.
    ///
    /// # Errors
    ///
    /// As [`Envelope::to_json`].
    fn json_len(&self) -> serde_json::Result<usize> {
        let mut counter = Counter(0);
        self.write_json(&mut counter)?;
        Ok(counter.0)
    }

    /// Writes [`Envelope::to_json`]'s text to `out`.
    fn write_json(&self, out: &mut impl Write) -> serde_json::Result<()> {
        write!(
            out,
            "{{\"eval_protocol\":{EVAL_PROTOCOL},\"ok\":{},\"runtime\":",
            self.is_ok()
        )
        .map_err(serde_json::Error::io)?;
        serde_json::to_writer(&mut *out, &self.runtime)?;
        raw(out, ",\"entry\":")?;
        serde_json::to_writer(&mut *out, &self.entry)?;
        match &self.result {
            Ok(value) => {
                raw(out, ",\"value\":")?;
                raw(out, value)?;
            }
            Err(error) => {
                raw(out, ",\"error\":")?;
                serde_json::to_writer(&mut *out, error)?;
            }
        }
        raw(out, ",\"access\":")?;
        serde_json::to_writer(&mut *out, &self.access)?;
        match &self.graph {
            GraphField::Omitted => {}
            GraphField::Unavailable => raw(out, ",\"graph\":null")?,
            GraphField::Dumped(graph) => {
                raw(out, ",\"graph\":")?;
                serde_json::to_writer(&mut *out, graph)?;
            }
        }
        raw(out, "}")
    }
}

/// The entry's canonical path, as the envelope names it.
pub(super) fn entry_id(entry: &Resolution) -> String {
    entry.url.to_file_path().map_or_else(
        |()| entry.url.to_string(),
        |path| path.to_string_lossy().into_owned(),
    )
}

/// The run's [`Access`]: every `import()` `graph` refused, in specifier
/// order, then importer order, one entry per specifier and importer; the
/// driver's are recorded as driver imports, not listed. Read on every run,
/// a success's included.
pub(super) fn access(graph: &Graph) -> Access {
    let dynamic = graph
        .failures()
        .iter()
        .filter(|(_, failures)| {
            failures
                .iter()
                .any(|failure| failure.reason == FailureReason::DynamicImport)
        })
        .filter_map(|(site, _)| {
            Some(DynamicImport {
                specifier: site.specifier.clone(),
                importer: site.importer.clone()?,
            })
        })
        .collect();
    Access { dynamic }
}

/// Writes `text` to `out` as it is.
fn raw(out: &mut impl Write, text: &str) -> serde_json::Result<()> {
    out.write_all(text.as_bytes())
        .map_err(serde_json::Error::io)
}

/// A writer that keeps only how many bytes it was given.
struct Counter(usize);

impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The bytes of each text field of a failure past the envelope's cap that
/// are kept (`message`, `at`, `specifier`, `importer`, and `hint`): 64 KiB.
///
/// Cut to this, a failure's five fields write at most six bytes of JSON
/// for each byte they keep (a control character's `\u` escape), with the
/// note naming each one's length, under 2 MiB in all, so a failure cut fits
/// the envelope's cap with what's left of it: what the run can't make, the
/// `runtime` and `entry` the caller passed, which `fgro` bounds by its
/// command line (each argument at the kernel's 128 KiB).
pub(super) const MAX_TEXT_KEPT: usize = 64 << 10;

const _: () = assert!(5 * 6 * (MAX_TEXT_KEPT + 32) < MAX_ENVELOPE_SIZE / 32);

/// Brings `envelope`, when its text would pass `max` bytes, back under it as
/// far as dropping and cutting can, which at `fgro`'s sizes always does
/// ([`MAX_TEXT_KEPT`]). A value is replaced with a small `serialize` failure
/// naming the size. A failure keeps its kind: its `access` and graph are
/// dropped (`null`, as in the out-of-memory backstop's envelope), and if it's
/// still past `max`, each of its text fields longer than `kept` bytes is cut to
/// them, at a character boundary, and named by its length (`aaa… (70000
/// bytes)`). The value is capped as it's written, but what's around it isn't
/// (an error's message or specifier, the specifiers `access.dynamic` lists, the
/// graph), and the code that ran can make any of them as long as its heap
/// holds.
pub(super) fn cap_envelope(envelope: &mut Envelope, max: usize, kept: usize) {
    // counted, not written: the text past the cap is never held
    let fits = |envelope: &Envelope| envelope.json_len().is_ok_and(|size| size <= max);
    let size = match envelope.json_len() {
        Ok(size) if size <= max => return,
        Ok(size) => Some(size),
        Err(_) => None,
    };
    if envelope.result.is_ok() {
        let size = size.map_or_else(|| "too large".to_owned(), |size| format!("{size} bytes"));
        envelope.result = Err(EvalError::new(
            ErrorKind::Serialize,
            format!(
                "the envelope would be {size}, past {}, the envelope's cap, so the \
                 value isn't written; write less data, and keep error messages and import \
                 specifiers short",
                format_size(max as u64)
            ),
        ));
    }
    envelope.access = None;
    envelope.graph = envelope.graph.dropped();
    if fits(envelope) {
        return;
    }
    if let Err(error) = &mut envelope.result {
        cut_text(&mut error.message, kept);
        let fields = [
            &mut error.at,
            &mut error.specifier,
            &mut error.importer,
            &mut error.hint,
        ];
        for text in fields.into_iter().flatten() {
            cut_text(text, kept);
        }
    }
}

/// Cuts `text`, when it's longer than `kept` bytes, to at most `kept`, at a
/// character boundary, ending in `…` and its length in bytes.
fn cut_text(text: &mut String, kept: usize) {
    use std::fmt::Write as _;
    let length = text.len();
    if length <= kept {
        return;
    }
    let mut end = kept;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    let _ = write!(text, "… ({length} bytes)");
    text.shrink_to_fit();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(result: Result<String, EvalError>) -> Envelope {
        Envelope {
            runtime: RuntimeInfo {
                fgro_version: "1.2.3".to_owned(),
                commit: None,
                dirty: None,
                v8: "9.9".to_owned(),
                deno_core: "0.1.0".to_owned(),
                tsv_rev: "abc".to_owned(),
            },
            entry: Entry {
                id: "/p/main.ts".to_owned(),
                export: Some("default".to_owned()),
                driver_blake3: None,
            },
            result,
            access: Some(Access {
                dynamic: vec![DynamicImport {
                    specifier: "./x.js".to_owned(),
                    importer: "file:///p/main.ts".to_owned(),
                }],
            }),
            graph: GraphField::Omitted,
        }
    }

    #[test]
    fn the_envelope_is_one_object_in_its_field_order() {
        let ok = envelope(Ok(r#"{"a":[1,"x"]}"#.to_owned()))
            .to_json()
            .unwrap();
        assert_eq!(
            ok,
            concat!(
                r#"{"eval_protocol":2,"ok":true,"runtime":{"fgro_version":"1.2.3","commit":null,"#,
                r#""dirty":null,"v8":"9.9","deno_core":"0.1.0","tsv_rev":"abc"},"entry":"#,
                r#"{"id":"/p/main.ts","export":"default","driver_blake3":null},"value":"#,
                r#"{"a":[1,"x"]},"access":{"dynamic":[{"specifier":"./x.js","#,
                r#""importer":"file:///p/main.ts"}]}}"#,
            )
        );
        let mut error = EvalError::new(ErrorKind::LateRejection, "late");
        error.at = Some("$".to_owned());
        let failed = envelope(Err(error)).to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&failed).unwrap();
        assert_eq!(parsed["ok"], false);
        assert_eq!(
            parsed["error"],
            serde_json::json!({"kind": "late_rejection", "message": "late", "at": "$"})
        );
        assert!(parsed.get("value").is_none());
        // a hint is written when there is one, and an unset access as null
        let mut error = EvalError::new(ErrorKind::Timeout, "slow");
        error.hint = Some("faster".to_owned());
        let mut unset = envelope(Err(error));
        unset.access = None;
        let parsed: serde_json::Value = serde_json::from_str(&unset.to_json().unwrap()).unwrap();
        assert_eq!(
            parsed["error"],
            serde_json::json!({"kind": "timeout", "message": "slow", "hint": "faster"})
        );
        assert_eq!(parsed["access"], serde_json::Value::Null);
        assert!(parsed.get("graph").is_none());
        // asked for, the graph is last, null when there is none
        let mut unavailable = envelope(Ok("1".to_owned()));
        unavailable.graph = GraphField::Unavailable;
        assert!(
            unavailable
                .to_json()
                .unwrap()
                .ends_with(r#""access":{"dynamic":[{"specifier":"./x.js","importer":"file:///p/main.ts"}]},"graph":null}"#)
        );
    }

    /// The envelope's length is its text's, counted without writing the
    /// value into it.
    #[test]
    fn the_length_is_the_texts() {
        let mut error = EvalError::new(ErrorKind::Serialize, "é");
        error.at = Some("$".to_owned());
        let mut graphless = envelope(Ok("1".to_owned()));
        graphless.graph = GraphField::Unavailable;
        let mut dumped = envelope(Ok("1".to_owned()));
        dumped.graph = GraphField::Dumped(Box::new(GraphDump {
            graph_version: super::super::GRAPH_VERSION,
            roots: vec!["project:main.ts".to_owned()],
            modules: Vec::new(),
            consulted: Vec::new(),
            absent: vec!["project:é/package.json".to_owned()],
            local: super::super::DumpLocal {
                project: "/p".to_owned(),
                box_roots: vec!["/p".to_owned()],
                paths: [("project:main.ts".to_owned(), "/p/main.ts".to_owned())].into(),
            },
        }));
        for envelope in [
            envelope(Ok(r#"{"a":"\u00e9 é"}"#.to_owned())),
            envelope(Ok(String::new())),
            envelope(Err(error)),
            graphless,
            dumped,
        ] {
            assert_eq!(
                envelope.json_len().unwrap(),
                envelope.to_json().unwrap().len()
            );
        }
    }

    #[test]
    fn the_commit_splits_off_its_dirty_suffix() {
        let info = RuntimeInfo::new("0.0.0", Some("abc-dirty"));
        assert_eq!(
            (info.commit.as_deref(), info.dirty),
            (Some("abc"), Some(true))
        );
        let info = RuntimeInfo::new("0.0.0", Some("abc"));
        assert_eq!(
            (info.commit.as_deref(), info.dirty),
            (Some("abc"), Some(false))
        );
        let info = RuntimeInfo::new("0.0.0", None);
        assert_eq!((info.commit, info.dirty), (None, None));
    }

    /// The pinned versions the envelope reports are the ones `Cargo.lock`
    /// resolves, so a bump that forgets them fails here.
    #[test]
    fn the_reported_versions_match_the_lockfile() {
        let lock =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
                .unwrap();
        let packages: Vec<&str> = lock.split("[[package]]").collect();
        let package = |name: &str| {
            packages
                .iter()
                .find(|package| package.contains(&format!("name = \"{name}\"\n")))
                .copied()
                .unwrap()
        };
        assert!(
            package("deno_core").contains(&format!("version = \"{DENO_CORE_VERSION}\"\n")),
            "{}",
            package("deno_core")
        );
        assert!(
            package("tsv_ts").contains(&format!("version = \"{TSV_VERSION}\"\n")),
            "{}",
            package("tsv_ts")
        );
    }

    /// `TSV_REV` is the commit the resolved `tsv_ts` was published from, as
    /// its registry source's `.cargo_vcs_info.json` records it, so a bump that
    /// moves `TSV_VERSION` and forgets the commit fails here.
    #[test]
    fn the_reported_tsv_rev_is_the_published_commit() {
        let output = std::process::Command::new(env!("CARGO"))
            .args(["metadata", "--format-version", "1", "--locked", "--offline"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let manifest = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == "tsv_ts")
            .and_then(|package| package["manifest_path"].as_str())
            .unwrap();
        let vcs_info = std::path::Path::new(manifest).with_file_name(".cargo_vcs_info.json");
        let vcs_info: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&vcs_info).unwrap()).unwrap();
        assert_eq!(vcs_info["git"]["sha1"], TSV_REV);
    }
}

#[cfg(test)]
mod cap_tests {
    use super::super::failure::stop_error;
    use super::super::limits::Stop;
    use super::super::{DEFAULT_EXPORT, options};
    use super::*;
    use std::path::Path;

    /// An envelope with `result` and `access`, and the graph `graph`.
    fn envelope(
        result: Result<String, EvalError>,
        dynamic: &[&str],
        graph: GraphField,
    ) -> Envelope {
        Envelope {
            runtime: RuntimeInfo::new("0.0.0", None),
            entry: Entry {
                id: "/p/main.js".to_owned(),
                export: Some(DEFAULT_EXPORT.to_owned()),
                driver_blake3: None,
            },
            result,
            access: Some(Access {
                dynamic: dynamic
                    .iter()
                    .map(|specifier| DynamicImport {
                        specifier: (*specifier).to_owned(),
                        importer: "file:///p/main.js".to_owned(),
                    })
                    .collect(),
            }),
            graph,
        }
    }

    /// A value past the cap, or one whose envelope passes it with the
    /// specifiers `access.dynamic` lists (two caught `import()` calls of
    /// long specifiers), is replaced with a small `serialize` failure naming
    /// the size it would have been, with no `access` and no graph (`null`
    /// when it was asked for). One at the cap is kept.
    #[test]
    fn a_value_past_the_cap_fails_as_serialize() {
        let long = "a".repeat(4096);
        let dynamic = format!("./{long}");
        let cases = [
            envelope(Ok(format!("\"{long}\"")), &[], GraphField::Omitted),
            envelope(
                Ok("1".to_owned()),
                &[
                    format!("{dynamic}0").as_str(),
                    format!("{dynamic}1").as_str(),
                ],
                GraphField::Unavailable,
            ),
        ];
        for original in cases {
            let size = original.json_len().unwrap();
            let mut kept = original.clone();
            cap_envelope(&mut kept, size, 16);
            assert_eq!(kept.to_json().unwrap(), original.to_json().unwrap());
            let mut capped = original.clone();
            cap_envelope(&mut capped, size - 1, 16);
            let error = capped.result.as_ref().unwrap_err();
            assert_eq!(error.kind, ErrorKind::Serialize);
            assert_eq!(
                error.message,
                format!(
                    "the envelope would be {size} bytes, past {} bytes, the envelope's cap, so \
                     the value isn't written; write less data, and keep error messages and \
                     import specifiers short",
                    size - 1
                )
            );
            assert_eq!(
                (&error.at, &error.specifier, &error.importer, &error.hint),
                (&None, &None, &None, &None)
            );
            assert_eq!(capped.access, None);
            let graph = match original.graph {
                GraphField::Omitted => GraphField::Omitted,
                _ => GraphField::Unavailable,
            };
            assert_eq!(capped.graph, graph);
            assert!(capped.json_len().unwrap() < 1024);
        }
    }

    /// A failure past the cap keeps its kind and its error: a run stopped
    /// at the heap's cap after caught `import()` calls of long specifiers
    /// loses only its `access` and graph, when that's enough.
    #[test]
    fn a_failure_past_the_cap_drops_access_first() {
        let long = "a".repeat(4096);
        let dynamic = [format!("./{long}0"), format!("./{long}1")];
        let dynamic: Vec<&str> = dynamic.iter().map(String::as_str).collect();
        let stopped = || {
            stop_error(
                Stop::Heap,
                &options(Path::new("/p/main.js"), Path::new("/p")),
            )
        };
        let original = envelope(Err(stopped()), &dynamic, GraphField::Unavailable);
        let size = original.json_len().unwrap();
        let mut kept = original.clone();
        cap_envelope(&mut kept, size, 16);
        assert_eq!(kept.to_json().unwrap(), original.to_json().unwrap());
        let mut capped = original;
        cap_envelope(&mut capped, size - 1, 16);
        assert_eq!(capped.result, Err(stopped()));
        assert_eq!(capped.access, None);
        assert_eq!(capped.graph, GraphField::Unavailable);
        assert!(capped.json_len().unwrap() < 1024);
    }

    /// A failure still past the cap without its `access` keeps its kind,
    /// with each text field longer than the bytes kept cut to them at a
    /// character boundary and named by its length: a long late rejection,
    /// and an uncaught `import()` of a long specifier.
    #[test]
    fn a_failure_past_the_cap_cuts_its_text() {
        // the 16th byte is inside the é, so the cut keeps 15
        let long = format!("{}é{}", "b".repeat(15), "b".repeat(4096));
        let cut = format!("{}… ({} bytes)", "b".repeat(15), long.len());
        let late = EvalError::new(ErrorKind::LateRejection, format!("Uncaught {long}"));
        let mut uncaught = EvalError::new(ErrorKind::ModuleThrew, "TypeError: cannot import");
        uncaught.at = Some(long.clone());
        uncaught.specifier = Some(long.clone());
        uncaught.importer = Some(long.clone());
        uncaught.hint = Some(long);
        for (error, dynamic) in [(late, &[][..]), (uncaught, &["./x.js"][..])] {
            let mut capped = envelope(Err(error.clone()), dynamic, GraphField::Omitted);
            assert!(capped.json_len().unwrap() > 4096);
            cap_envelope(&mut capped, 1024, 16);
            let capped_error = capped.result.as_ref().unwrap_err();
            assert_eq!(capped_error.kind, error.kind);
            let expected = |text: &str| {
                if text.len() <= 16 {
                    text.to_owned()
                } else {
                    let mut end = 16;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}… ({} bytes)", &text[..end], text.len())
                }
            };
            assert_eq!(capped_error.message, expected(&error.message));
            for (field, original) in [
                (&capped_error.at, &error.at),
                (&capped_error.specifier, &error.specifier),
                (&capped_error.importer, &error.importer),
                (&capped_error.hint, &error.hint),
            ] {
                assert_eq!(
                    field.as_deref(),
                    original.as_deref().map(expected).as_deref()
                );
                if original.is_some() {
                    assert_eq!(field.as_deref(), Some(cut.as_str()));
                }
            }
            assert_eq!(capped.access, None);
            assert!(capped.json_len().unwrap() <= 1024);
        }
    }

    /// At the real sizes, a failure cut to [`MAX_TEXT_KEPT`] fits the cap
    /// however its text escapes: every field a control character, each
    /// written as six bytes.
    #[test]
    fn a_cut_failure_fits_the_cap() {
        let long = "\u{1}".repeat(MAX_TEXT_KEPT * 4);
        let mut error = EvalError::new(ErrorKind::LateRejection, long.clone());
        error.at = Some(long.clone());
        error.specifier = Some(long.clone());
        error.importer = Some(long.clone());
        error.hint = Some(long);
        let mut capped = envelope(Err(error), &[], GraphField::Omitted);
        // past a small cap, so every field is cut
        cap_envelope(&mut capped, 1 << 20, MAX_TEXT_KEPT);
        let size = capped.json_len().unwrap();
        assert!(size > 5 * 6 * MAX_TEXT_KEPT, "{size}");
        assert!(size <= 5 * 6 * (MAX_TEXT_KEPT + 32) + 1024, "{size}");
        assert_eq!(capped.result.unwrap_err().kind, ErrorKind::LateRejection);
    }
}
