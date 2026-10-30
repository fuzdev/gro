use std::io;
use std::path::Path;

use url::Url;

use super::{from_entry, importer, packages, tree};
use crate::resolver::{DiskTree, Entry, FileId, MAX_PACKAGE_JSON_SIZE, SourceTree, resolve_entry};

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
    assert_eq!(
        tree.read(path, other).unwrap_err().to_string(),
        FileId::MISMATCH
    );
}

/// On disk, a lookup takes the file's device and inode, and a read
/// after the file was renamed over refuses it.
#[test]
fn a_disk_read_refuses_a_replaced_file() {
    use std::os::unix::fs::MetadataExt as _;
    let dir = std::env::temp_dir().join(format!("gro_resolver_id_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.canonicalize().unwrap().join("package.json");
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
    let replacement = dir.join("replacement.json");
    std::fs::write(&replacement, "{}").unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    let after = DiskTree.read(&path, id);
    // removed before any assertion, so a failure leaves nothing behind
    std::fs::remove_dir_all(&dir).unwrap();
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
    assert_eq!(after.unwrap_err().to_string(), FileId::MISMATCH);
}

#[test]
fn disk_reads_are_bounded() {
    let path =
        std::env::temp_dir().join(format!("gro_resolver_bounded_{}.json", std::process::id()));
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
    std::fs::remove_file(&path).unwrap();
    assert_eq!(huge, io::ErrorKind::FileTooLarge);
    assert_eq!(too_large, io::ErrorKind::FileTooLarge);
    assert_eq!(at_most.unwrap(), MAX_PACKAGE_JSON_SIZE);
}
