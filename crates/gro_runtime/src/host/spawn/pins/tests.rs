use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use super::{
    Fault, Pin, PinKind, is_package_bin_entry, is_package_bin_name, is_refused_extension,
    node_flags, resolve, to_project_path,
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

/// A program name is a `node_modules/.bin` program's by its last three
/// segments, resolved by their text, however it's spelled, and no other is.
#[test]
fn a_package_bin_name_is_known_by_its_spelling() {
    for name in [
        "node_modules/.bin/vitest",
        "./node_modules/.bin/vitest",
        "node_modules/./.bin//vitest",
        "packages/a/node_modules/.bin/x",
        "node_modules/x/../.bin/vitest",
    ] {
        assert!(is_package_bin_name(name), "{name}");
    }
    for name in [
        "node_modules/.bin",
        "node_modules/.bin/",
        "node_modules/.bin/x/y",
        "node_modules/.bin/x/..",
        "bin/vitest",
        ".bin/vitest",
        "node_modules/vitest",
    ] {
        assert!(!is_package_bin_name(name), "{name}");
    }
}

/// A `node_modules/.bin` script's line is `#!/usr/bin/env node`, or
/// `#!/usr/bin/env -S node` with plain flags, split at spaces and tabs;
/// any other line passes no flags, since `env` would read it otherwise.
#[test]
fn a_node_line_is_env_node_or_env_s_node_with_plain_flags() {
    let flags = |line: &str| node_flags(line.as_bytes());
    let none: Vec<String> = Vec::new();
    assert_eq!(flags("#!/usr/bin/env node"), Some(none.clone()));
    assert_eq!(flags("#! /usr/bin/env\tnode  "), Some(none.clone()));
    assert_eq!(flags("#!/usr/bin/env -S node"), Some(none));
    assert_eq!(
        flags(
            "#!/usr/bin/env -S node --experimental-import-meta-resolve --max-old-space-size=4096"
        ),
        Some(vec![
            "--experimental-import-meta-resolve".to_owned(),
            "--max-old-space-size=4096".to_owned()
        ])
    );
    for refused in [
        "#!/bin/sh",
        "#!/usr/bin/node",
        "#!/usr/bin/env nodejs",
        "#!/usr/bin/env node --flag",
        "#!/usr/bin/env -S node script.js",
        "#!/usr/bin/env -S node --x=${HOME}",
        "#!/usr/bin/env -S node '--x'",
        "#!/usr/bin/env -S node --x\\y",
        "#!/usr/bin/env -S node --x #comment",
        "#!/usr/bin/env -S -i node",
        "#!/usr/bin/env -Snode",
        "#!/usr/bin/env node\r",
        "#!/usr/local/bin/env node",
        "#!/usr/bin/env",
        "/usr/bin/env node",
    ] {
        assert_eq!(flags(refused), None, "{refused:?}");
    }
    assert_eq!(node_flags(b"#!/usr/bin/env -S node --\xff"), None);
}

/// The `node` pinned beside a `node_modules/.bin` script, or why there's
/// none.
fn node_of(pin: &Pin) -> (&Pin, &[String]) {
    match &pin.kind {
        PinKind::NodeScript { node, flags } => (node, flags),
        PinKind::Native(_) | PinKind::Script => panic!("{} runs without its node", pin.real),
    }
}

/// A script at a `node_modules/.bin` name is pinned with the `node` the
/// walk finds, past a `node` planted in a `node_modules/.bin` entry, to
/// run by its real path with its line's flags; a `node` that is a script
/// is pinned as one; a native binary there, and a script reached by any
/// other name, run as before; and a line naming anything else, or no
/// `node` to find, is `not_executable`.
#[test]
fn a_package_bin_script_is_pinned_with_its_node() {
    let dir = TestDir::in_system_temp("spawn_pins_node");
    let root = dir.path();
    let project = text(root);
    file(
        root,
        "pkg/cli.js",
        b"#!/usr/bin/env node\nconsole.log(1);\n",
        0o755,
    );
    file(
        root,
        "pkg/flagged.js",
        b"#!/usr/bin/env -S node --a=1 --b\n",
        0o755,
    );
    file(root, "pkg/shell.sh", SCRIPT, 0o755);
    file(
        root,
        "pkg/long.js",
        &[b"#!/usr/bin/env node".as_slice(), &[b' '; 300], b"\n"].concat(),
        0o755,
    );
    file(root, "node_modules/.bin/native", ELF, 0o755);
    file(root, "node_modules/.bin/node", ELF, 0o755);
    file(root, "tools/node", ELF, 0o755);
    file(root, "shims/node", SCRIPT, 0o755);
    for (bin, target) in [
        ("cli", "cli.js"),
        ("flagged", "flagged.js"),
        ("shell", "shell.sh"),
        ("long", "long.js"),
    ] {
        std::os::unix::fs::symlink(
            format!("../../pkg/{target}"),
            root.join("node_modules/.bin").join(bin),
        )
        .unwrap();
    }
    let path = format!("{project}/node_modules/.bin:{project}/tools:/usr/bin:/bin");
    let pin = resolve("node_modules/.bin/cli", Some(&path), project).unwrap();
    assert_eq!(pin.real, format!("{project}/pkg/cli.js"));
    assert_eq!(pin.run_path, pin.real);
    let (node, flags) = node_of(&pin);
    assert_eq!(node.real, format!("{project}/tools/node"));
    assert_eq!(node.argv0, "node");
    assert!(is_native(node));
    assert!(flags.is_empty());
    let spelled = resolve("./node_modules/./.bin/cli", Some(&path), project).unwrap();
    assert!(pin.same(&spelled));
    let pin = resolve("node_modules/.bin/flagged", Some(&path), project).unwrap();
    assert_eq!(node_of(&pin).1, ["--a=1", "--b"]);
    let shimmed = format!("{project}/shims:{project}/tools");
    let pin = resolve("node_modules/.bin/cli", Some(&shimmed), project).unwrap();
    let (node, _) = node_of(&pin);
    assert!(matches!(node.kind, PinKind::Script));
    assert_eq!(node.run_path, format!("{project}/shims/node"));
    assert!(is_native(
        &resolve("node_modules/.bin/native", Some(&path), project).unwrap()
    ));
    let by_path = resolve("pkg/cli.js", Some(&path), project).unwrap();
    assert!(matches!(by_path.kind, PinKind::Script));
    for (name, line) in [("shell", "#!/bin/sh"), ("long", "#!/usr/bin/env node")] {
        match resolve(&format!("node_modules/.bin/{name}"), Some(&path), project) {
            Err(Fault::NotExecutable(cause)) => assert!(
                cause.starts_with(&format!("node_modules/.bin/{name} at {project}/pkg/"))
                    && cause.contains(&format!(" starts with `{line}")),
                "{cause}"
            ),
            other => panic!("{name}: {other:?}"),
        }
    }
    let shell = resolve("node_modules/.bin/shell", Some(&path), project).unwrap_err();
    assert_eq!(
        shell,
        Fault::NotExecutable(format!(
            "node_modules/.bin/shell at {project}/pkg/shell.sh starts with `#!/bin/sh`, and a \
             node_modules/.bin script runs pinned only under node, by `#!/usr/bin/env node` or \
             `#!/usr/bin/env -S node` and its flags"
        ))
    );
    let planted_only = format!("{project}/node_modules/.bin");
    assert_eq!(
        resolve("node_modules/.bin/cli", Some(&planted_only), project).unwrap_err(),
        Fault::NotExecutable(format!(
            "node_modules/.bin/cli at {project}/pkg/cli.js runs under node, and no PATH entry \
             outside a node_modules/.bin holds one"
        ))
    );
    file(root, "text/node", b"echo hi\n", 0o755);
    let text_node = format!("{project}/text");
    assert!(matches!(
        resolve("node_modules/.bin/cli", Some(&text_node), project),
        Err(Fault::Unresolved { cause, .. })
            if cause == format!("node_modules/.bin/cli at {project}/pkg/cli.js runs under node, which can't be pinned")
    ));
}

/// A `node_modules/.bin` script's launch checks its real path still leads
/// to the file pinned, and then its `node`, as its own pin: either one
/// replaced since refuses the launch.
#[test]
fn a_package_bin_script_s_launch_checks_it_and_its_node() {
    let dir = TestDir::in_system_temp("spawn_pins_node_launch");
    let root = dir.path();
    let project = text(root);
    let line = b"#!/usr/bin/env node\n";
    file(root, "pkg/cli.js", line, 0o755);
    file(root, "pkg/other.js", line, 0o755);
    file(root, "shims/node", SCRIPT, 0o755);
    file(root, "shims/other", SCRIPT, 0o755);
    std::fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
    std::os::unix::fs::symlink("../../pkg/cli.js", root.join("node_modules/.bin/cli")).unwrap();
    let path = format!("{project}/shims");
    let pinned = || resolve("node_modules/.bin/cli", Some(&path), project).unwrap();
    let pin = pinned();
    assert!(pin.check_launch("cli").is_ok());
    std::fs::rename(root.join("pkg/other.js"), root.join("pkg/cli.js")).unwrap();
    let refused = pin.check_launch("cli").unwrap_err();
    assert_eq!(
        refused.cause,
        format!("cli at {project}/pkg/cli.js is no longer the file pinned")
    );
    let pin = pinned();
    assert!(pin.check_launch("cli").is_ok());
    std::fs::rename(root.join("shims/other"), root.join("shims/node")).unwrap();
    let refused = pin.check_launch("cli").unwrap_err();
    assert_eq!(
        refused.cause,
        format!("node at {project}/shims/node is no longer the file pinned")
    );
}

/// A script's launch names what its found path now leads to when that's
/// another file's real path, and says only that it's no longer the file
/// pinned when another file was renamed over the pinned one.
#[test]
fn a_replaced_script_is_named_by_where_its_path_now_leads() {
    let dir = TestDir::in_system_temp("spawn_pins_replaced");
    let root = dir.path();
    let project = text(root);
    file(root, "bin/a.sh", SCRIPT, 0o755);
    file(root, "bin/b.sh", SCRIPT, 0o755);
    file(root, "bin/c.sh", SCRIPT, 0o755);
    std::os::unix::fs::symlink("a.sh", root.join("bin/link")).unwrap();
    let pin = resolve("bin/link", None, project).unwrap();
    std::fs::remove_file(root.join("bin/link")).unwrap();
    std::os::unix::fs::symlink("b.sh", root.join("bin/link")).unwrap();
    assert_eq!(
        pin.check_launch("link").unwrap_err().cause,
        format!(
            "link at {project}/bin/link now leads to {project}/bin/b.sh, not the file pinned at \
             {project}/bin/a.sh"
        )
    );
    let pin = resolve("bin/a.sh", None, project).unwrap();
    std::fs::rename(root.join("bin/c.sh"), root.join("bin/a.sh")).unwrap();
    assert_eq!(
        pin.check_launch("a").unwrap_err().cause,
        format!("a at {project}/bin/a.sh is no longer the file pinned")
    );
}

/// A pin names the file the run's box grants: a native binary and a script
/// reached by a plain name their own, a `node_modules/.bin` script the
/// `node` it runs under, native or a script, and a script granted as
/// `node` itself a script `node`, whichever grant names it.
#[test]
fn a_pin_names_the_file_the_box_grants() {
    use std::path::PathBuf;

    use crate::confine::plan::ProgramFile;

    let dir = TestDir::in_system_temp("spawn_pins_files");
    let root = dir.path();
    let project = text(root);
    file(root, "tools/native", ELF, 0o755);
    file(root, "tools/script", SCRIPT, 0o755);
    file(root, "tools/node", ELF, 0o755);
    file(root, "shims/node", SCRIPT, 0o755);
    file(root, "pkg/cli.js", b"#!/usr/bin/env node\n", 0o755);
    std::fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
    std::os::unix::fs::symlink("../../pkg/cli.js", root.join("node_modules/.bin/cli")).unwrap();
    let at = |path: &str| PathBuf::from(format!("{project}/{path}"));
    let file_of = |name: &str, path: &str| {
        ProgramFile::from(&resolve(name, Some(&format!("{project}/{path}")), project).unwrap())
    };
    assert_eq!(
        file_of("native", "tools"),
        ProgramFile::Direct {
            real: at("tools/native")
        }
    );
    assert_eq!(
        file_of("script", "tools"),
        ProgramFile::Direct {
            real: at("tools/script")
        }
    );
    assert_eq!(
        file_of("node_modules/.bin/cli", "tools"),
        ProgramFile::Node {
            found: at("tools/node"),
            real: at("tools/node"),
            script: false,
        }
    );
    assert_eq!(
        file_of("node_modules/.bin/cli", "shims"),
        ProgramFile::Node {
            found: at("shims/node"),
            real: at("shims/node"),
            script: true,
        }
    );
    assert_eq!(
        file_of("node", "shims"),
        ProgramFile::Node {
            found: at("shims/node"),
            real: at("shims/node"),
            script: true,
        }
    );
    assert_eq!(
        file_of("node", "tools"),
        ProgramFile::Direct {
            real: at("tools/node")
        }
    );
}
