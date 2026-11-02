use std::io;
use std::path::Path;

use url::Url;

use super::{from_entry, importer, packages, resolve, roots, tree};
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{
    DiskTree, Entry, FileId, MAX_PACKAGE_JSON_SIZE, ReadRefusal, ResolveErrorKind, SourceTree,
    resolve_entry,
};
use crate::test_support::TestDir;

/// Whether `error` is the refusal of a file replaced since its lookup.
fn is_replaced(error: &io::Error) -> bool {
    ReadRefusal::of(error) == Some(&ReadRefusal::ReplacedSinceLookup)
}

/// A resolution carries the identity of the file it found, its real
/// file's when reached through a symlink, the entry's included.
#[test]
fn a_resolution_carries_its_file_id() {
    let tree = tree();
    let id = |path| tree.id(path).unwrap();
    let file_id = |specifier| from_entry(&tree, specifier, &importer()).unwrap().file_id;
    assert_eq!(file_id("./a.js"), id("/p/a.js"));
    assert_eq!(file_id("./b.mjs"), id("/p/b.mjs"));
    assert_ne!(id("/p/a.js"), id("/p/b.mjs"));
    assert_eq!(file_id("./link.js"), id("/p/real/linked.js"));
    assert_eq!(file_id("./real/linked.js"), id("/p/real/linked.js"));
    let (entry, _) = resolve_entry(&tree, Path::new("main.js"), Path::new("/p")).unwrap();
    assert_eq!(entry.file_id, id("/p/main.js"));
    // a bare specifier's target too
    let packages = packages();
    let importer = Url::parse("file:///w/proj/main.js").unwrap();
    let resolution = from_entry(&packages, "t", &importer).unwrap();
    assert_eq!(
        Some(resolution.file_id),
        packages.id("/w/proj/node_modules/t/main.js")
    );
}

/// A read checks the identity the lookup found, so a `package.json`
/// replaced in between isn't read.
#[test]
fn a_read_refuses_another_file() {
    let tree = tree();
    let path = Path::new("/p/package.json");
    let Some(Entry::File { id, .. }) = tree.lookup(path).unwrap() else {
        panic!("/p/package.json is a file");
    };
    assert!(tree.read(path, id).is_ok());
    let other = tree.id("/p/a.js").unwrap();
    assert!(is_replaced(&tree.read(path, other).unwrap_err()));
}

/// A tree whose every read opens a file other than the one looked up, as
/// if each were replaced in between.
struct Replaced(MemoryTree);

impl SourceTree for Replaced {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        self.0.lookup(path)
    }

    fn read(&self, path: &Path, _: FileId) -> io::Result<Vec<u8>> {
        let other = FileId {
            dev: 0,
            ino: 0,
            born: None,
        };
        self.0.read(path, other)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        self.0.is_world_writable(dir)
    }
}

/// A resolution whose `package.json` read is refused by its identity fails
/// with the refusal's kind, a failure of the tree rather than a refusal of
/// the import.
#[test]
fn a_resolution_names_a_replaced_package_json() {
    let roots = roots(&tree(), &importer());
    let error = resolve(&Replaced(tree()), "./a.js", &importer(), &roots).unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::ReadReplacedSinceLookup("/p/package.json".into())
    );
    assert!(!error.kind.is_refusal());
}

/// On disk, a lookup takes the file's device and inode, and a read
/// after the file was renamed over refuses it.
#[test]
fn a_disk_read_refuses_a_replaced_file() {
    use std::os::unix::fs::MetadataExt as _;
    let dir = TestDir::on_disk("resolver_id");
    let path = dir.path().join("package.json");
    std::fs::write(&path, "{}").unwrap();
    let metadata = std::fs::metadata(&path).unwrap();
    let entry = DiskTree.lookup(&path).unwrap();
    let id = match &entry {
        Some(Entry::File { id, .. }) => *id,
        _ => FileId {
            dev: 0,
            ino: 0,
            born: None,
        },
    };
    let before = DiskTree.read(&path, id);
    // made before the rename, so it can't reuse the replaced file's inode
    let replacement = dir.path().join("replacement.json");
    std::fs::write(&replacement, "{}").unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    let after = DiskTree.read(&path, id);
    assert_eq!(
        entry,
        Some(Entry::File {
            path,
            id: FileId {
                dev: metadata.dev(),
                ino: metadata.ino(),
                born: metadata.created().ok(),
            },
        })
    );
    assert_eq!(before.unwrap(), b"{}");
    assert!(is_replaced(&after.unwrap_err()));
}

/// A file with the device and inode looked up but another birth time, as
/// one remade on a freed inode number has, is refused.
#[test]
fn a_disk_read_refuses_another_birth_time() {
    let dir = TestDir::on_disk("resolver_born");
    let path = dir.path().join("package.json");
    std::fs::write(&path, "{}").unwrap();
    let id = match DiskTree.lookup(&path).unwrap() {
        Some(Entry::File { id, .. }) => Some(id),
        _ => None,
    };
    let remade = id.map(|id| FileId {
        born: id
            .born
            .map(|born| born + std::time::Duration::from_nanos(1)),
        ..id
    });
    let same = id.map(|id| DiskTree.read(&path, id));
    let other = remade.map(|remade| DiskTree.read(&path, remade));
    assert!(
        id.is_some_and(|id| id.born.is_some()),
        "this filesystem records birth times"
    );
    assert_eq!(same.unwrap().unwrap(), b"{}");
    assert!(is_replaced(&other.unwrap().unwrap_err()));
}

#[test]
fn disk_reads_are_bounded() {
    let dir = TestDir::in_system_temp("resolver_bounded");
    let path = dir.path().join("package.json");
    let file = std::fs::File::create(&path).unwrap();
    // sparse, so nothing is written; refused before any allocation of
    // its size, which would abort the process
    file.set_len(1 << 40).unwrap();
    let id = FileId::of(&file.metadata().unwrap()).unwrap();
    let huge = DiskTree.read(&path, id).unwrap_err().kind();
    file.set_len(MAX_PACKAGE_JSON_SIZE + 1).unwrap();
    let too_large = DiskTree.read(&path, id).unwrap_err().kind();
    file.set_len(MAX_PACKAGE_JSON_SIZE).unwrap();
    let at_most = DiskTree.read(&path, id).map(|bytes| bytes.len() as u64);
    assert_eq!(huge, io::ErrorKind::FileTooLarge);
    assert_eq!(too_large, io::ErrorKind::FileTooLarge);
    assert_eq!(at_most.unwrap(), MAX_PACKAGE_JSON_SIZE);
}
