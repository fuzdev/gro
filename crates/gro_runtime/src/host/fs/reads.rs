//! A whole-file read opens its target beneath the handle without waiting
//! for it, and `gro_sys` refuses anything but a regular file by the opened
//! descriptor before a byte is read: a directory is `is_a_directory`, as
//! the Node module's, and a FIFO, a socket, or a device `io_error`, where
//! the Node module's read would wait on a FIFO or read a device without
//! end. A regular file is read to its end, never trusting the size it
//! reports to end it, and one past `2 ** 31 - 1` bytes, or that reports
//! more, is `io_error`, a size past it refused before a byte is read. Its
//! bytes are read straight into a buffer V8 allocated of the size it
//! reported ([`read_bytes`]), so V8's collector counts them as it counts a
//! buffer JS allocates; only a file whose size was wrong is copied.

use std::fmt::Write as _;
use std::os::unix::ffi::OsStringExt as _;

use deno_core::{OpState, v8};
use gro_v8::PendingArrayBuffer;

use super::{
    Access, Failure, FsErrorKind, Op, file_kind_name, finish, is_missing, path_text, read_follow,
    sys,
};
use crate::host::error_class::DeclaredFamily;
use crate::host::handle::FS_READ_DIR;
use crate::host::{name, string};

/// What `stat` and `exists` answer: the entry's info, or none when nothing
/// is there.
enum Found {
    /// The entry, as `stat` reported it.
    Entry(sys::Info),
    /// Nothing at the path.
    Missing,
}

/// `stat(path, options?)` called on `receiver`: the entry's info, a
/// null-prototype record of its `kind`, `size`, `mtime_ms`, and `mode`, or
/// `null` when nothing is at `path`, following a final symlink unless
/// `options` says `follow: false`, which a trailing `/` overrides, as in the
/// kernel.
pub fn stat<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = find(state, scope, receiver, path, Some(options)).and_then(|found| match found {
        Found::Entry(info) => file_info(scope, &info).ok_or(Failure::Thrown),
        Found::Missing => Ok(v8::null(scope).into()),
    });
    finish(state, scope, Some(errors), result)
}

/// `exists(path)` called on `receiver`: whether an entry is at `path`,
/// following a final symlink; any failure but a missing entry throws.
pub fn exists<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = find(state, scope, receiver, path, None)
        .map(|found| v8::Boolean::new(scope, matches!(found, Found::Entry(_))).into());
    finish(state, scope, Some(errors), result)
}

/// The entry at `path` beneath `receiver`, a handle of `FsReadDir`'s kind
/// or one it extends, as `stat` reads it with `options`, or as `exists`
/// does with none.
fn find<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    options: Option<v8::Local<'s, v8::Value>>,
) -> Result<Found, Failure> {
    let op = Op::new(state, scope, &FS_READ_DIR, receiver)?;
    let requested = path_text(scope, path)?;
    let follow = match options {
        Some(options) => read_follow(scope, options)?,
        None => true,
    };
    let end = op.resolve(requested)?;
    op.check(Access::Read, &[&end], false)?;
    // a trailing `/` follows a final symlink, as in the kernel
    let follow = follow || end.lexical.dir;
    let (fd, dir, rel) = op.call(&end)?;
    match sys::stat(fd, dir, rel, follow) {
        Ok(info) => Ok(Found::Entry(info)),
        Err(error) if is_missing(&error) => Ok(Found::Missing),
        Err(error) => Err(op.failure(&end, error)),
    }
}

/// What a whole-file read returns.
#[derive(Clone, Copy)]
pub enum Contents {
    /// The bytes, a `Uint8Array` of a buffer of its own.
    Bytes,
    /// The text, decoded from UTF-8 as Node's `readFile` with `'utf8'`
    /// decodes it: each malformed sequence a U+FFFD, a BOM kept.
    Text,
}

/// `read(path)`, `read_text(path)`, and their kin, called on `receiver`:
/// the contents of the regular file at `path`, read to its end, as
/// `contents` says; with `if_exists`, `null` when nothing is at `path`,
/// where any other failure still throws. A directory is `is_a_directory`,
/// and anything else that isn't a regular file, a FIFO, a socket, or a
/// device, `io_error`, refused before any byte is read, where the Node
/// module would wait on a FIFO or read a device without end.
pub fn read<'s>(
    state: &OpState,
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    contents: Contents,
    if_exists: bool,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let op = Op::new(state, scope, &FS_READ_DIR, receiver)?;
        let end = op.path(scope, path)?;
        op.check(Access::Read, &[&end], false)?;
        let (fd, dir, rel) = op.call(&end)?;
        let reading = match sys::open_read(fd, dir, rel) {
            Ok(reading) => reading,
            Err(error) if if_exists && is_missing(&error) => return Ok(v8::null(scope).into()),
            Err(error) => return Err(op.failure(&end, error)),
        };
        let read_failure = |error| op.failure(&end, sys::OpError::Target(error));
        match contents {
            Contents::Bytes => {
                let bytes = read_bytes(scope, reading).map_err(read_failure)?;
                to_bytes(scope, bytes)
            }
            Contents::Text => {
                let bytes = reading.to_end().map_err(read_failure)?;
                v8::String::new_from_utf8(scope, &bytes, v8::NewStringType::Normal)
                    .map(Into::into)
                    .ok_or_else(|| Failure::Fs {
                        kind: FsErrorKind::IoError,
                        path: end.target.clone(),
                        cause: Some(format!(
                            "{} holds {} bytes, more text than a string holds",
                            end.target,
                            bytes.len()
                        )),
                    })
            }
        }
    })();
    finish(state, scope, Some(errors), result)
}

/// The file `reading` opened, read to its end into a buffer V8 allocated
/// ([`PendingArrayBuffer`]), so its collector paces itself by every read's
/// bytes as by a buffer JS allocates, and frees what JS drops before more
/// piles up.
///
/// The buffer is allocated of the size the file reported and read
/// straight into, so a file whose size is right is in memory once. One
/// that shrank since it reported its size is copied into a buffer of what
/// it held; one that grew, or reported none, as a `/proc` file reports 0,
/// has its rest read into a `Vec` and both parts copied into a buffer of
/// their length.
fn read_bytes(
    isolate: &mut v8::Isolate,
    mut reading: sys::Reading,
) -> std::io::Result<PendingArrayBuffer> {
    let mut sized = PendingArrayBuffer::new(isolate, reading.size());
    let filled = reading.fill(sized.bytes_mut())?;
    if filled < sized.len() {
        // the file ended short of its size, so its end is read
        return Ok(copied(isolate, &sized.bytes()[..filled], &[]));
    }
    let rest = reading.rest()?;
    if rest.is_empty() {
        return Ok(sized);
    }
    Ok(copied(isolate, sized.bytes(), &rest))
}

/// `bytes` as a `Uint8Array` of the buffer of its own they become.
fn to_bytes<'s>(
    scope: &v8::PinScope<'s, '_>,
    bytes: PendingArrayBuffer,
) -> Result<v8::Local<'s, v8::Value>, Failure> {
    let length = bytes.len();
    let buffer = bytes.into_array_buffer(scope);
    v8::Uint8Array::new(scope, buffer, 0, length)
        .map(Into::into)
        .ok_or_else(|| Failure::Plain(format!("V8 made no Uint8Array of {length} bytes")))
}

/// `head` then `tail` in a buffer of their length, which V8 allocated.
fn copied(isolate: &mut v8::Isolate, head: &[u8], tail: &[u8]) -> PendingArrayBuffer {
    let mut whole = PendingArrayBuffer::new(isolate, head.len() + tail.len());
    let (start, end) = whole.bytes_mut().split_at_mut(head.len());
    start.copy_from_slice(head);
    end.copy_from_slice(tail);
    whole
}

/// `read_link(path)` called on `receiver`: the target of the symlink at
/// `path`, as stored, never followed, so it may be absolute or lead out of
/// the handle, which reading it reaches nothing of; with a trailing `/` the
/// link is followed, as in the kernel, and what it reaches is no symlink.
/// An entry that isn't a symlink is `io_error`, and so is a target that
/// isn't UTF-8, since no string spells it.
pub fn read_link<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let op = Op::new(state, scope, &FS_READ_DIR, receiver)?;
        let end = op.path(scope, path)?;
        op.check(Access::Read, &[&end], false)?;
        let (fd, dir, rel) = op.call(&end)?;
        let target = sys::read_link(fd, dir, rel, end.lexical.dir)
            .map_err(|error| op.failure(&end, error))?;
        let target = to_utf8(target.into_vec(), &end.target, "is a symlink to")?;
        string(scope, &target)
            .map(Into::into)
            .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `bytes`, a name the kernel gave for `path`, as text, or `io_error`
/// about `path` when they aren't UTF-8, its cause saying `what` they are
/// with each byte that isn't printable ASCII escaped, as the Node module
/// says it.
fn to_utf8(bytes: Vec<u8>, path: &str, what: &str) -> Result<String, Failure> {
    String::from_utf8(bytes).map_err(|error| not_utf8(error.as_bytes(), path, what))
}

/// `io_error` about `path` for `bytes`, a name the kernel gave for it
/// that isn't UTF-8, as [`to_utf8`] fails.
pub(super) fn not_utf8(bytes: &[u8], path: &str, what: &str) -> Failure {
    Failure::Fs {
        kind: FsErrorKind::IoError,
        path: path.to_owned(),
        cause: Some(format!(
            "{path} {what} {}, which isn't UTF-8",
            escaped(bytes)
        )),
    }
}

/// `bytes` with printable ASCII as itself and every other byte, a `\`
/// among them, as `\xNN`.
pub(in crate::host) fn escaped(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for &byte in bytes {
        if (0x20..0x7f).contains(&byte) && byte != b'\\' {
            text.push(char::from(byte));
        } else {
            let _ = write!(text, "\\x{byte:02x}");
        }
    }
    text
}

/// `info` as `stat` returns it: a null-prototype record of the entry's
/// `kind`, `size`, `mtime_ms`, and `mode`, its own to change, as the Node
/// module's plain object is; null-prototype, so a promise resolved with it
/// finds no `then` of the realm's.
fn file_info<'s>(
    scope: &v8::PinScope<'s, '_>,
    info: &sys::Info,
) -> Option<v8::Local<'s, v8::Value>> {
    let keys = [
        name(scope, "kind")?,
        name(scope, "size")?,
        name(scope, "mtime_ms")?,
        name(scope, "mode")?,
    ];
    let values = [
        string(scope, file_kind_name(info.kind))?.into(),
        v8::Number::new(scope, info.size).into(),
        v8::Number::new(scope, info.mtime_ms).into(),
        v8::Integer::new_from_unsigned(scope, info.mode).into(),
    ];
    let null = v8::null(scope).into();
    Some(v8::Object::with_prototype_and_properties(scope, null, &keys, &values).into())
}

#[cfg(test)]
mod tests {
    use super::sys;

    /// A read's bytes come from the isolate's own `ArrayBuffer` allocator,
    /// a success after a failed allocation clearing what the recording
    /// allocator holds, and end where the file ends when read: one of the
    /// size it reported, one grown or shrunk since its open, an empty one,
    /// and a `/proc` file, which reports 0; each a `Uint8Array` of its
    /// whole buffer.
    #[test]
    fn a_read_s_bytes_are_v8_s_and_end_where_the_file_ends() {
        use std::io::Write as _;
        use std::path::Path;
        use std::sync::Arc;

        use deno_core::{JsRuntime, RuntimeOptions, v8};

        use super::{read_bytes, to_bytes};

        let dir = crate::test_support::TestDir::on_disk("read_bytes");
        let file = |name: &str| dir.path().join(name);
        for name in ["sized", "grows", "shrinks"] {
            std::fs::write(file(name), "12345").unwrap();
        }
        std::fs::write(file("empty"), "").unwrap();
        let open = |root: &Path, name: &str| {
            let root = std::fs::File::open(root).unwrap();
            let opened = sys::open_read(Arc::new(root.into()), String::new(), name.to_owned());
            opened.unwrap_or_else(|_| panic!("{name} didn't open"))
        };
        let (create_params, record) = gro_v8::record_array_buffers(v8::Isolate::create_params());
        // the collections after a failed allocation post delayed tasks
        let tokio = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _entered = tokio.enter();
        let mut runtime = JsRuntime::new(RuntimeOptions {
            create_params: Some(create_params),
            ..Default::default()
        });
        // an allocation too large for any process, which the record holds
        // until the next one succeeds
        let fail = |scope: &mut v8::PinScope<'_, '_>| {
            let source = "try { new ArrayBuffer(2 ** 52); } catch {}";
            let source = v8::String::new(scope, source).unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            script.run(scope).unwrap();
            assert!(record.latest_failed());
        };
        let reads = [
            ("sized", b"12345".as_slice(), None),
            ("grows", b"123456789", Some(true)),
            ("shrinks", b"12", Some(false)),
            ("empty", b"", None),
        ];
        let mut opened = Vec::new();
        for (name, expected, change) in reads {
            let reading = open(dir.path(), name);
            match change {
                Some(true) => std::fs::OpenOptions::new()
                    .append(true)
                    .open(file(name))
                    .unwrap()
                    .write_all(b"6789")
                    .unwrap(),
                Some(false) => std::fs::OpenOptions::new()
                    .write(true)
                    .open(file(name))
                    .unwrap()
                    .set_len(2)
                    .unwrap(),
                None => {}
            }
            opened.push((name, expected, reading));
        }
        let status = open(Path::new("/proc/self"), "status");
        assert_eq!(status.size(), 0);
        deno_core::scope!(scope, &mut runtime);
        for (name, expected, reading) in opened {
            fail(scope);
            let bytes = read_bytes(scope, reading).unwrap_or_else(|_| panic!("{name} failed"));
            if !expected.is_empty() {
                assert!(!record.latest_failed(), "{name}'s buffer isn't V8's");
            }
            assert_eq!(bytes.bytes(), expected, "{name}");
            let length = bytes.len();
            let Ok(array) = to_bytes(scope, bytes) else {
                panic!("{name} made no Uint8Array");
            };
            let array = v8::Local::<v8::Uint8Array>::try_from(array).unwrap();
            let buffer = array.buffer(scope).unwrap();
            assert_eq!(
                (array.byte_length(), buffer.byte_length()),
                (length, length),
                "{name}"
            );
        }
        fail(scope);
        let status = read_bytes(scope, status).unwrap_or_else(|_| panic!("status failed"));
        assert!(
            status.bytes().starts_with(b"Name:"),
            "{}",
            String::from_utf8_lossy(status.bytes())
        );
        assert!(!record.latest_failed(), "status's buffer isn't V8's");
    }
}
