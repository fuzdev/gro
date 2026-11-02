//! How an eval fails: the step that failed, its kind and hint, and the
//! errors the host itself raises.

use std::path::Path;

use deno_core::error::{CoreError, CoreErrorKind, JsError};

use super::dump::DumpError;
use super::limits::{Stop, format_size};
use super::{ErrorKind, EvalError, EvalOptions, HEAP_LIMIT, MEMORY_LIMIT};
use crate::confine::Jit;
use crate::loader::graph::{
    Failure, FailureReason, FailureSite, Graph, LoadFailure, Reach, Reached,
};
use crate::report::{report, report_js};

/// The error of an eval whose graph names `path` as consulted with no read
/// of it recorded, which only a bug in the host makes: the graph would name
/// a file with no hash, so the eval fails rather than write one partial or
/// none under a value.
pub(super) fn unread_error(path: &Path) -> EvalError {
    EvalError::new(
        ErrorKind::Internal,
        format!(
            "{}, a bug in fgro, so no graph is written",
            DumpError::Unread(path.to_owned())
        ),
    )
}

/// The error of an eval whose loader was asked to read `path`, outside the
/// box: reached through a link the closure doesn't follow, or a
/// `package.json` symlinked out of the box.
pub(super) fn breach_error(path: &Path) -> EvalError {
    let mut error = EvalError::new(
        ErrorKind::Internal,
        format!(
            "the eval was to read {}, which lies outside its box, so it wasn't read: the box \
             is the project root and each package linked directly into its node_modules (or \
             an @scope directory there) from outside it, followed through each one's own \
             node_modules, except a link to a directory with no package.json file of its \
             own, to a directory holding the project or another package's directory, or to \
             one a world-writable directory holds; a link elsewhere (in a nested \
             node_modules, or a subdirectory's own) or a package.json symlinked out of the \
             box reaches past it",
            path.display()
        ),
    );
    error.hint = Some(BREACH_HINT.to_owned());
    error
}

/// The hint for a read outside the box.
const BREACH_HINT: &str = "link the package's own directory directly into the project's \
                           node_modules, not a directory holding the project or another \
                           package, and keep each package.json a file rather than a symlink";

/// The error of a run the host stopped.
pub(super) fn stop_error(stop: Stop, options: &EvalOptions) -> EvalError {
    match stop {
        Stop::Heap => EvalError::new(
            ErrorKind::Heap,
            format!(
                "the eval's heap reached its cap of {}, and the run was stopped",
                format_size(HEAP_LIMIT as u64)
            ),
        ),
        Stop::Memory => EvalError::new(
            ErrorKind::Memory,
            format!(
                "the eval ran out of memory outside the heap's cap: an allocation failed with \
                 the process at its memory cap of {}, or a lower one its caller set, and \
                 the run was stopped",
                format_size(MEMORY_LIMIT)
            ),
        ),
        Stop::Timeout => {
            let mut error = EvalError::new(
                ErrorKind::Timeout,
                format!(
                    "the eval ran past its timeout of {} ms, and the run was stopped",
                    options.timeout.as_millis()
                ),
            );
            if options.jit == Jit::Off {
                error.hint = Some(TIMEOUT_HINT.to_owned());
            }
            error
        }
    }
}

/// The error of a confined eval with no startup snapshot, an `internal`.
///
/// Started from source, its runtime would read its extensions' JS from the
/// build tree, outside the box. [`eval`](super::eval) refuses one, and a
/// caller that confines refuses one with this before it confines, whatever
/// the host.
pub fn no_snapshot_error() -> EvalError {
    let mut error = EvalError::new(
        ErrorKind::Internal,
        "a confined eval starts only from a startup snapshot embedded in the binary, and this \
         build has none: started from source, the runtime reads its extensions' JS from the \
         build tree, outside the box",
    );
    error.hint = Some(NO_SNAPSHOT_HINT.to_owned());
    error
}

/// The hint for a confined eval with no startup snapshot.
const NO_SNAPSHOT_HINT: &str = "build fgro with its snapshot: cargo xtask build";

/// The hint for a `WebAssembly` that isn't defined in a jitless eval.
const WASM_HINT: &str = "eval runs jitless unless asked, and WebAssembly comes only with V8's \
                         JIT: pass --jit to turn it on";

/// The hint for an uncaught `EvalError`.
const CODE_GENERATION_HINT: &str = "eval turns code generation from strings off, with or \
                                    without --jit: eval(), new Function(), and the other \
                                    function constructors throw an EvalError; write the code \
                                    in the module instead";

/// The hint for a jitless eval that timed out.
const TIMEOUT_HINT: &str = "eval runs jitless unless asked, in V8's interpreter alone: \
                            --jit runs compute-heavy code faster, or pass a longer --timeout";

/// The step of an eval, which decides a failure's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// Starting the runtime.
    Start,
    /// Loading and linking the driver.
    DriverLoad,
    /// Evaluating the driver.
    DriverEvaluate,
    /// Reading the driver's exports.
    DriverExports,
    /// Loading and linking the entry's graph.
    Load,
    /// Evaluating the entry's graph.
    Evaluate,
    /// Reading the export, with no driver.
    Export,
    /// Calling the driver's `default`.
    Call,
    /// Serializing the value.
    Serialize,
    /// Draining the event loop, and calling the driver's `after_drain`.
    Drain,
    /// Checking the process's environment, the time zone, before any code
    /// runs.
    Environment,
    /// The host broke an invariant of its own.
    Internal,
}

/// A failed step: which, the message, and where when the serializer says.
#[derive(Debug)]
pub(super) struct Failed {
    /// The step that failed.
    pub(super) step: Step,
    /// What failed.
    pub(super) message: String,
    /// The JS path the serializer refused.
    pub(super) at: Option<String>,
    /// The uncaught JS error, when one failed the step.
    pub(super) thrown: Option<Thrown>,
}

/// An uncaught JS error, as V8 named it: what the hints read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Thrown {
    /// Its `name`, the class for a native error (`EvalError`).
    name: Option<String>,
    /// Its `message`.
    message: Option<String>,
}

impl Failed {
    pub(super) fn new(step: Step, message: impl Into<String>) -> Self {
        Self {
            step,
            message: message.into(),
            at: None,
            thrown: None,
        }
    }

    /// The failure in `step` that `error` reports.
    pub(super) fn core(step: Step, error: &CoreError) -> Self {
        match &*error.0 {
            CoreErrorKind::Js(error) => Self::js(step, error),
            _ => Self::new(step, report(error)),
        }
    }

    /// The failure in `step` of the uncaught JS `error`.
    pub(super) fn js(step: Step, error: &JsError) -> Self {
        let mut failed = Self::new(step, report_js(error));
        failed.thrown = Some(Thrown {
            name: error.name.clone(),
            message: error.message.clone(),
        });
        failed
    }

    /// The envelope's error: the kind from the step and, for a load, the
    /// first failure of the static load in `graph`, with a hint when the
    /// uncaught error has one in an eval with the JIT as `jit` says.
    pub(super) fn into_error(self, graph: &Graph, jit: Jit) -> EvalError {
        let kind = match self.step {
            Step::Start | Step::Internal => ErrorKind::Internal,
            Step::DriverLoad | Step::Load => {
                return load_error(self.message, first_static_failure(graph));
            }
            Step::DriverEvaluate | Step::Evaluate | Step::Call => ErrorKind::ModuleThrew,
            Step::DriverExports | Step::Export => ErrorKind::ExportMissing,
            Step::Serialize => ErrorKind::Serialize,
            Step::Drain => ErrorKind::LateRejection,
            Step::Environment => ErrorKind::Environment,
        };
        let mut error = EvalError::new(kind, self.message);
        error.at = self.at;
        error.hint = self.thrown.as_ref().and_then(|thrown| hint(thrown, jit));
        error
    }
}

/// The hint for an uncaught `thrown` in an eval with the JIT as `jit`
/// says, keyed on V8's class and, for `WebAssembly`, on the one message V8
/// gives an undefined name, the only signal there is.
fn hint(thrown: &Thrown, jit: Jit) -> Option<String> {
    match (thrown.name.as_deref(), thrown.message.as_deref()) {
        // V8 throws an EvalError natively only for code generation refused
        (Some("EvalError"), _) => Some(CODE_GENERATION_HINT.to_owned()),
        (Some("ReferenceError"), Some("WebAssembly is not defined")) if jit == Jit::Off => {
            Some(WASM_HINT.to_owned())
        }
        _ => None,
    }
}

/// The first failure of a static load: a load records at most one, since
/// `deno_core` stops it at the first error and linking resolves nothing
/// again, and a failed driver load ends the eval before the entry's.
fn first_static_failure(graph: &Graph) -> Option<(&FailureSite, &Failure)> {
    let mut statics = graph
        .failures()
        .iter()
        .filter(|(site, _)| site.reached == Reached::Import(Reach::Static))
        .flat_map(|(site, failures)| failures.iter().map(move |failure| (site, failure)));
    let first = statics.next();
    let second = statics.next();
    // the first by site order is the run's own only while there is one
    debug_assert!(second.is_none(), "a run records at most one static failure");
    first
}

/// The error of a failed load, its kind from the first static failure
/// recorded: a resolution's is `resolve`, unless only a host bug causes it;
/// a load's is `link`, unless only a host bug or a file replaced on disk
/// causes it; none is `link`.
fn load_error(message: String, failure: Option<(&FailureSite, &Failure)>) -> EvalError {
    let Some((site, failure)) = failure else {
        return EvalError::new(ErrorKind::Link, message);
    };
    let kind = match &failure.reason {
        FailureReason::Load(
            LoadFailure::NeverResolved
            | LoadFailure::IdentityMismatch
            | LoadFailure::Read(_)
            | LoadFailure::OutsideBox
            | LoadFailure::NoLocalPath
            | LoadFailure::Format(_),
        )
        | FailureReason::UnknownImporter
        // never static: an `import()` is refused before it loads anything
        | FailureReason::DynamicImport => ErrorKind::Internal,
        // a read that failed or changed, a breach, or a host bug, as the entry's are
        FailureReason::Resolve(kind) if !kind.is_refusal() => ErrorKind::Internal,
        FailureReason::Resolve(_) | FailureReason::DriverImport => ErrorKind::Resolve,
        FailureReason::Load(
            LoadFailure::Strip { .. } | LoadFailure::ImportAttributes | LoadFailure::TooLarge,
        ) => ErrorKind::Link,
    };
    let at = match &failure.reason {
        // a position in the module stripped
        FailureReason::Load(LoadFailure::Strip { at: Some(at), .. }) => {
            Some(format!("{}:{}:{}", site.specifier, at.line, at.column))
        }
        // the import's position in its importer
        _ => failure.at.and_then(|at| {
            site.importer
                .as_ref()
                .map(|importer| format!("{importer}:{}:{}", at.line, at.column))
        }),
    };
    let mut error = EvalError::new(kind, message);
    error.at = at;
    error.specifier = Some(site.specifier.clone());
    error.importer.clone_from(&site.importer);
    error
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};

    use url::Url;

    use super::super::DynamicImport;
    use super::super::envelope::access;
    use super::*;
    use crate::loader::graph::Recording;
    use crate::loader::graph::{Position, StripFailure};
    use crate::resolver::{FileId, Origin, Resolution, ResolveErrorKind, Root, Roots, UrlFault};

    /// A graph of `/p/main.js` with `failures` recorded, each at its own
    /// site and reached as given.
    fn graph(failures: &[(&str, Reached, FailureReason)]) -> Graph {
        let entry = Resolution {
            url: Url::parse("file:///p/main.js").unwrap(),
            file_id: FileId {
                dev: 1,
                ino: 1,
                born: None,
            },
            root: Root {
                dir: PathBuf::from("/p"),
                origin: Origin::Project,
            },
            files_consulted: Vec::new(),
        };
        let mut graph = Graph::new(&entry, Roots::new(PathBuf::from("/p")), Recording::Full);
        for (specifier, reached, error) in failures {
            let failure = Failure {
                at: Some(Position { line: 2, column: 8 }),
                reason: error.clone(),
            };
            graph.fail(specifier, Some("file:///p/main.js"), *reached, failure);
        }
        graph
    }

    const STATIC: Reached = Reached::Import(Reach::Static);
    const DYNAMIC: Reached = Reached::Import(Reach::Dynamic);

    fn load_kind(error: FailureReason) -> ErrorKind {
        Failed::new(Step::Load, "failed")
            .into_error(&graph(&[("./x.js", STATIC, error)]), Jit::Off)
            .kind
    }

    /// A load's kind comes from its static failure's variant, never its
    /// message: a resolution's is `resolve`, a strip refusal, import
    /// attributes, or a module past its byte cap `link`, and what only a
    /// host bug, a file replaced on disk, a failed read, or a read outside
    /// the box causes `internal`, a resolution's included.
    #[test]
    fn a_load_failure_takes_its_kind_from_the_recorded_failure() {
        let not_found = ResolveErrorKind::NotFound { ts_sibling: None };
        for (error, kind) in [
            (
                FailureReason::Resolve(not_found.clone()),
                ErrorKind::Resolve,
            ),
            (FailureReason::DriverImport, ErrorKind::Resolve),
            (
                FailureReason::Resolve(ResolveErrorKind::InvalidUrl(UrlFault::ImporterNoLocalPath)),
                ErrorKind::Internal,
            ),
            (
                FailureReason::Resolve(ResolveErrorKind::ReadReplacedSinceFirstRead(
                    "/p/package.json".into(),
                )),
                ErrorKind::Internal,
            ),
            (
                FailureReason::Resolve(ResolveErrorKind::ReadReplacedSinceLookup(
                    "/p/package.json".into(),
                )),
                ErrorKind::Internal,
            ),
            (
                FailureReason::Load(LoadFailure::Strip {
                    failure: StripFailure::Parse,
                    at: None,
                }),
                ErrorKind::Link,
            ),
            (
                FailureReason::Load(LoadFailure::ImportAttributes),
                ErrorKind::Link,
            ),
            (FailureReason::Load(LoadFailure::TooLarge), ErrorKind::Link),
            (
                FailureReason::Load(LoadFailure::IdentityMismatch),
                ErrorKind::Internal,
            ),
            (
                FailureReason::Load(LoadFailure::Read(io::ErrorKind::PermissionDenied)),
                ErrorKind::Internal,
            ),
            (
                FailureReason::Load(LoadFailure::OutsideBox),
                ErrorKind::Internal,
            ),
            (
                FailureReason::Load(LoadFailure::NeverResolved),
                ErrorKind::Internal,
            ),
            (
                FailureReason::Load(LoadFailure::NoLocalPath),
                ErrorKind::Internal,
            ),
            (
                FailureReason::Load(LoadFailure::Format(not_found)),
                ErrorKind::Internal,
            ),
            (FailureReason::UnknownImporter, ErrorKind::Internal),
        ] {
            assert_eq!(load_kind(error.clone()), kind, "{error:?}");
        }
    }

    /// With no static failure, a failed load is `link` (a syntax error, a
    /// missing export binding), whatever failed dynamically; a failure's
    /// site and position are reported.
    #[test]
    fn a_load_with_no_static_failure_is_link() {
        let dynamic = graph(&[("./y.js", DYNAMIC, FailureReason::DynamicImport)]);
        let error = Failed::new(Step::Load, "SyntaxError").into_error(&dynamic, Jit::Off);
        assert_eq!(
            (error.kind, error.specifier, error.importer, error.at),
            (ErrorKind::Link, None, None, None)
        );
        let error = Failed::new(Step::DriverLoad, "refused").into_error(
            &graph(&[(
                "./x.js",
                STATIC,
                FailureReason::Resolve(ResolveErrorKind::NotFound { ts_sibling: None }),
            )]),
            Jit::Off,
        );
        assert_eq!(error.kind, ErrorKind::Resolve);
        assert_eq!(error.specifier.as_deref(), Some("./x.js"));
        assert_eq!(error.importer.as_deref(), Some("file:///p/main.js"));
        assert_eq!(error.at.as_deref(), Some("file:///p/main.js:2:8"));
    }

    #[test]
    fn every_other_step_has_its_kind() {
        let graph = graph(&[]);
        for (step, kind) in [
            (Step::Start, ErrorKind::Internal),
            (Step::Internal, ErrorKind::Internal),
            (Step::DriverEvaluate, ErrorKind::ModuleThrew),
            (Step::Evaluate, ErrorKind::ModuleThrew),
            (Step::Call, ErrorKind::ModuleThrew),
            (Step::DriverExports, ErrorKind::ExportMissing),
            (Step::Export, ErrorKind::ExportMissing),
            (Step::Serialize, ErrorKind::Serialize),
            (Step::Drain, ErrorKind::LateRejection),
            (Step::Environment, ErrorKind::Environment),
        ] {
            assert_eq!(
                Failed::new(step, "x").into_error(&graph, Jit::Off).kind,
                kind,
                "{step:?}"
            );
        }
    }

    /// `access.dynamic` lists the refused `import()` calls, not other
    /// failures.
    #[test]
    fn access_lists_the_refused_dynamic_imports() {
        let graph = graph(&[
            ("./b.js", DYNAMIC, FailureReason::DynamicImport),
            ("./a.js", DYNAMIC, FailureReason::DynamicImport),
            ("./c.js", STATIC, FailureReason::DriverImport),
        ]);
        let importer = "file:///p/main.js".to_owned();
        assert_eq!(
            access(&graph).dynamic,
            [
                DynamicImport {
                    specifier: "./a.js".to_owned(),
                    importer: importer.clone(),
                },
                DynamicImport {
                    specifier: "./b.js".to_owned(),
                    importer,
                },
            ]
        );
    }

    /// A graph with a consulted file the run didn't record reading is an
    /// `internal` naming the path.
    #[test]
    fn an_unread_consulted_file_is_internal() {
        let error = unread_error(Path::new("/p/package.json"));
        assert_eq!(error.kind, ErrorKind::Internal);
        assert_eq!(
            error.message,
            "the graph names /p/package.json as consulted with no read of it recorded, a bug in \
             fgro, so no graph is written"
        );
    }

    /// A read outside the box names the path, says how the box is made, and
    /// hints at the fix; it is `internal`, not a resolve the code could fix.
    #[test]
    fn a_read_outside_the_box_names_the_path_and_the_box() {
        let error = breach_error(Path::new("/elsewhere/package.json"));
        assert_eq!(error.kind, ErrorKind::Internal);
        assert!(
            error.message.starts_with(
                "the eval was to read /elsewhere/package.json, which lies outside its box"
            ),
            "{}",
            error.message
        );
        assert!(
            error
                .message
                .contains("linked directly into its node_modules")
        );
        assert!(
            error.message.contains(
                "except a link to a directory with no package.json file of its own, to a \
                 directory holding the project or another package's directory"
            ),
            "{}",
            error.message
        );
        assert_eq!(error.hint.as_deref(), Some(BREACH_HINT));
    }
}
