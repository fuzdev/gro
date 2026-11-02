//! The eval's pre-pass: the entry resolved and the box's roots computed, reading the disk
//! and starting no runtime.

use std::path::Path;

use super::{Envelope, ErrorKind, EvalError, EvalOptions, engine_options, failed};
use crate::confine;
use crate::loader::reads::{Reads, RecordingTree};
use crate::resolver::{self, DiskTree, EntryBound, Resolution, Roots};

/// An eval resolved, its box computed, before any runtime: what [`prepass`]
/// returns and [`eval`](super::eval) runs.
#[derive(Debug)]
pub struct Prepass {
    /// What to evaluate, and how.
    pub(super) options: EvalOptions,
    /// The entry, resolved.
    pub(super) entry: Resolution,
    /// The roots the entry's graph starts with.
    pub(super) roots: Roots,
    /// What the entry's resolution read and found absent, its graph's
    /// first reads.
    pub(super) reads: Reads,
    /// The box: the project's real directory and the link closure below it.
    pub(super) confinement: confine::Prepared,
}

impl Prepass {
    /// The box the eval's process is to be confined to, before the engine
    /// starts.
    pub const fn confinement(&self) -> &confine::Prepared {
        &self.confinement
    }

    /// Confines the process to the eval's box ([`confine::Prepared::confine`]),
    /// installed for the JIT the eval's engine is to have, read from
    /// [`engine_options`], so the box and the engine take the choice from one
    /// place: jitless, the box refuses executable mappings.
    ///
    /// # Errors
    ///
    /// As [`confine::Prepared::confine`].
    pub fn confine(&self) -> Result<confine::Confined, confine::ConfineError> {
        let jit = confine::Jit::from_jitless(engine_options(self.options.jit).jitless);
        self.confinement.confine(jit)
    }

    /// Whether the eval has a startup snapshot to start from, which a
    /// confined eval needs ([`no_snapshot_error`](super::no_snapshot_error)).
    pub const fn has_snapshot(&self) -> bool {
        self.options.snapshot.is_some()
    }

    /// The envelope of this eval failed with `error` before it ran: the
    /// entry by its canonical path.
    pub fn failed(&self, error: EvalError) -> Envelope {
        let mut envelope = failed(&self.options, error);
        envelope.entry.id = entry_id(&self.entry);
        envelope
    }
}

/// The entry's canonical path, as the envelope names it.
pub(super) fn entry_id(entry: &Resolution) -> String {
    entry.url.to_file_path().map_or_else(
        |()| entry.url.to_string(),
        |path| path.to_string_lossy().into_owned(),
    )
}

/// Resolves the entry of `options` and computes the box its eval runs in,
/// reading the disk and starting no runtime: the first step of the module
/// doc's protocol, the box's roots included.
///
/// # Errors
///
/// Returns the envelope of a failed eval, a `resolve`, when the entry
/// doesn't resolve, is inside `node_modules`, or has no project root, and an
/// `internal` when the project's directory can't be opened as the one
/// resolved ([`confine::Prepared::for_project`]).
pub fn prepass(options: EvalOptions) -> Result<Prepass, Box<Envelope>> {
    let refused = |options: &EvalOptions, message: String, specifier_error: bool| {
        let mut failure = EvalError::new(ErrorKind::Resolve, message);
        failure.usage = specifier_error;
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
            let failure = refused(&options, error.to_string(), error.kind.is_specifier_error());
            return Err(Box::new(failed(&options, failure)));
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
        let failure = refused(&options, message, true);
        let mut envelope = failed(&options, failure);
        envelope.entry.id = entry_id(&entry);
        return Err(Box::new(envelope));
    }
    let confinement = match confine::Prepared::for_project(project) {
        Ok(confinement) => confinement,
        Err(error) => {
            let failure = EvalError::new(
                ErrorKind::Internal,
                format!("failed to open the project's directory for the box: {error}"),
            );
            let mut envelope = failed(&options, failure);
            envelope.entry.id = entry_id(&entry);
            return Err(Box::new(envelope));
        }
    };
    Ok(Prepass {
        options,
        entry,
        roots,
        reads,
        confinement,
    })
}

/// The error of an eval whose process `error` says couldn't be confined: an
/// `environment` when the host can't hold the box, an `internal` when the
/// process broke the order the box needs.
pub fn confine_error(error: &confine::ConfineError) -> EvalError {
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
    use crate::resolver::test_tree::TestDir;

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
            resolved.confinement().roots().as_slice(),
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
            let envelope = prepass(options(Path::new(entry), &project)).unwrap_err();
            let error = envelope.result.as_ref().unwrap_err();
            assert_eq!(error.kind, ErrorKind::Resolve, "{entry}");
            assert!(error.usage, "{entry}");
            assert!(error.message.contains(says), "{}", error.message);
            assert_eq!(error.specifier.as_deref(), Some(entry));
            assert_eq!(
                Path::new(&envelope.entry.id),
                project.join(entry).canonicalize().unwrap()
            );
        }
    }
}
