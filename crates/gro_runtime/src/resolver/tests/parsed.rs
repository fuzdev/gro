//! The run's parsed `package.json` files: a file read again only when its
//! lookup finds another identity or a changed stamp, or one changed too
//! recently to keep, and every kept parse checked as a read is.

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::file_id::FileId;
use crate::resolver::parsed::SETTLED;
use crate::resolver::walk::{Consulted, read_package_json};
use crate::resolver::{
    Entry, ParsedPackages, Parsing, ReadRefusal, ResolveErrorKind, SourceTree, Stamp,
};

/// `/p` and its `package.json`, whose identity and stamp the test sets,
/// counting its reads, and refusing every read once `refused` is set.
struct Stamped {
    ino: Cell<u64>,
    stamp: Cell<Stamp>,
    refused: Cell<bool>,
    reads: Cell<usize>,
    /// How long each read takes.
    delay: Cell<Duration>,
}

impl Stamped {
    /// A file changed `age` before now.
    fn new(age: Duration) -> Self {
        Self {
            ino: Cell::new(1),
            stamp: Cell::new(changed(SystemTime::now() - age, 17)),
            refused: Cell::new(false),
            reads: Cell::new(0),
            delay: Cell::new(Duration::ZERO),
        }
    }

    /// Reads `/p/package.json` through `parsed`, returning whether it set
    /// the module type.
    fn read(&self, parsed: &ParsedPackages) -> Result<bool, ResolveErrorKind> {
        let tree = Parsing::new(self, parsed);
        let package_json = read_package_json(&tree, Path::new("/p"))?.unwrap();
        Ok(package_json.is_module_type(&mut Consulted::default()))
    }
}

/// The stamp of a file of `len` bytes changed at `at`.
fn changed(at: SystemTime, len: u64) -> Stamp {
    let since = at.duration_since(SystemTime::UNIX_EPOCH).unwrap();
    Stamp {
        changed_secs: i64::try_from(since.as_secs()).unwrap(),
        changed_nanos: i64::from(since.subsec_nanos()),
        len,
    }
}

impl SourceTree for Stamped {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        Ok(match path.to_str().unwrap() {
            "/" | "/p" => Some(Entry::Dir(path.to_owned())),
            "/p/package.json" => Some(Entry::File {
                path: path.to_owned(),
                id: FileId {
                    dev: 1,
                    ino: self.ino.get(),
                    born: None,
                },
                stamp: self.stamp.get(),
            }),
            _ => None,
        })
    }

    fn read(&self, path: &Path, _id: FileId) -> io::Result<Vec<u8>> {
        self.check_read(path)?;
        std::thread::sleep(self.delay.get());
        self.reads.set(self.reads.get() + 1);
        Ok(br#"{"type": "module"}"#.to_vec())
    }

    fn is_world_writable(&self, _dir: &Path) -> io::Result<bool> {
        Ok(false)
    }

    fn check_read(&self, path: &Path) -> io::Result<()> {
        if self.refused.get() {
            return Err(ReadRefusal::OutsideBox(path.to_owned()).into());
        }
        Ok(())
    }
}

/// A settled file is read and parsed once, every later read of the same
/// file served its parse; a tree that keeps none reads it each time.
#[test]
fn a_settled_file_is_read_once() {
    let tree = Stamped::new(SETTLED * 2);
    let parsed = ParsedPackages::default();
    assert_eq!(tree.read(&parsed), Ok(true));
    assert_eq!(tree.read(&parsed), Ok(true));
    assert_eq!(tree.reads.get(), 1);
    // with none kept, each read reaches the tree
    let package_json = read_package_json(&tree, Path::new("/p")).unwrap();
    assert!(package_json.is_some());
    read_package_json(&tree, Path::new("/p")).unwrap();
    assert_eq!(tree.reads.get(), 3);
    // nor is another run's kept
    tree.read(&ParsedPackages::default()).unwrap();
    assert_eq!(tree.reads.get(), 4);
}

/// Another identity, status-change time, or size at the path is another
/// file, or the same one changed, so it is read again, and kept again.
#[test]
fn another_identity_or_a_changed_stamp_is_read_again() {
    let tree = Stamped::new(SETTLED * 2);
    let parsed = ParsedPackages::default();
    tree.read(&parsed).unwrap();
    tree.ino.set(2);
    tree.read(&parsed).unwrap();
    assert_eq!(tree.reads.get(), 2);
    // each part of the stamp alone, from the stamp kept last
    let moved: [fn(Stamp) -> Stamp; 3] = [
        |stamp: Stamp| Stamp {
            changed_nanos: stamp.changed_nanos + 1,
            ..stamp
        },
        |stamp: Stamp| Stamp {
            changed_secs: stamp.changed_secs + 1,
            ..stamp
        },
        |stamp: Stamp| Stamp {
            len: stamp.len + 1,
            ..stamp
        },
    ];
    for (reads, move_stamp) in (3..).zip(moved) {
        tree.stamp.set(move_stamp(tree.stamp.get()));
        tree.read(&parsed).unwrap();
        assert_eq!(tree.reads.get(), reads);
    }
    // and the file as it now is, kept
    tree.read(&parsed).unwrap();
    assert_eq!(tree.reads.get(), 5);
}

/// A file changed less than the window before its read isn't kept, since
/// a rewrite within the same tick of the filesystem's clock would keep its
/// stamp: each read parses it again until it has settled. One changed in
/// the future, by the clock, isn't kept either.
#[test]
fn a_file_changed_within_the_window_is_read_each_time() {
    let tree = Stamped::new(SETTLED / 2);
    let parsed = ParsedPackages::default();
    tree.read(&parsed).unwrap();
    tree.read(&parsed).unwrap();
    assert_eq!(tree.reads.get(), 2);
    tree.stamp
        .set(changed(SystemTime::now() + Duration::from_secs(60), 17));
    tree.read(&parsed).unwrap();
    tree.read(&parsed).unwrap();
    assert_eq!(tree.reads.get(), 4);
    // settled, it is kept
    tree.stamp.set(changed(
        SystemTime::now() - SETTLED - Duration::from_secs(1),
        17,
    ));
    tree.read(&parsed).unwrap();
    tree.read(&parsed).unwrap();
    assert_eq!(tree.reads.get(), 5);
}

/// A kept parse is served only after the tree's check admits the read, so a
/// path refused since its first read is refused as a read of it is.
#[test]
fn a_kept_parse_is_checked_as_a_read_is() {
    let tree = Stamped::new(SETTLED * 2);
    let parsed = ParsedPackages::default();
    tree.read(&parsed).unwrap();
    tree.refused.set(true);
    assert_eq!(
        tree.read(&parsed),
        Err(ResolveErrorKind::ReadOutsideBox(PathBuf::from(
            "/p/package.json"
        )))
    );
    assert_eq!(tree.reads.get(), 1);
}

/// A file is kept by how long before its read it last changed, not before
/// the parse is kept: one that settles while it is read was read within the
/// window, so a rewrite in the same tick could follow the read. The file
/// settles a tenth of the way into a read of half the window, so a stall
/// before the read shorter than the rest still finds it unsettled.
#[test]
fn a_file_that_settles_while_it_is_read_is_read_again() {
    let delay = SETTLED / 2;
    let tree = Stamped::new(SETTLED.checked_sub(delay * 9 / 10).unwrap());
    tree.delay.set(delay);
    let parsed = ParsedPackages::default();
    tree.read(&parsed).unwrap();
    tree.delay.set(Duration::ZERO);
    tree.read(&parsed).unwrap();
    assert_eq!(tree.reads.get(), 2);
}

/// A lookup's stamp is the file's status-change time, which every write
/// moves and no call sets back, not its modification time, which a program
/// can set to anything: a rewrite to the same size, its modification time
/// set back, is read again.
#[test]
fn a_rewrite_with_its_modification_time_set_back_is_read_again() {
    let test_dir = crate::test_support::TestDir::on_disk("parsed_mtime");
    let dir = test_dir.path();
    let path = dir.join("package.json");
    let an_hour_ago = SystemTime::now()
        .checked_sub(Duration::from_secs(3600))
        .unwrap();
    let write = |contents: &str| {
        std::fs::write(&path, contents).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(an_hour_ago)
            .unwrap();
    };
    let is_module = |parsed: &ParsedPackages| {
        let tree = Parsing::new(&crate::resolver::DiskTree, parsed);
        read_package_json(&tree, dir)
            .unwrap()
            .unwrap()
            .is_module_type(&mut Consulted::default())
    };
    let parsed = ParsedPackages::default();
    write(r#"{"type": "module"  }"#);
    assert!(is_module(&parsed));
    write(r#"{"type": "commonjs"}"#);
    assert!(!is_module(&parsed));
}
