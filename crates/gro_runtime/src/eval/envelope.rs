//! The envelope: the one JSON object `fgro eval` writes, success or failure.

use std::io::{self, Write};

use serde::Serialize;

use super::dump::GraphDump;

/// The envelope's protocol version, its first field. It moves with any change
/// to the envelope that a reader must know about: its shape, or a value a
/// closed set takes, such as a new error kind.
pub const EVAL_PROTOCOL: u32 = 2;

/// The `deno_core` version the runtime is built on, as `Cargo.lock` pins it,
/// which a test checks.
pub const DENO_CORE_VERSION: &str = "0.412.0";

/// The tsv commit the type strip is built from, as `Cargo.lock` pins it,
/// which a test checks.
pub const TSV_REV: &str = "509ba615d1c381548a987a71b60717216d55f839";

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
    /// What the run tried to reach and was refused; unset only in the
    /// envelope the out-of-memory backstop writes, prepared before the run
    /// and written without it, and in one whose text would pass
    /// [`MAX_ENVELOPE_SIZE`](super::MAX_ENVELOPE_SIZE): a value's,
    /// replaced by a small failure, or a failure's, which drops it.
    pub access: Option<Access>,
    /// The module graph, when the caller asked for it.
    pub graph: GraphField,
}

/// Whether an envelope carries the module graph, and what it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphField {
    /// Not asked for: the envelope has no `graph` field.
    Omitted,
    /// Asked for, with no graph to write: `null`. The run never reached
    /// its module graph (the entry didn't resolve, the box didn't go up,
    /// or the process ran out of memory and wrote its prepared envelope),
    /// or the graph names a path no id can: one that isn't UTF-8, or a
    /// module URL with no path ([`super::DumpError`]). A graph the host
    /// failed to record fails the eval as `internal`, and is `null` too, as
    /// is one in an envelope that would pass
    /// [`MAX_ENVELOPE_SIZE`](super::MAX_ENVELOPE_SIZE), which drops it.
    Unavailable,
    /// The graph, from the run as far as it got.
    Dumped(Box<GraphDump>),
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
/// A kind is derived from the phase that failed and the failures the
/// loader recorded, never from a message, and a failure past the
/// envelope's cap keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The entry didn't resolve, or a static import didn't: the first
    /// failure of the graph's static load was a resolution's (an import from
    /// the driver among them).
    Resolve,
    /// The graph didn't form with every import resolved: a syntax error, an
    /// import of a name a module doesn't export, TypeScript the strip
    /// refuses, or import attributes.
    Link,
    /// A module's evaluation threw or rejected, or left a top-level `await`
    /// unsettled, or the driver's call did.
    ModuleThrew,
    /// The export the host reads is absent: with no driver, the one named
    /// (`default` unless `--export` names another); with one, the driver's
    /// `default`, or an `after_drain` that isn't a function.
    ExportMissing,
    /// The value isn't JSON data by the host serializer's rules, or the
    /// envelope holding it would pass
    /// [`MAX_ENVELOPE_SIZE`](super::MAX_ENVELOPE_SIZE). A failure past the
    /// cap keeps its kind.
    Serialize,
    /// After the value was serialized, the event loop's remaining work
    /// threw or rejected, or the driver's `after_drain` threw.
    LateRejection,
    /// The heap reached its cap: the run was stopped, or, out of memory,
    /// the process wrote a prepared envelope and exited.
    Heap,
    /// The process ran out of memory outside the heap's cap, at its cap on
    /// memory ([`MEMORY_LIMIT`](super::MEMORY_LIMIT)) or a lower one its
    /// caller set: an allocation outside the heap failed and the run was
    /// stopped, or, out of memory with the heap not at fault, the process
    /// wrote a prepared envelope and exited.
    Memory,
    /// The run passed its timeout and was stopped.
    Timeout,
    /// The process's environment can't give the same result everywhere, or
    /// can't hold the eval: its time zone isn't UTC, the descriptors it
    /// inherited can't be closed, or the host can't hold the box.
    Environment,
    /// The host failed: a module changed on disk between resolving and
    /// loading, a read failed, or the host broke one of its own invariants.
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
    /// Whether the command line must change: the entry didn't resolve as a
    /// specifier, or it is no project's own module (inside `node_modules`,
    /// or with no project root). Not written; it decides the exit code.
    #[serde(skip)]
    pub usage: bool,
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
            usage: false,
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
    pub fn json_len(&self) -> serde_json::Result<usize> {
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
        error.usage = true;
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
            package("tsv_ts").contains(&format!("?rev={TSV_REV}#{TSV_REV}\"")),
            "{}",
            package("tsv_ts")
        );
    }
}
