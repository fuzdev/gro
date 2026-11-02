//! Threads with a stated stack: `fgro`'s work runs on one sized by
//! [`STACK_SIZE`], never the platform's main thread.

use gro_runtime::STACK_SIZE;

use crate::cli::Task;
use crate::print::stderr_line;
use crate::{EXIT_PANIC, run};

/// How a thread [`on_sized_thread`] ran ended.
pub enum Joined<T> {
    /// Its work returned this.
    Returned(T),
    /// It couldn't start.
    Unstarted(std::io::Error),
    /// Its work panicked, and the panic has printed.
    Panicked,
}

/// Runs `work` on a thread named `name` with [`STACK_SIZE`] reserved, and
/// joins it.
pub fn on_sized_thread<T: Send>(name: &str, work: impl FnOnce() -> T + Send) -> Joined<T> {
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, work);
        match spawned {
            Ok(handle) => handle.join().map_or(Joined::Panicked, Joined::Returned),
            Err(error) => Joined::Unstarted(error),
        }
    })
}

/// Runs `task` by [`fn@crate::run`] on a thread named `fgro` with [`STACK_SIZE`]
/// reserved, so the runtime, the module loads, and the strip all share the
/// one stated stack: tsv's, so a debug build keeps inside its 16 MiB contract (a release build
/// needs 2 MiB), and the main thread's platform stack (commonly 8 MiB on
/// Linux, 1 MiB on Windows) never decides whether a module within tsv's depth
/// limits strips.
///
/// Not `tsv_ts::run_with_stack`: that names its thread `tsv`, and resumes a
/// panic rather than mapping it to [`EXIT_PANIC`], and drops its work when the
/// spawn is refused.
///
/// A refused spawn runs on this thread instead, and says so, since a module
/// could then overflow the inherited stack with nothing saying why. That
/// rescues only the tasks that don't start V8 (`--version`, `--help`; a
/// usage error is reported before any thread starts): V8 needs threads of
/// its own, so a process that can't start this one can't start the runtime
/// either.
pub fn run_on_sized_stack(task: &Task) -> u8 {
    // `task` is only borrowed, so a refused spawn leaves it to run here
    match on_sized_thread("fgro", || run(task)) {
        // a panic has printed by the time it's joined
        Joined::Returned(code) => code,
        Joined::Panicked => EXIT_PANIC,
        Joined::Unstarted(error) => {
            stderr_line(format_args!(
                "warning: could not start the fgro thread ({error}); running on the \
                 inherited stack"
            ));
            run(task)
        }
    }
}
