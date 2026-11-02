//! The eval's pre-pass: the entry resolved and the box's roots computed,
//! reading the disk and starting no runtime.

use std::path::Path;

use super::envelope::entry_id;
use super::{Envelope, ErrorKind, EvalError, EvalOptions};
use crate::confine;
use crate::loader::reads::{Reads, RecordingTree};
use crate::resolver::{self, DiskTree, EntryBound, Resolution, Roots};

/// An eval resolved, its box computed, before any runtime.
///
/// [`prepass`] returns one whose box `B` is the [`confine::Prepared`] roots,
/// opened; once [`Prepass::confine`] has held the process in that box,
/// [`eval`](super::eval) runs one whose box is the [`confine::BoxRoots`] it
/// admits.
#[derive(Debug)]
pub struct Prepass<B = confine::Prepared> {
    /// What to evaluate, and how.
    pub(super) options: EvalOptions,
    /// The entry, resolved.
    pub(super) entry: Resolution,
    /// The roots the entry's graph starts with.
    pub(super) roots: Roots,
    /// What the entry's resolution read and found absent, its graph's
    /// first reads.
    pub(super) reads: Reads,
    /// The box's roots: the project's real directory and the link closure
    /// below it.
    pub(super) box_roots: B,
}

impl Prepass {
    /// Confines the process to the eval's box ([`confine::Prepared::confine`]),
    /// installed for the JIT its options name, the one
    /// [`engine_options`](super::engine_options) gives the engine: jitless,
    /// the box refuses executable mappings. Returns the eval, its box the
    /// roots the box admits, and the confinement the engine starts on.
    ///
    /// # Errors
    ///
    /// Returns the envelope of the eval failed by [`confine::Prepared::confine`]'s
    /// error: an `environment` when the host can't hold the box, an `internal`
    /// when the process broke the order the box needs.
    pub fn confine(self) -> Result<(Prepass<confine::BoxRoots>, confine::Confined), Box<Envelope>> {
        let Self {
            options,
            entry,
            roots,
            reads,
            box_roots,
        } = self;
        let jit = options.jit;
        let boxed = Prepass {
            options,
            entry,
            roots,
            reads,
            box_roots: box_roots.roots().clone(),
        };
        match box_roots.confine(jit) {
            Ok(confined) => Ok((boxed, confined)),
            Err(error) => Err(Box::new(boxed.failed(confine_error(&error)))),
        }
    }
}

impl<B> Prepass<B> {
    /// Whether the eval has a startup snapshot to start from, which a
    /// confined eval needs ([`no_snapshot_error`](super::no_snapshot_error)).
    pub const fn has_snapshot(&self) -> bool {
        self.options.snapshot.is_some()
    }

    /// The envelope of this eval failed with `error` before it ran: the
    /// entry by its canonical path.
    pub fn failed(&self, error: EvalError) -> Envelope {
        Envelope::failed_resolved(&self.options, &self.entry, error)
    }
}

/// A pre-pass that failed, as the envelope that reports it, by whether the
/// command line must change, which decides the exit code.
#[derive(Debug)]
pub enum PrepassError {
    /// The command line must change: the entry didn't resolve as a
    /// specifier, or it is no project's own module (inside `node_modules`,
    /// or with no project root).
    Usage(Box<Envelope>),
    /// Anything else.
    Failed(Box<Envelope>),
}

impl PrepassError {
    /// The envelope that reports the failure.
    pub fn envelope(&self) -> &Envelope {
        match self {
            Self::Usage(envelope) | Self::Failed(envelope) => envelope,
        }
    }
}

/// Resolves the entry of `options` and computes the box its eval runs in,
/// reading the disk and starting no runtime: the first step of the module
/// doc's protocol, the box's roots included.
///
/// # Errors
///
/// Returns the envelope of a failed eval, a `resolve`, when the entry
/// doesn't resolve, is inside `node_modules`, or has no project root (each
/// a [`PrepassError::Usage`]), an `internal` when the resolution failed on
/// the host's side (a read that failed or changed, a symlink loop), and an
/// `internal` when the project's directory can't be opened as the one
/// resolved ([`confine::Prepared::for_project`]).
pub fn prepass(options: EvalOptions) -> Result<Prepass, PrepassError> {
    let refused = |options: &EvalOptions, kind: ErrorKind, message: String| {
        let mut failure = EvalError::new(kind, message);
        failure.specifier = Some(options.entry.display().to_string());
        failure.importer = Some(options.cwd.display().to_string());
        failure
    };
    let mut reads = Reads::default();
    let resolved = resolver::resolve_entry_with_bound(
        &RecordingTree::new(&DiskTree, &mut reads),
        &options.entry,
        &options.cwd,
    );
    let (entry, roots, bound) = match resolved {
        Ok(resolved) => resolved,
        Err(error) => {
            // a read that failed or changed, a breach, or a host bug, as an import's are
            let (kind, wrap): (_, fn(Box<Envelope>) -> PrepassError) = if error.kind.is_refusal() {
                (ErrorKind::Resolve, PrepassError::Usage)
            } else {
                (ErrorKind::Internal, PrepassError::Failed)
            };
            return Err(wrap(Box::new(Envelope::failed(
                &options,
                refused(&options, kind, error.to_string()),
            ))));
        }
    };
    let project = roots.project().dir.clone();
    let refusal = match bound {
        EntryBound::Found => None,
        EntryBound::InNodeModules => Some(format!(
            "cannot evaluate {}: it is inside node_modules, and eval evaluates a project's own \
             module, whose project root bounds what it reads. Fix: evaluate a module of the \
             project, outside node_modules",
            entry_id(&entry)
        )),
        EntryBound::Unbounded { stopped_at } => Some(format!(
            "cannot evaluate {}: it has no project root, which bounds what eval reads: no \
             package.json outside node_modules that holds more than \"type\" and \
             \"sideEffects\" is at or above {}, and the search for one {}. Fix: add a \
             package.json to the project's directory",
            entry_id(&entry),
            project.display(),
            unbounded_end(stopped_at.as_deref())
        )),
    };
    if let Some(message) = refusal {
        let failure = refused(&options, ErrorKind::Resolve, message);
        return Err(PrepassError::Usage(Box::new(Envelope::failed_resolved(
            &options, &entry, failure,
        ))));
    }
    let box_roots = match confine::Prepared::for_project(project) {
        Ok(box_roots) => box_roots,
        Err(error) => {
            let failure = EvalError::new(
                ErrorKind::Internal,
                format!("failed to open the project's directory for the box: {error}"),
            );
            return Err(PrepassError::Failed(Box::new(Envelope::failed_resolved(
                &options, &entry, failure,
            ))));
        }
    };
    Ok(Prepass {
        options,
        entry,
        roots,
        reads,
        box_roots,
    })
}

/// The error of an eval whose process `error` says couldn't be confined: an
/// `environment` when the host can't hold the box, an `internal` when the
/// process broke the order the box needs.
fn confine_error(error: &confine::ConfineError) -> EvalError {
    let kind = if error.is_host_unsupported() {
        ErrorKind::Environment
    } else {
        ErrorKind::Internal
    };
    EvalError::new(kind, format!("cannot confine the eval: {error}"))
}

/// How the walk for a project root ended without one.
fn unbounded_end(stopped_at: Option<&Path>) -> String {
    stopped_at.map_or_else(
        || "reached the filesystem's root".to_owned(),
        |dir| {
            format!(
                "stopped at {}, a directory any user can write, reading nothing in it",
                dir.display()
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::super::options;
    use super::*;
    use crate::test_support::TestDir;

    /// The pre-pass refuses an entry inside `node_modules` and one with no
    /// project root as a `resolve` the command line must fix, naming the
    /// entry by its canonical path; a project's module gets the project's
    /// directory as its box. The temporary directory is world-writable, so
    /// the walk from a directory in it with no `package.json` stops there.
    #[test]
    fn the_prepass_refuses_an_entry_no_project_owns() {
        let test_dir = TestDir::in_system_temp("eval_c1");
        let dir = test_dir.path();
        let files = [
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/main.mjs", ""),
            (
                "project/node_modules/pkg/package.json",
                r#"{"name": "pkg", "exports": "./i.mjs"}"#,
            ),
            ("project/node_modules/pkg/i.mjs", ""),
            ("bare/main.mjs", ""),
        ];
        for (file, content) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let project = dir.join("project");

        let resolved = prepass(options(Path::new("main.mjs"), &project)).unwrap();
        assert_eq!(
            resolved.box_roots.roots().as_slice(),
            std::slice::from_ref(&project)
        );
        assert_eq!(
            resolved
                .failed(EvalError::new(ErrorKind::Internal, "x"))
                .entry
                .id,
            project.join("main.mjs").to_string_lossy()
        );

        for (entry, says) in [
            ("node_modules/pkg/i.mjs", "it is inside node_modules"),
            ("../bare/main.mjs", "it has no project root"),
        ] {
            let refused = prepass(options(Path::new(entry), &project)).unwrap_err();
            assert!(matches!(refused, PrepassError::Usage(_)), "{entry}");
            let envelope = refused.envelope();
            let error = envelope.result.as_ref().unwrap_err();
            assert_eq!(error.kind, ErrorKind::Resolve, "{entry}");
            assert!(error.message.contains(says), "{}", error.message);
            assert_eq!(error.specifier.as_deref(), Some(entry));
            assert_eq!(
                Path::new(&envelope.entry.id),
                project.join(entry).canonicalize().unwrap()
            );
        }
    }

    /// An entry the host fails to read, a symlink loop here, is an
    /// `internal` failure, not a refusal the command line must fix, as the
    /// same failure on an import is.
    #[test]
    fn an_entry_the_host_fails_to_read_is_internal() {
        let test_dir = TestDir::in_system_temp("eval_loop");
        let project = test_dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("package.json"),
            r#"{"name": "p", "type": "module"}"#,
        )
        .unwrap();
        std::os::unix::fs::symlink("b.mjs", project.join("a.mjs")).unwrap();
        std::os::unix::fs::symlink("a.mjs", project.join("b.mjs")).unwrap();

        let failed = prepass(options(Path::new("a.mjs"), &project)).unwrap_err();
        assert!(matches!(failed, PrepassError::Failed(_)));
        let error = failed.envelope().result.as_ref().unwrap_err();
        assert_eq!(error.kind, ErrorKind::Internal, "{}", error.message);
    }
}
