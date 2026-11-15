//! The calls confined beneath a descriptor, each on a scratch tree of its
//! own under the temporary directory: what `RESOLVE_BENEATH` refuses and
//! allows, each call's success and the errors a caller maps, and a rename
//! racing the opens.

#![cfg(target_os = "linux")]
#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]
#![expect(clippy::panic, reason = "a test fails by panicking")]

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gro_sys::beneath::{self, DirEntry, FileKind, FinalLink, Links, OpenAs, Stat};

/// A scratch tree, `root/` and `outside/` beside it, removed when dropped.
struct Scratch {
    base: PathBuf,
}

impl Scratch {
    /// A fresh tree for the test `name`.
    fn new(name: &str) -> Self {
        let base =
            std::env::temp_dir().join(format!("gro_sys_beneath_{}_{name}", std::process::id()));
        if base.exists() {
            fs::remove_dir_all(&base).unwrap();
        }
        fs::create_dir_all(base.join("root")).unwrap();
        fs::create_dir_all(base.join("outside")).unwrap();
        fs::write(base.join("outside/f"), "outside").unwrap();
        Self { base }
    }

    /// The path of `rel` under `root/`.
    fn root(&self, rel: &str) -> PathBuf {
        self.base.join("root").join(rel)
    }

    /// The path of `rel` under `outside/`.
    fn outside(&self, rel: &str) -> PathBuf {
        self.base.join("outside").join(rel)
    }

    /// A descriptor on `root/`.
    fn root_fd(&self) -> OwnedFd {
        File::open(self.root("")).unwrap().into()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // the directory a test made unwritable is writable again, so its
        // entries can go
        let _ = fs::set_permissions(self.root("locked"), fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn p(path: &str) -> &Path {
    Path::new(path)
}

/// The OS code of `result`'s error, panicking on success.
fn code<T: std::fmt::Debug>(result: io::Result<T>) -> i32 {
    let error = result.unwrap_err();
    error
        .raw_os_error()
        .unwrap_or_else(|| panic!("no OS code: {error:?}"))
}

/// Whether `fd` is the entry at `path`, by device and inode.
fn is_entry(fd: BorrowedFd<'_>, path: &Path) -> bool {
    let opened = File::from(fd.try_clone_to_owned().unwrap())
        .metadata()
        .unwrap();
    let there = fs::symlink_metadata(path).unwrap();
    (opened.dev(), opened.ino()) == (there.dev(), there.ino())
}

fn read_all(fd: OwnedFd) -> String {
    let mut text = String::new();
    File::from(fd).read_to_string(&mut text).unwrap();
    text
}

// --- what `RESOLVE_BENEATH` refuses and allows (probe P14)

/// A `..` that climbs above the descriptor is refused even when the path
/// lands back inside it.
#[test]
fn a_dot_dot_climbing_out_is_refused_though_it_lands_back_inside() {
    let scratch = Scratch::new("climb");
    fs::create_dir(scratch.root("sub")).unwrap();
    fs::write(scratch.root("f"), "inside").unwrap();
    let root = scratch.root_fd();
    for path in ["../root/f", "sub/../../root/f", "..", "../outside/f"] {
        let result = beneath::open(root.as_fd(), p(path), OpenAs::Read, Links::Beneath);
        assert_eq!(code(result), libc::EXDEV, "{path}");
        let result = beneath::stat(root.as_fd(), p(path), FinalLink::Keep);
        assert_eq!(code(result), libc::EXDEV, "{path}");
    }
}

/// An absolute path is refused, one naming an entry inside included.
#[test]
fn an_absolute_path_is_refused() {
    let scratch = Scratch::new("absolute");
    fs::write(scratch.root("f"), "inside").unwrap();
    let root = scratch.root_fd();
    let inside = scratch.root("f");
    for path in [inside.as_path(), p("/"), p("/etc")] {
        for how in [OpenAs::Read, OpenAs::Directory] {
            let result = beneath::open(root.as_fd(), path, how, Links::Beneath);
            assert_eq!(code(result), libc::EXDEV, "{path:?}");
        }
    }
}

/// A symlink whose target leaves the tree is refused, relative or absolute,
/// at the path's end or on its way, and what it points at is untouched.
#[test]
fn a_symlink_pointing_out_is_refused() {
    let scratch = Scratch::new("link_out");
    symlink("../outside/f", scratch.root("relative")).unwrap();
    symlink(scratch.outside("f"), scratch.root("absolute")).unwrap();
    symlink("../outside", scratch.root("dir")).unwrap();
    let root = scratch.root_fd();
    for path in ["relative", "absolute", "dir/f", "dir/new"] {
        let opened = beneath::open(root.as_fd(), p(path), OpenAs::Read, Links::Beneath);
        assert_eq!(code(opened), libc::EXDEV, "{path}");
        let written = beneath::open(
            root.as_fd(),
            p(path),
            OpenAs::Write { mode: 0o644 },
            Links::Beneath,
        );
        assert_eq!(code(written), libc::EXDEV, "{path}");
        let stat = beneath::stat(root.as_fd(), p(path), FinalLink::Follow);
        assert_eq!(code(stat), libc::EXDEV, "{path}");
    }
    assert_eq!(fs::read_to_string(scratch.outside("f")).unwrap(), "outside");
    assert!(!scratch.outside("new").exists());
}

/// A relative symlink whose target stays inside is followed with
/// `Links::Beneath` and refused with `Links::Refused`, at the end or on the
/// way; an absolute one is refused even when it names an entry inside.
#[test]
fn a_symlink_pointing_in_is_followed_or_refused_by_flag() {
    let scratch = Scratch::new("link_in");
    fs::create_dir(scratch.root("sub")).unwrap();
    fs::write(scratch.root("sub/f"), "inside").unwrap();
    symlink("sub", scratch.root("in")).unwrap();
    symlink(scratch.root("sub"), scratch.root("absolute")).unwrap();
    let root = scratch.root_fd();

    let dir = beneath::open(root.as_fd(), p("in"), OpenAs::Directory, Links::Beneath).unwrap();
    assert!(is_entry(dir.as_fd(), &scratch.root("sub")));
    let file = beneath::open(root.as_fd(), p("in/f"), OpenAs::Read, Links::Beneath).unwrap();
    assert_eq!(read_all(file), "inside");

    for path in ["in", "in/f", "sub/../in"] {
        let how = if path == "in/f" {
            OpenAs::Read
        } else {
            OpenAs::Directory
        };
        let result = beneath::open(root.as_fd(), p(path), how, Links::Refused);
        assert_eq!(code(result), libc::ELOOP, "{path}");
    }
    let plain = beneath::open(root.as_fd(), p("sub"), OpenAs::Directory, Links::Refused);
    assert!(plain.is_ok());

    for path in ["absolute", "absolute/f"] {
        let result = beneath::open(root.as_fd(), p(path), OpenAs::Read, Links::Beneath);
        assert_eq!(code(result), libc::EXDEV, "{path}");
    }
}

/// A magic link is refused, at the path's end or on its way, though it
/// lies beneath the descriptor; `stat` keeping the final link reports the
/// link itself.
#[test]
fn a_magic_link_is_refused() {
    let scratch = Scratch::new("magic");
    fs::write(scratch.root("f"), "inside").unwrap();
    let held = File::open(scratch.root("f")).unwrap();
    let held_fd = std::os::fd::AsRawFd::as_raw_fd(&held).to_string();
    let proc_fds = File::open("/proc/self/fd").unwrap();
    let result = beneath::open(proc_fds.as_fd(), p(&held_fd), OpenAs::Read, Links::Beneath);
    assert_eq!(code(result), libc::ELOOP);
    let result = beneath::stat(proc_fds.as_fd(), p(&held_fd), FinalLink::Follow);
    assert_eq!(code(result), libc::ELOOP);
    let kept = beneath::stat(proc_fds.as_fd(), p(&held_fd), FinalLink::Keep).unwrap();
    assert_eq!(kept.kind, FileKind::Symlink);

    let proc_self = File::open("/proc/self").unwrap();
    for path in [
        format!("fd/{held_fd}"),
        "cwd/.".to_owned(),
        "root/etc".to_owned(),
    ] {
        let how = if path.starts_with("fd/") {
            OpenAs::Read
        } else {
            OpenAs::Directory
        };
        let result = beneath::open(proc_self.as_fd(), p(&path), how, Links::Beneath);
        assert_eq!(code(result), libc::ELOOP, "{path}");
    }
}

/// `.` and `..` that stay beneath the descriptor resolve as they would
/// anywhere.
#[test]
fn dots_that_stay_inside_resolve() {
    let scratch = Scratch::new("dots");
    fs::create_dir_all(scratch.root("a/b")).unwrap();
    fs::write(scratch.root("f"), "inside").unwrap();
    let root = scratch.root_fd();
    for path in ["f", "./f", "a/../f", "a/b/../.././f", "a/./b/../../f"] {
        let fd = beneath::open(root.as_fd(), p(path), OpenAs::Read, Links::Beneath).unwrap();
        assert!(is_entry(fd.as_fd(), &scratch.root("f")), "{path}");
    }
    for path in [".", "a/..", "./a/b/../.."] {
        let fd = beneath::open(root.as_fd(), p(path), OpenAs::Directory, Links::Beneath).unwrap();
        assert!(is_entry(fd.as_fd(), &scratch.root("")), "{path}");
    }
}

/// While another thread moves a directory on the path out of the tree and
/// back, no open returns a descriptor on anything but the entry inside: an
/// open fails (`ENOENT` while the directory is out, `EXDEV` when the walk
/// is caught climbing out, `EAGAIN` once the retries run out) or opens the
/// inside entry. The path descends far below the moved directory and climbs
/// back past it, so a move lands mid-walk often.
#[test]
fn a_rename_racing_the_opens_never_opens_outside() {
    const DEPTH: usize = 24;
    let scratch = Scratch::new("race");
    let deep: String =
        (0..DEPTH).fold(String::new(), |deep, level| deep + "/" + &level.to_string());
    fs::create_dir_all(scratch.root(&format!("a/b{deep}"))).unwrap();
    fs::write(scratch.root("a/x"), "inside").unwrap();
    // the path is `a/x` with `b` inside, and `outside/x` with `b` moved there
    fs::write(scratch.outside("x"), "outside").unwrap();
    let path = format!("a/b{deep}/{}x", "../".repeat(DEPTH + 1));
    let root = scratch.root_fd();
    let inside = fs::metadata(scratch.root("a/x")).unwrap().ino();

    let stop = Arc::new(AtomicBool::new(false));
    let mover = {
        let stop = Arc::clone(&stop);
        let (inside, out) = (scratch.root("a/b"), scratch.outside("b"));
        std::thread::spawn(move || {
            let mut moves = 0u64;
            while !stop.load(Ordering::Relaxed) {
                fs::rename(&inside, &out).unwrap();
                fs::rename(&out, &inside).unwrap();
                moves += 2;
            }
            moves
        })
    };
    let (mut opened, mut outside, mut missing, mut climbing, mut raced) = (0u64, 0, 0, 0, 0);
    let mut unexpected = None;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match beneath::open(root.as_fd(), p(&path), OpenAs::Read, Links::Beneath) {
            Ok(fd) => {
                // counted, not asserted here, so the mover stops before a failure
                if File::from(fd).metadata().unwrap().ino() == inside {
                    opened += 1;
                } else {
                    outside += 1;
                }
            }
            Err(error) => match error.raw_os_error() {
                Some(libc::ENOENT) => missing += 1,
                Some(libc::EXDEV) => climbing += 1,
                Some(libc::EAGAIN) => raced += 1,
                _ => {
                    unexpected = Some(error);
                    break;
                }
            },
        }
    }
    stop.store(true, Ordering::Relaxed);
    let moves = mover.join().unwrap();
    eprintln!(
        "race: {moves} moves; {opened} opened inside, {outside} outside, {missing} ENOENT, \
         {climbing} EXDEV, {raced} EAGAIN"
    );
    assert!(unexpected.is_none(), "{unexpected:?}");
    assert_eq!(outside, 0, "an open reached outside the tree");
    assert!(opened > 0 && moves > 0);
}

// --- each call

/// `open` reads a file, writes one with its mode, replaces one, and writes
/// through a final symlink inside to its target, a dangling one making it.
#[test]
fn open_reads_and_writes_beneath() {
    let scratch = Scratch::new("open");
    fs::create_dir(scratch.root("d")).unwrap();
    fs::write(scratch.root("f"), "old contents").unwrap();
    symlink("f", scratch.root("to_f")).unwrap();
    symlink("made", scratch.root("dangling")).unwrap();
    let root = scratch.root_fd();

    let fd = beneath::open(root.as_fd(), p("f"), OpenAs::Read, Links::Beneath).unwrap();
    assert_eq!(read_all(fd), "old contents");

    let write = OpenAs::Write { mode: 0o600 };
    let fd = beneath::open(root.as_fd(), p("new"), write, Links::Beneath).unwrap();
    io::Write::write_all(&mut File::from(fd), b"new").unwrap();
    assert_eq!(fs::read_to_string(scratch.root("new")).unwrap(), "new");
    let mode = fs::metadata(scratch.root("new")).unwrap().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let fd = beneath::open(root.as_fd(), p("to_f"), write, Links::Beneath).unwrap();
    io::Write::write_all(&mut File::from(fd), b"x").unwrap();
    assert_eq!(fs::read_to_string(scratch.root("f")).unwrap(), "x");
    assert!(
        fs::symlink_metadata(scratch.root("to_f"))
            .unwrap()
            .is_symlink()
    );

    let fd = beneath::open(root.as_fd(), p("dangling"), write, Links::Beneath).unwrap();
    drop(fd);
    assert!(scratch.root("made").is_file());

    assert_eq!(
        code(beneath::open(root.as_fd(), p("d"), write, Links::Beneath)),
        libc::EISDIR
    );
    assert_eq!(
        code(beneath::open(
            root.as_fd(),
            p("missing/f"),
            write,
            Links::Beneath
        )),
        libc::ENOENT
    );
    assert_eq!(
        code(beneath::open(
            root.as_fd(),
            p("f/x"),
            OpenAs::Read,
            Links::Beneath
        )),
        libc::ENOTDIR
    );
    let too_wide = OpenAs::Write { mode: 0o10000 };
    assert_eq!(
        code(beneath::open(
            root.as_fd(),
            p("wide"),
            too_wide,
            Links::Beneath
        )),
        libc::EINVAL
    );
}

/// `CreateNew` makes only a new file: any entry there, a dangling symlink
/// included, fails with `EEXIST` and is left as it was.
#[test]
fn create_new_refuses_any_entry_there() {
    let scratch = Scratch::new("create_new");
    fs::create_dir(scratch.root("d")).unwrap();
    fs::write(scratch.root("f"), "kept").unwrap();
    symlink("f", scratch.root("to_f")).unwrap();
    symlink("made", scratch.root("dangling")).unwrap();
    let root = scratch.root_fd();
    let create_new = OpenAs::CreateNew { mode: 0o644 };

    let fd = beneath::open(root.as_fd(), p("new"), create_new, Links::Beneath).unwrap();
    drop(fd);
    assert!(scratch.root("new").is_file());

    for path in ["new", "f", "to_f", "dangling", "d"] {
        let result = beneath::open(root.as_fd(), p(path), create_new, Links::Beneath);
        assert_eq!(code(result), libc::EEXIST, "{path}");
    }
    assert_eq!(fs::read_to_string(scratch.root("f")).unwrap(), "kept");
    assert!(!scratch.root("made").exists());
}

/// No open waits on a FIFO: a read opens at once with no writer, a write
/// with no reader fails with `ENXIO`, and a regular file's descriptor reads
/// and writes as any other.
#[test]
fn no_open_waits_on_a_fifo() {
    let scratch = Scratch::new("fifo");
    let made = std::process::Command::new("mkfifo")
        .arg(scratch.root("pipe"))
        .status()
        .unwrap();
    assert!(made.success());
    fs::write(scratch.root("f"), "inside").unwrap();
    let root = scratch.root_fd();
    // on a thread, so an open that waits fails the test rather than hanging it
    let opened = {
        let root = root.try_clone().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // the read end closes before the write opens, so the FIFO has no reader
            let read =
                beneath::open(root.as_fd(), p("pipe"), OpenAs::Read, Links::Beneath).map(drop);
            let write = OpenAs::Write { mode: 0o644 };
            let written = beneath::open(root.as_fd(), p("pipe"), write, Links::Beneath).map(drop);
            send.send((read, written)).unwrap();
        });
        receive.recv_timeout(Duration::from_secs(5))
    };
    let (read, written) = opened.unwrap_or_else(|_| panic!("an open of a FIFO waited"));
    read.unwrap();
    assert_eq!(code(written), libc::ENXIO);

    let fd = beneath::open(root.as_fd(), p("f"), OpenAs::Read, Links::Beneath).unwrap();
    assert_eq!(read_all(fd), "inside");
}

/// `OpenAs::Directory` opens a directory only, and `Links::Refused` opens
/// one reached by its own name only.
#[test]
fn open_as_a_directory() {
    let scratch = Scratch::new("directory");
    fs::create_dir(scratch.root("d")).unwrap();
    fs::write(scratch.root("f"), "").unwrap();
    symlink("d", scratch.root("to_d")).unwrap();
    let root = scratch.root_fd();
    for links in [Links::Beneath, Links::Refused] {
        let fd = beneath::open(root.as_fd(), p("d"), OpenAs::Directory, links).unwrap();
        assert!(is_entry(fd.as_fd(), &scratch.root("d")));
        let result = beneath::open(root.as_fd(), p("f"), OpenAs::Directory, links);
        assert_eq!(code(result), libc::ENOTDIR);
    }
    let result = beneath::open(root.as_fd(), p("to_d"), OpenAs::Directory, Links::Refused);
    assert_eq!(code(result), libc::ELOOP);
    let result = beneath::open(root.as_fd(), p(""), OpenAs::Directory, Links::Beneath);
    assert_eq!(code(result), libc::ENOENT);
    let error =
        beneath::open(root.as_fd(), p("d\0"), OpenAs::Directory, Links::Beneath).unwrap_err();
    assert_eq!(
        (error.kind(), error.raw_os_error()),
        (io::ErrorKind::InvalidInput, None)
    );
}

/// `stat` reports a file's, a directory's, and a symlink's kind, size,
/// mode, and modification time, the link kept or followed.
#[test]
fn stat_reports_each_kind() {
    let scratch = Scratch::new("stat");
    fs::create_dir(scratch.root("d")).unwrap();
    fs::write(scratch.root("f"), "12345").unwrap();
    fs::set_permissions(scratch.root("f"), fs::Permissions::from_mode(0o640)).unwrap();
    symlink("f", scratch.root("to_f")).unwrap();
    symlink("gone", scratch.root("dangling")).unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(scratch.root("sock")).unwrap();
    let root = scratch.root_fd();

    let f = beneath::stat(root.as_fd(), p("f"), FinalLink::Keep).unwrap();
    let there = fs::metadata(scratch.root("f")).unwrap();
    assert_eq!(
        f,
        Stat {
            kind: FileKind::File,
            size: 5,
            mode: 0o640,
            mtime_sec: there.mtime(),
            mtime_nsec: u32::try_from(there.mtime_nsec()).unwrap(),
        }
    );
    let followed = beneath::stat(root.as_fd(), p("to_f"), FinalLink::Follow).unwrap();
    assert_eq!(followed, f);
    let kept = beneath::stat(root.as_fd(), p("to_f"), FinalLink::Keep).unwrap();
    assert_eq!((kept.kind, kept.size), (FileKind::Symlink, 1));
    let dangling = beneath::stat(root.as_fd(), p("dangling"), FinalLink::Keep).unwrap();
    assert_eq!(dangling.kind, FileKind::Symlink);
    let d = beneath::stat(root.as_fd(), p("d"), FinalLink::Follow).unwrap();
    assert_eq!(d.kind, FileKind::Directory);
    let sock = beneath::stat(root.as_fd(), p("sock"), FinalLink::Follow).unwrap();
    assert_eq!(sock.kind, FileKind::Other);

    for (path, final_link, expected) in [
        ("dangling", FinalLink::Follow, libc::ENOENT),
        ("missing", FinalLink::Keep, libc::ENOENT),
        ("f/", FinalLink::Keep, libc::ENOTDIR),
        ("f/x", FinalLink::Keep, libc::ENOTDIR),
    ] {
        let result = beneath::stat(root.as_fd(), p(path), final_link);
        assert_eq!(code(result), expected, "{path}");
    }
}

/// `mtime_ms` is the double Node's `mtimeMs` is, each value here printed by
/// Node 26 for a file with that time.
#[test]
fn mtime_ms_is_node_s_double() {
    for (sec, nsec, node) in [
        (1_700_000_000, 123_456_789, "1700000000123.4568"),
        (1, 1, "1000.000001"),
        (1_763_200_000, 999_999_999, "1763200001000"),
        (0, 1000, "0.001"),
        (-2, 500_000_000, "-1500"),
        (-1, 999_999_999, "-9.999999974752427e-7"),
    ] {
        let node: f64 = node.parse().unwrap();
        let stat = Stat {
            kind: FileKind::File,
            size: 0,
            mode: 0,
            mtime_sec: sec,
            mtime_nsec: nsec,
        };
        assert_eq!(
            stat.mtime_ms().to_bits(),
            f64::to_bits(node),
            "{sec} {nsec}"
        );
    }
}

/// `read_dir` lists every entry but `.` and `..`, with its kind, names as
/// bytes, across as many reads as the directory takes.
#[test]
fn read_dir_lists_names_as_bytes() {
    let scratch = Scratch::new("read_dir");
    fs::create_dir(scratch.root("d")).unwrap();
    fs::create_dir(scratch.root("d/sub")).unwrap();
    fs::write(scratch.root("d/f"), "").unwrap();
    symlink("sub", scratch.root("d/link")).unwrap();
    fs::write(
        scratch.root("d").join(OsStr::from_bytes(b"not\xffutf8")),
        "",
    )
    .unwrap();
    symlink("d", scratch.root("to_d")).unwrap();
    fs::create_dir(scratch.root("many")).unwrap();
    let long = "n".repeat(60);
    for i in 0..600 {
        fs::write(scratch.root("many").join(format!("{long}{i}")), "").unwrap();
    }
    let root = scratch.root_fd();

    for path in ["d", "to_d"] {
        let mut entries = beneath::read_dir(root.as_fd(), p(path)).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let expected = [
            (&b"f"[..], FileKind::File),
            (b"link", FileKind::Symlink),
            (b"not\xffutf8", FileKind::File),
            (b"sub", FileKind::Directory),
        ];
        let expected = expected.map(|(name, kind)| DirEntry {
            name: OsStr::from_bytes(name).to_owned(),
            kind: Some(kind),
        });
        assert_eq!(entries, expected, "{path}");
    }

    let many = beneath::read_dir(root.as_fd(), p("many")).unwrap();
    assert_eq!(many.len(), 600);
    let mut names: Vec<_> = many.into_iter().map(|entry| entry.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 600);

    let empty = beneath::read_dir(root.as_fd(), p("d/sub")).unwrap();
    assert!(empty.is_empty());

    for (path, expected) in [
        ("d/f", libc::ENOTDIR),
        ("missing", libc::ENOENT),
        ("../outside", libc::EXDEV),
    ] {
        assert_eq!(
            code(beneath::read_dir(root.as_fd(), p(path))),
            expected,
            "{path}"
        );
    }
}

/// `mkdir` makes a directory beneath, and fails on any entry there, a
/// missing or non-directory parent, and a parent that leaves the tree.
#[test]
fn mkdir_beneath() {
    let scratch = Scratch::new("mkdir");
    fs::write(scratch.root("f"), "").unwrap();
    symlink("gone", scratch.root("dangling")).unwrap();
    symlink("../outside", scratch.root("out")).unwrap();
    let root = scratch.root_fd();

    beneath::mkdir(root.as_fd(), p("d"), 0o755).unwrap();
    beneath::mkdir(root.as_fd(), p("d/e/"), 0o700).unwrap();
    assert!(scratch.root("d/e").is_dir());
    let mode = fs::metadata(scratch.root("d/e")).unwrap().mode() & 0o777;
    assert_eq!(mode, 0o700);

    for (path, expected) in [
        ("d", libc::EEXIST),
        ("f", libc::EEXIST),
        ("dangling", libc::EEXIST),
        ("missing/d", libc::ENOENT),
        ("f/d", libc::ENOTDIR),
        ("../d", libc::EXDEV),
        ("out/d", libc::EXDEV),
        ("/d", libc::EXDEV),
    ] {
        assert_eq!(
            code(beneath::mkdir(root.as_fd(), p(path), 0o755)),
            expected,
            "{path}"
        );
    }
    assert!(!scratch.root("gone").exists());
    assert!(!scratch.outside("d").exists());
}

/// `mkdir` in a directory the process may not write fails with `EACCES`,
/// unless the process is privileged past the mode.
#[test]
fn mkdir_without_permission() {
    let scratch = Scratch::new("mkdir_denied");
    fs::create_dir(scratch.root("locked")).unwrap();
    fs::set_permissions(scratch.root("locked"), fs::Permissions::from_mode(0o500)).unwrap();
    let root = scratch.root_fd();
    match beneath::mkdir(root.as_fd(), p("locked/d"), 0o755) {
        // a process with `CAP_DAC_OVERRIDE` writes past the mode
        Ok(()) => eprintln!("mkdir_without_permission: privileged, nothing to check"),
        Err(error) => assert_eq!(error.raw_os_error(), Some(libc::EACCES)),
    }
}

/// `remove_file` removes a file or a symlink, never its target, and fails
/// on a directory, a missing entry, a trailing `/`, and a path that leaves
/// the tree.
#[test]
fn remove_file_beneath() {
    let scratch = Scratch::new("remove_file");
    fs::create_dir(scratch.root("d")).unwrap();
    fs::write(scratch.root("d/f"), "").unwrap();
    fs::write(scratch.root("f"), "").unwrap();
    symlink("f", scratch.root("to_f")).unwrap();
    symlink("../outside", scratch.root("out")).unwrap();
    symlink("../outside/f", scratch.root("to_out")).unwrap();
    let root = scratch.root_fd();

    beneath::remove_file(root.as_fd(), p("d/f")).unwrap();
    assert!(!scratch.root("d/f").exists());
    beneath::remove_file(root.as_fd(), p("to_f")).unwrap();
    assert!(scratch.root("f").is_file());
    // a link pointing out is removed itself, its target untouched
    beneath::remove_file(root.as_fd(), p("to_out")).unwrap();
    assert!(scratch.outside("f").is_file());

    for (path, expected) in [
        ("d", libc::EISDIR),
        ("missing", libc::ENOENT),
        ("f/", libc::ENOTDIR),
        ("out/f", libc::EXDEV),
        ("../outside/f", libc::EXDEV),
    ] {
        assert_eq!(
            code(beneath::remove_file(root.as_fd(), p(path))),
            expected,
            "{path}"
        );
    }
    assert!(scratch.outside("f").is_file());
}

/// `remove_dir` removes an empty directory, and fails on one with entries,
/// on a file, on a symlink to a directory (with a trailing `/` or not), and
/// on a path that leaves the tree.
#[test]
fn remove_dir_beneath() {
    let scratch = Scratch::new("remove_dir");
    fs::create_dir_all(scratch.root("full/sub")).unwrap();
    fs::create_dir(scratch.root("empty")).unwrap();
    fs::create_dir(scratch.root("target")).unwrap();
    fs::write(scratch.root("f"), "").unwrap();
    symlink("target", scratch.root("link")).unwrap();
    fs::create_dir(scratch.outside("empty")).unwrap();
    symlink("../outside", scratch.root("out")).unwrap();
    let root = scratch.root_fd();

    beneath::remove_dir(root.as_fd(), p("empty/")).unwrap();
    assert!(!scratch.root("empty").exists());
    beneath::remove_dir(root.as_fd(), p("full/sub")).unwrap();

    fs::create_dir(scratch.root("full/sub")).unwrap();
    for (path, expected) in [
        ("full", libc::ENOTEMPTY),
        ("f", libc::ENOTDIR),
        ("link", libc::ENOTDIR),
        ("link/", libc::ENOTDIR),
        ("missing", libc::ENOENT),
        ("out/empty", libc::EXDEV),
    ] {
        assert_eq!(
            code(beneath::remove_dir(root.as_fd(), p(path))),
            expected,
            "{path}"
        );
    }
    assert!(scratch.root("target").is_dir());
    assert!(scratch.outside("empty").is_dir());
}

/// `rename` moves an entry within a tree and between two, replacing a file
/// at the target, moving a symlink itself, and fails as the kernel does on
/// a missing source, a kind mismatch, a directory with entries, and a side
/// that leaves its tree.
#[test]
fn rename_beneath() {
    let scratch = Scratch::new("rename");
    fs::create_dir_all(scratch.root("a")).unwrap();
    fs::create_dir_all(scratch.root("b")).unwrap();
    fs::create_dir_all(scratch.root("full/sub")).unwrap();
    fs::create_dir(scratch.root("d")).unwrap();
    fs::write(scratch.root("a/f"), "f").unwrap();
    fs::write(scratch.root("b/g"), "g").unwrap();
    symlink("../outside/f", scratch.root("to_out")).unwrap();
    symlink("../outside", scratch.root("out")).unwrap();
    let root = scratch.root_fd();
    let a = File::open(scratch.root("a")).unwrap();
    let b = File::open(scratch.root("b")).unwrap();

    beneath::rename(root.as_fd(), p("a/f"), root.as_fd(), p("a/f2")).unwrap();
    assert_eq!(fs::read_to_string(scratch.root("a/f2")).unwrap(), "f");
    // between two descriptors, replacing the file there
    beneath::rename(a.as_fd(), p("f2"), b.as_fd(), p("g")).unwrap();
    assert_eq!(fs::read_to_string(scratch.root("b/g")).unwrap(), "f");
    assert!(!scratch.root("a/f2").exists());
    beneath::rename(root.as_fd(), p("to_out"), root.as_fd(), p("a/moved")).unwrap();
    assert!(
        fs::symlink_metadata(scratch.root("a/moved"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(scratch.outside("f")).unwrap(), "outside");

    fs::write(scratch.root("f"), "").unwrap();
    for (from, to, expected) in [
        ("missing", "x", libc::ENOENT),
        ("f", "d", libc::EISDIR),
        ("d", "f", libc::ENOTDIR),
        ("d", "full", libc::ENOTEMPTY),
        ("f/", "x", libc::ENOTDIR),
        ("out/f", "x", libc::EXDEV),
        ("f", "out/f", libc::EXDEV),
        ("f", "../outside/f", libc::EXDEV),
    ] {
        let result = beneath::rename(root.as_fd(), p(from), root.as_fd(), p(to));
        assert_eq!(code(result), expected, "{from} {to}");
    }
    assert!(scratch.root("f").is_file());
    assert_eq!(fs::read_to_string(scratch.outside("f")).unwrap(), "outside");
}

/// A rename between two mounts fails without the raw `EXDEV` that means a
/// path leaving its tree, so `is_escape` tells the two apart. Skipped where
/// `/dev/shm` isn't another mount than the temporary directory.
#[test]
fn rename_across_mounts_is_no_escape() {
    let scratch = Scratch::new("cross_device");
    let shm = PathBuf::from(format!("/dev/shm/gro_sys_beneath_{}", std::process::id()));
    let Ok(()) = fs::create_dir(&shm) else {
        eprintln!("rename_across_mounts_is_no_escape: no /dev/shm, nothing to check");
        return;
    };
    let other_device =
        fs::metadata(&shm).unwrap().dev() != fs::metadata(scratch.root("")).unwrap().dev();
    fs::write(scratch.root("f"), "").unwrap();
    let root = scratch.root_fd();
    let shm_fd = File::open(&shm).unwrap();
    let result = beneath::rename(root.as_fd(), p("f"), shm_fd.as_fd(), p("f"));
    let _ = fs::remove_file(shm.join("f"));
    fs::remove_dir(&shm).unwrap();
    if !other_device {
        eprintln!("rename_across_mounts_is_no_escape: one mount, nothing to check");
        return;
    }
    let error = result.unwrap_err();
    assert_eq!(
        (error.kind(), error.raw_os_error()),
        (io::ErrorKind::CrossesDevices, None)
    );
    assert!(!beneath::is_escape(&error));
}

/// One of the calls on a last component, by the path it's given.
type Call<'a> = dyn Fn(&Path) -> io::Result<()> + 'a;

/// A last component that is empty, `.`, or `..`, or a path holding a NUL,
/// is refused before any call: with no OS code, though the parent named is
/// missing, and with nothing changed.
#[test]
fn a_bad_last_component_is_refused_before_any_call() {
    let scratch = Scratch::new("components");
    fs::create_dir(scratch.root("a")).unwrap();
    fs::write(scratch.root("a/f"), "").unwrap();
    let root = scratch.root_fd();
    let calls: [(&str, &Call); 5] = [
        ("mkdir", &|path| beneath::mkdir(root.as_fd(), path, 0o755)),
        ("remove_file", &|path| {
            beneath::remove_file(root.as_fd(), path)
        }),
        ("remove_dir", &|path| {
            beneath::remove_dir(root.as_fd(), path)
        }),
        ("rename from", &|path| {
            beneath::rename(root.as_fd(), path, root.as_fd(), p("x"))
        }),
        ("rename to", &|path| {
            beneath::rename(root.as_fd(), p("a/f"), root.as_fd(), path)
        }),
    ];
    for (name, call) in calls {
        for path in [
            "",
            "/",
            "//",
            ".",
            "..",
            "./",
            "../",
            "a/.",
            "a/..",
            "a/../",
            "a//.",
            "missing/..",
            "missing/.",
            "a/f\0",
            "a\0/f",
        ] {
            let error = call(p(path)).unwrap_err();
            assert_eq!(
                (error.kind(), error.raw_os_error()),
                (io::ErrorKind::InvalidInput, None),
                "{name} {path:?}: {error}"
            );
        }
    }
    assert!(scratch.root("a/f").is_file());
    assert!(!scratch.root("x").exists());
}
