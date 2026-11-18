use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use super::{
    Fault, Pin, PinKind, is_package_bin_entry, is_refused_extension, resolve, to_project_path,
};
use crate::test_support::TestDir;

/// Writes `bytes` at `dir`/`name` with `mode`, whatever the umask.
fn file(dir: &Path, name: &str, bytes: &[u8], mode: u32) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// `dir`'s path as text.
fn text(dir: &Path) -> &str {
    dir.to_str().unwrap()
}

/// A native binary's first bytes.
const ELF: &[u8] = b"\x7fELF\x02\x01\x01";

/// A script's.
const SCRIPT: &[u8] = b"#!/bin/sh\nexit 0\n";

/// Whether `pin` is native, holding a descriptor.
const fn is_native(pin: &Pin) -> bool {
    matches!(pin.kind, PinKind::Native(_))
}

/// A `PATH` entry spelled as a `node_modules/.bin` directory is one, by
/// its text alone, however it's spelled, and no other is.
#[test]
fn a_package_bin_entry_is_known_by_its_spelling() {
    for entry in [
        "/p/node_modules/.bin",
        "/p/node_modules/.bin/",
        "/p/node_modules/./.bin",
        "/p/node_modules/x/../.bin",
        "/node_modules/.bin//",
    ] {
        assert!(is_package_bin_entry(entry), "{entry}");
    }
    for entry in [
        "/p/.bin",
        "/p/node_modules",
        "/p/node_modules/.bin/x",
        "/p/node_modules/.bin/..",
    ] {
        assert!(!is_package_bin_entry(entry), "{entry}");
    }
}

/// A name holding `/` resolves beneath the project by its text, and one
/// absolute or climbing above it is `not_granted`.
#[test]
fn a_name_with_a_slash_resolves_beneath_the_project_by_its_text() {
    assert_eq!(to_project_path("./bin/x", "/p"), Ok("/p/bin/x".to_owned()));
    assert_eq!(to_project_path("bin//./x", "/p"), Ok("/p/bin/x".to_owned()));
    assert_eq!(to_project_path("a/../x/", "/p"), Ok("/p/x".to_owned()));
    assert_eq!(to_project_path("./x", "/"), Ok("/x".to_owned()));
    assert!(matches!(
        to_project_path("/bin/x", "/p"),
        Err(Fault::NotGranted(_))
    ));
    assert!(matches!(
        to_project_path("../x", "/p"),
        Err(Fault::NotGranted(_))
    ));
    assert!(matches!(
        to_project_path("a/../../x", "/p"),
        Err(Fault::NotGranted(_))
    ));
}

/// A Windows shell's extension is refused however it's cased and padded,
/// as the Node module's pattern refuses it.
#[test]
fn a_shell_extension_is_known_as_the_node_module_knows_it() {
    for path in ["x.bat", "x.CMD", "x.Bat. .", "/a/.cmd", "x.cmd  "] {
        assert!(is_refused_extension(path), "{path}");
    }
    for path in ["x.batch", "xbat", "x.bat.sh", "x.cm d", "x"] {
        assert!(!is_refused_extension(path), "{path}");
    }
}

/// A bare name walks the `PATH` given, absolute entries only, past each
/// `node_modules/.bin` and each entry with nothing runnable of the name, to
/// the first file it may execute; nothing there is `not_found`, and a file
/// of the name it may not execute, with none it may, `not_executable`.
#[test]
fn a_bare_name_walks_the_path_to_the_first_it_may_run() {
    let dir = TestDir::in_system_temp("spawn_pins_walk");
    let root = dir.path();
    file(root, "node_modules/.bin/tool", ELF, 0o755);
    file(root, "denied/tool", ELF, 0o644);
    file(root, "found/tool", ELF, 0o755);
    file(root, "later/tool", ELF, 0o755);
    std::fs::create_dir_all(root.join("dir/tool")).unwrap();
    let path = format!(
        "relative:{0}/node_modules/.bin:{0}/dir:{0}/denied:{0}/found/:{0}/later",
        text(root)
    );
    let pin = resolve("tool", Some(&path), "/").unwrap();
    assert_eq!(pin.real, format!("{}/found/tool", text(root)));
    assert_eq!(pin.argv0, "tool");
    assert!(is_native(&pin));
    assert_eq!(
        resolve("missing", Some(&path), "/").err(),
        Some(Fault::NotFound)
    );
    assert_eq!(resolve("tool", None, "/").err(), Some(Fault::NotFound));
    let only_denied = format!("{}/denied", text(root));
    assert!(matches!(
        resolve("tool", Some(&only_denied), "/"),
        Err(Fault::NotExecutable(_))
    ));
}

/// A file is pinned at its real path by what its first bytes are: a `#!`
/// script runs by the path it was found at, a native binary by its real
/// path, and a file this user can't read passes as native; any other
/// format, and a `.cmd`, are `not_executable`.
#[test]
fn a_file_is_pinned_by_its_first_bytes() {
    let dir = TestDir::in_system_temp("spawn_pins_format");
    let root = dir.path();
    file(root, "bin/script", SCRIPT, 0o755);
    file(root, "bin/native", ELF, 0o755);
    file(root, "bin/text", b"echo hi\n", 0o755);
    file(root, "bin/short", b"#", 0o755);
    file(root, "bin/empty", b"", 0o755);
    file(root, "bin/run.cmd", SCRIPT, 0o755);
    file(root, "bin/unreadable", SCRIPT, 0o311);
    std::os::unix::fs::symlink("script", root.join("bin/link")).unwrap();
    let project = text(root);
    let pin = resolve("./bin/link", None, project).unwrap();
    assert!(!is_native(&pin));
    assert_eq!(pin.real, format!("{project}/bin/script"));
    assert_eq!(pin.run_path, format!("{project}/bin/link"));
    assert_eq!(pin.argv0, format!("{project}/bin/link"));
    let pin = resolve("bin/native", None, project).unwrap();
    assert!(is_native(&pin));
    assert_eq!(pin.run_path, pin.real);
    // a `.cmd` name is refused before anything is looked up
    for name in [
        "bin/text",
        "bin/short",
        "bin/empty",
        "bin/run.cmd",
        "bin/RUN.CMD.",
    ] {
        let resolved = resolve(name, None, project);
        assert!(
            matches!(resolved, Err(Fault::NotExecutable(_))),
            "{name}: {resolved:?}"
        );
    }
    // root reads what it can't execute: only a user that can't read it sees one
    if std::fs::read(root.join("bin/unreadable")).is_err() {
        assert!(is_native(
            &resolve("bin/unreadable", None, project).unwrap()
        ));
    }
    for name in [
        "bin/missing",
        "bin/",
        "bin/.",
        "bin/script/",
        "bin/script/.",
    ] {
        assert_eq!(
            resolve(name, None, project).err(),
            Some(Fault::NotFound),
            "{name}"
        );
    }
    assert!(matches!(
        resolve("../x", None, project),
        Err(Fault::NotGranted(_))
    ));
}

/// A native pin holds its file: one renamed over its path after it's
/// pinned leaves the pin's identity and descriptor on the file pinned.
#[test]
fn a_native_pin_holds_the_file_pinned() {
    let dir = TestDir::in_system_temp("spawn_pins_held");
    let root = dir.path();
    file(root, "bin/tool", ELF, 0o755);
    file(root, "bin/other", ELF, 0o755);
    let pin = resolve("bin/tool", None, text(root)).unwrap();
    let before = pin.id;
    std::fs::rename(root.join("bin/other"), root.join("bin/tool")).unwrap();
    let PinKind::Native(held) = &pin.kind else {
        panic!("a native binary is held");
    };
    let held = std::fs::File::from(held.try_clone().unwrap());
    assert_eq!(
        crate::file_id::FileId::of(&held.metadata().unwrap()),
        before
    );
    let again = resolve("bin/tool", None, text(root)).unwrap();
    assert_ne!(again.id, before);
    assert!(!pin.same(&again));
    assert!(again.same(&resolve("bin/tool", None, text(root)).unwrap()));
}
