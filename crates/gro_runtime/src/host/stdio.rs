//! The stdout and stderr host modules, and the one path every write to
//! either stream takes, the console's included ([`write_all`]).
//!
//! A write is inline and synchronous: the bytes reach the kernel before
//! the call returns, through the same path the console writes by, so the
//! run's output keeps program order across `console` and both modules,
//! and nothing is held back to flush before the runtime is handed back or
//! before `exit` ends the process. Nothing is buffered, so a write that
//! failed leaves nothing a later write could carry out: std's `Stdout`
//! keeps what a failed or partial write left in its line buffer and writes
//! it ahead of the next, so the path writes the descriptor directly
//! (`gro_sys::write_unbuffered`), holding std's lock on the stream so a
//! line Rust prints elsewhere never lands inside a write. The cost is that
//! a slow reader stalls the run, as it stalls the console.
//!
//! A descriptor something else made non-blocking (a process sharing the
//! open file, as Node does its own stdio) answers a write the pipe can't
//! take now with `EAGAIN`; the path sleeps and tries again, the pause
//! doubling from [`FIRST_PAUSE`] to [`LAST_PAUSE`] and starting over once
//! a write takes bytes, so a stalled reader costs the run its latency, up
//! to the last pause, not a core spinning. A write a signal interrupted
//! is tried again at once.
//!
//! Each module's handle is a frozen object of its own kind (`handle.rs`),
//! [`STDOUT`](super::handle::STDOUT) or [`STDERR`](super::handle::STDERR),
//! holding `is_terminal`, read once when the run first imports the module,
//! as `pid` is, as its own data: Node's is a getter on the prototype that
//! asks each time. Its state is which stream, plain data; it holds no
//! descriptor, since the process's own are the only ones written.
//!
//! `write(data)` reads its data natively ([`read_chunk`]): a string as
//! UTF-8, a lone surrogate a `TypeError`, or a `Uint8Array`, copied, one of
//! a detached buffer a `TypeError`, and a fixed-length one a resize left out
//! of its buffer's bounds empty; anything else is a `TypeError`, each with
//! the Node module's message. Empty data resolves `true` with no system
//! call, whatever the descriptor is, where Node's empty write is a write of
//! no bytes answered as the descriptor answers it: `true` on a pipe whose
//! reader has gone, `false` on such a socket, and a rejection on a full disk
//! or a descriptor open only for reading. A reader gone (`EPIPE` or
//! `ECONNRESET`) resolves `false`, as the kernel says each time; any other
//! failure rejects with `StdioError` `io_error`, its target the stream, its
//! `cause` an `Error` holding what the OS said.

use std::io::{self, IsTerminal as _};
use std::os::fd::{AsFd as _, BorrowedFd};
use std::time::Duration;

use deno_core::{OpState, v8};

use super::error_class::{DeclaredFamily, ErrorClasses};
use super::handle::{self, HandleKind};
use super::{ChunkMessages, NO_ERROR_CLASSES, read_chunk, string, throw_error, throw_type_error};

/// The first pause before a write the descriptor refused for now is tried
/// again.
pub const FIRST_PAUSE: Duration = Duration::from_micros(10);

/// The longest pause before a refused write is tried again, the most a
/// stalled reader adds to the run's latency once it drains.
pub const LAST_PAUSE: Duration = Duration::from_millis(1);

/// What `write` throws called on anything but its module's handle.
const NOT_A_HANDLE: &str = "a stream's write is called on its handle, not another object";

/// What `write` refuses its data for, the Node module's messages.
const MESSAGES: ChunkMessages = ChunkMessages {
    not_data: "stdio data is a string or a `Uint8Array`",
    not_well_formed: "stdio data isn't well-formed Unicode",
    detached: "stdio can't write a view of a detached buffer",
};

/// What `StdioError`'s kind is for a write that failed other than by the
/// reader going.
const IO_ERROR: &str = "io_error";

/// A stream the process writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// Standard output, descriptor 1.
    Stdout,
    /// Standard error, descriptor 2.
    Stderr,
}

impl Stream {
    /// Its name, as `StdioError`'s target names it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }

    /// Whether it's a terminal, asked of the OS now.
    fn is_terminal(self) -> bool {
        match self {
            Self::Stdout => io::stdout().is_terminal(),
            Self::Stderr => io::stderr().is_terminal(),
        }
    }
}

/// Writes `bytes` whole to `stream`, unbuffered, holding std's lock on it
/// for the whole write, a write the descriptor refuses for now tried again
/// after a pause ([`write_all_to`]).
///
/// # Errors
///
/// The first error a write met that is neither `EAGAIN` nor `EINTR`, and
/// `WriteZero` for a write that took nothing; what the earlier writes took
/// stays written.
pub fn write_all(stream: Stream, bytes: &[u8]) -> io::Result<()> {
    match stream {
        Stream::Stdout => {
            let locked = io::stdout().lock();
            write_all_to(locked.as_fd(), bytes)
        }
        Stream::Stderr => {
            let locked = io::stderr().lock();
            write_all_to(locked.as_fd(), bytes)
        }
    }
}

/// Writes `bytes` whole to `fd` ([`write_whole`]), by `write(2)` and
/// sleeping for each pause.
fn write_all_to(fd: BorrowedFd<'_>, bytes: &[u8]) -> io::Result<()> {
    write_whole(
        |bytes| gro_sys::write_unbuffered(fd, bytes),
        std::thread::sleep,
        bytes,
    )
}

/// Writes `bytes` whole by `write`, by as many writes as it takes: one a
/// signal interrupted tried again at once, and one the descriptor refused
/// for now (`EAGAIN`) after `pause`, given a pause that doubles from
/// [`FIRST_PAUSE`] to [`LAST_PAUSE`] while the refusals last and starts over
/// once a write takes bytes.
fn write_whole(
    mut write: impl FnMut(&[u8]) -> io::Result<usize>,
    mut pause: impl FnMut(Duration),
    mut bytes: &[u8],
) -> io::Result<()> {
    let mut next = FIRST_PAUSE;
    while !bytes.is_empty() {
        match write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                bytes = bytes.get(written..).unwrap_or_default();
                next = FIRST_PAUSE;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                pause(next);
                next = next_pause(next);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The pause after `pause`: twice it, up to [`LAST_PAUSE`].
fn next_pause(pause: Duration) -> Duration {
    pause.saturating_mul(2).min(LAST_PAUSE)
}

/// Whether a write that failed with `error` found its reader gone, the
/// case a `false` result is for: `EPIPE`, or `ECONNRESET` from a socket.
pub fn is_reader_gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
    )
}

/// The handle of `stream`, of `kind` and `prototype`: its state the
/// stream's name, and whether the stream is a terminal as its
/// `is_terminal`, asked of the OS now.
pub fn output<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &'static HandleKind,
    prototype: v8::Local<'s, v8::Object>,
    stream: Stream,
) -> Option<v8::Local<'s, v8::Value>> {
    let state = string(scope, stream.name())?;
    let is_terminal = v8::Boolean::new(scope, stream.is_terminal());
    handle::make(scope, kind, prototype, state.into(), &[is_terminal.into()]).map(Into::into)
}

/// `write(data)` called on `receiver`, a handle of `kind`, the handle of
/// `stream`: `data` written whole, `true` once the kernel took it, `false`
/// when the reader has gone; a `TypeError` for a receiver that isn't the
/// handle or data that isn't a well-formed string or a live `Uint8Array`,
/// and `StdioError` `io_error` from `errors` for any other failure, each
/// thrown for the member's promise to reject with.
pub fn write<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    stream: Stream,
    kind: &'static HandleKind,
    receiver: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    if handle::read(scope, kind, receiver).is_none() {
        throw_type_error(scope, NOT_A_HANDLE);
        return None;
    }
    let bytes = match read_chunk(scope, data, &MESSAGES) {
        Ok(bytes) => bytes,
        Err(message) => {
            throw_type_error(scope, message);
            return None;
        }
    };
    if bytes.is_empty() {
        return Some(v8::Boolean::new(scope, true).into());
    }
    match write_all(stream, &bytes) {
        Ok(()) => Some(v8::Boolean::new(scope, true).into()),
        Err(error) if is_reader_gone(&error) => Some(v8::Boolean::new(scope, false).into()),
        Err(error) => {
            throw_io_error(state, scope, stream, &error, errors);
            None
        }
    }
}

/// Throws `StdioError` `io_error` about `stream` through `scope`, its
/// `cause` an `Error` holding what the OS said of `error`.
fn throw_io_error(
    state: &OpState,
    scope: &v8::PinScope<'_, '_>,
    stream: Stream,
    error: &io::Error,
    errors: DeclaredFamily,
) {
    let Some(classes) = state.try_borrow::<ErrorClasses>() else {
        return throw_error(scope, NO_ERROR_CLASSES);
    };
    let (Some(target), Some(cause)) = (
        string(scope, stream.name()),
        string(scope, &error.to_string()),
    ) else {
        return;
    };
    let cause = v8::Exception::error(scope, cause);
    classes.throw(scope, errors, IO_ERROR, target, Some(cause));
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Duration;

    use super::{FIRST_PAUSE, LAST_PAUSE, next_pause, write_whole};

    /// What [`write_whole`] made of `bytes` given `writes`, each answering
    /// the next write, an `Ok` taking up to that many bytes: its result's
    /// error kind, the bytes taken, and the pauses it asked for.
    fn scripted(
        mut writes: Vec<io::Result<usize>>,
        bytes: &[u8],
    ) -> (Result<(), io::ErrorKind>, String, Vec<Duration>) {
        let mut taken = Vec::new();
        let mut pauses = Vec::new();
        let written = write_whole(
            |buf| {
                let written = writes.remove(0)?.min(buf.len());
                taken.extend_from_slice(&buf[..written]);
                Ok(written)
            },
            |pause| pauses.push(pause),
            bytes,
        );
        (
            written.map_err(|error| error.kind()),
            String::from_utf8(taken).unwrap(),
            pauses,
        )
    }

    /// A write is whole across short writes, a write a signal interrupted,
    /// and writes the descriptor refused for now, each refusal in a row
    /// paused twice as long as the last, starting over once a write takes
    /// bytes; a write of nothing, or any other failure, ends it with that
    /// error, what was taken before it kept, and nothing is written for no
    /// bytes.
    #[test]
    fn a_write_is_whole_or_fails() {
        let would_block = || Err(io::ErrorKind::WouldBlock.into());
        let us = Duration::from_micros;
        assert_eq!(
            scripted(
                vec![
                    Ok(2),
                    would_block(),
                    would_block(),
                    Err(io::ErrorKind::Interrupted.into()),
                    would_block(),
                    Ok(1),
                    would_block(),
                    Ok(9),
                ],
                b"abcdef"
            ),
            (
                Ok(()),
                "abcdef".to_owned(),
                vec![us(10), us(20), us(40), us(10)]
            )
        );
        assert_eq!(
            scripted(
                vec![Ok(1), Err(io::ErrorKind::BrokenPipe.into())],
                b"abcdef"
            ),
            (Err(io::ErrorKind::BrokenPipe), "a".to_owned(), Vec::new())
        );
        assert_eq!(
            scripted(vec![Ok(0)], b"abcdef"),
            (Err(io::ErrorKind::WriteZero), String::new(), Vec::new())
        );
        assert_eq!(
            scripted(vec![Err(io::ErrorKind::StorageFull.into())], b"abcdef"),
            (Err(io::ErrorKind::StorageFull), String::new(), Vec::new())
        );
        assert_eq!(
            scripted(Vec::new(), b""),
            (Ok(()), String::new(), Vec::new())
        );
    }

    /// The pause doubles from the first to the last and stays there, so a
    /// stalled reader is polled ever less often, never more than the last
    /// pause apart.
    #[test]
    fn the_pause_doubles_to_its_last() {
        let pauses: Vec<Duration> = std::iter::successors(Some(FIRST_PAUSE), |&pause| {
            (pause < LAST_PAUSE).then(|| next_pause(pause))
        })
        .collect();
        assert_eq!(
            pauses,
            [10, 20, 40, 80, 160, 320, 640, 1000].map(Duration::from_micros)
        );
        assert_eq!(next_pause(LAST_PAUSE), LAST_PAUSE);
    }
}
