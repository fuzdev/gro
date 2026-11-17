//! The ops that write through an `FsDir`: each checks its paths as it
//! would write them, and a handle that may not write refuses it
//! `not_granted`, recorded with what would let it write
//! ([`scope_needed`]).

use deno_core::{OpState, v8};

use super::walk::scope_needed;
use super::{
    Failure, MODULE, Scope, finish, handle_root, path_text, read_handle, run_roots, run_state,
    to_lexical,
};
use crate::NeededGrant;
use crate::host::error_class::DeclaredFamily;
use crate::host::handle::FS_DIR;

/// The write op `name` called on `receiver`, an `FsDir`, with `paths`, each
/// checked in order as the op would check it, the op writing those from
/// `written` on (a copy's `to`, every path of the others): refused
/// `not_granted` when the handle may not write, naming the first path it
/// writes, recorded as it's thrown with what would let the op write them
/// all ([`scope_needed`]); else a plain `Error` saying the op isn't built
/// on fgro yet.
pub fn write_op<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    name: &str,
    receiver: v8::Local<'s, v8::Value>,
    paths: &[v8::Local<'s, v8::Value>],
    written: usize,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        run_state(state)?.ready()?;
        let handle = read_handle(scope, &FS_DIR, receiver)?;
        let roots = run_roots(state)?;
        let (_, handle_path) = handle_root(&roots, &handle)?;
        let mut targets = Vec::with_capacity(paths.len());
        for path in paths {
            let requested = path_text(scope, *path)?;
            targets.push(to_lexical(&requested, &handle_path)?.path_beneath(&handle_path));
        }
        if handle.scope.write {
            return Err(Failure::Plain(format!(
                "{name} of {MODULE} isn't built on fgro yet"
            )));
        }
        let written = targets.get(written..).unwrap_or_default();
        let needed = scope_needed(&roots, written, Scope::WRITE, NeededGrant::Write);
        let first = written.first().cloned().unwrap_or(handle_path);
        Err::<v8::Local<'s, v8::Value>, _>(Failure::NotGranted {
            path: first,
            needed: Some(needed),
        })
    })();
    finish(state, scope, Some(errors), result)
}
