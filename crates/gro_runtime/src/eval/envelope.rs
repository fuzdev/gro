//! The envelope: the one JSON object `fgro eval` writes, success or failure.

use serde::Serialize;

/// The envelope's protocol version, its first field. It moves with any change
/// to the envelope's shape that a reader must know about.
pub const EVAL_PROTOCOL: u32 = 1;

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
    /// and written without it.
    pub access: Option<Access>,
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

/// The kind of failure, the contract a reader branches on. A kind is derived
/// from the phase that failed and the failures the loader recorded, never
/// from a message.
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
    /// The value isn't JSON data by the host serializer's rules.
    Serialize,
    /// After the value was serialized, the event loop's remaining work
    /// threw or rejected, or the driver's `after_drain` threw.
    LateRejection,
    /// The heap reached its cap: the run was stopped, or, out of memory,
    /// the process wrote a prepared envelope and exited.
    Heap,
    /// The run passed its timeout and was stopped.
    Timeout,
    /// The process's environment can't give the same result everywhere:
    /// its time zone isn't UTC.
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
    /// then `access` (`null` when unset). The value is spliced in as the
    /// serializer wrote it.
    ///
    /// # Errors
    ///
    /// Returns serde's error when a part fails to serialize, which none of
    /// these types can.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let mut out = format!(
            "{{\"eval_protocol\":{EVAL_PROTOCOL},\"ok\":{},\"runtime\":{},\"entry\":{},",
            self.is_ok(),
            serde_json::to_string(&self.runtime)?,
            serde_json::to_string(&self.entry)?,
        );
        match &self.result {
            Ok(value) => {
                out.push_str("\"value\":");
                out.push_str(value);
            }
            Err(error) => {
                out.push_str("\"error\":");
                out.push_str(&serde_json::to_string(error)?);
            }
        }
        out.push_str(",\"access\":");
        out.push_str(&serde_json::to_string(&self.access)?);
        out.push('}');
        Ok(out)
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
                r#"{"eval_protocol":1,"ok":true,"runtime":{"fgro_version":"1.2.3","commit":null,"#,
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
