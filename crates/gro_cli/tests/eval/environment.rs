//! What eval leaves alone and ignores: the project tree it leaves unchanged and
//! the environment it clears.

use std::path::{Path, PathBuf};

use crate::support::Scratch;
use crate::{command, eval, parse, spawn};

/// A project, `project/`, whose module imports a package linked into its
/// `node_modules` from `lib/` beside it and reads what the host could
/// change: the locale's formats, the time zone, the clock, and its URL.
fn host_project(name: &str) -> Scratch {
    let scratch = Scratch::new(
        name,
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/config.js",
                "import {value} from '@x/lib/value.js';\n\
                 const format = new Intl.DateTimeFormat().resolvedOptions();\n\
                 let clock;\n\
                 try { clock = typeof Date.now(); } catch (error) { clock = error.message; }\n\
                 console.log('logged');\n\
                 export default {\n\
                 \tvalue,\n\
                 \tnumber: (1234567.891).toLocaleString(),\n\
                 \tdate: new Date(0).toString(),\n\
                 \tlocale: format.locale,\n\
                 \ttime_zone: format.timeZone,\n\
                 \tupper: 'i'.toLocaleUpperCase(),\n\
                 \tsorted: ['z', '\\u00e4', 'a'].sort(new Intl.Collator().compare),\n\
                 \tclock,\n\
                 \turl: import.meta.url,\n\
                 };\n",
            ),
            (
                "lib/package.json",
                r#"{"name": "@x/lib", "type": "module", "exports": {"./value.js": "./value.js"}}"#,
            ),
            ("lib/value.js", "export const value = 'linked';\n"),
        ],
    );
    std::fs::create_dir_all(scratch.0.join("project/node_modules/@x")).unwrap();
    std::os::unix::fs::symlink(
        "../../../lib",
        scratch.0.join("project/node_modules/@x/lib"),
    )
    .unwrap();
    scratch
}

/// Every entry under `root`, links not followed, by its path relative to
/// `root`: its kind, mode, owner, size, modification and change times, and
/// its content's BLAKE3 or its link's target. Access times are left out, a
/// read's only trace.
fn tree_state(root: &Path) -> std::collections::BTreeMap<PathBuf, String> {
    use std::os::unix::fs::MetadataExt as _;

    let mut state = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let content = if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
            "dir".to_owned()
        } else if metadata.is_symlink() {
            format!("link {}", std::fs::read_link(&path).unwrap().display())
        } else {
            format!("file {}", blake3::hash(&std::fs::read(&path).unwrap()))
        };
        state.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            format!(
                "{content} mode {:o} owner {}:{} size {} mtime {}.{} ctime {}.{}",
                metadata.mode(),
                metadata.uid(),
                metadata.gid(),
                metadata.size(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec()
            ),
        );
    }
    state
}

/// An eval writes nothing: a project with a linked package, evaluated
/// jitless and with the JIT, with the graph, and failing, is left as it was,
/// the package's directory too, with no file, cache, lockfile, or
/// `node_modules` added, and no entry's content, mode, or times changed.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn an_eval_leaves_the_project_tree_unchanged() {
    let scratch = host_project("unchanged");
    std::fs::write(
        scratch.0.join("project/throws.js"),
        "import '@x/lib/value.js';\nthrow new Error('thrown');\n",
    )
    .unwrap();
    let before = tree_state(&scratch.0);
    let config = scratch.path("project/config.js");
    for args in [
        vec![config.as_str(), "--graph"],
        vec![config.as_str(), "--jit"],
    ] {
        let run = eval(&args);
        assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
        assert_eq!(run.envelope["value"]["value"], "linked", "{}", run.stdout);
    }
    eval(&[&scratch.path("project/throws.js"), "--graph"]).assert_failed("module_threw", 1);
    assert_eq!(tree_state(&scratch.0), before);
}

/// Eval clears its environment but `TZ`, so no variable configures it: a
/// run under a broad hostile environment, the variables its dependencies
/// and neighbors read (`deno_core`'s diagnostics and import graph file,
/// Deno's and Node's own, Rust's, tokio's, glibc's and the dynamic loader's,
/// ICU's, the locale and color, the home and temporary directories, and
/// gro's own build-time and feature-gated names), writes the same envelope,
/// byte for byte, as a run with `TZ=UTC` alone, and the same stderr but for
/// the loader's warning, which comes at exec, before eval runs, and creates
/// no file a variable names. `TZ` itself is read, and refused when not UTC,
/// which the time zone tests cover.
///
/// Left out: variables only a debugger or a sanitizer build reads.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_environment_configures_nothing() {
    let scratch = host_project("hostile_env");
    let elsewhere = scratch.0.join("elsewhere");
    for dir in ["home", "tmp", "cache", "config", "runtime", "icu", "empty"] {
        std::fs::create_dir_all(elsewhere.join(dir)).unwrap();
    }
    let at = |file: &str| elsewhere.join(file).display().to_string();
    let named_files = [
        at("phases.txt"),
        at("node_repl_history"),
        at("import_graph.jsonl"),
    ];
    let hostile: Vec<(&str, String)> = vec![
        // deno_core's logs on stderr, and the file it appends each import
        // to, whose open inside the box would panic
        ("DENO_LOG_LAZY_LOAD", "1".to_owned()),
        ("DENO_STARTUP_PHASES", "1".to_owned()),
        ("DENO_SNAPSHOT_IMPORT_GRAPH", named_files[2].clone()),
        // Deno's and Node's own
        (
            "DENO_V8_FLAGS",
            "--allow-natives-syntax --no-jitless --max-old-space-size=8".to_owned(),
        ),
        ("DENO_DIR", at("cache")),
        ("DENO_NO_PROMPT", "1".to_owned()),
        ("DENO_TLS_CA_STORE", "system".to_owned()),
        ("DENO_FUTURE", "1".to_owned()),
        (
            "NODE_OPTIONS",
            "--max-old-space-size=8 --stack-size=10 --require /nonexistent/gro_hostile.js"
                .to_owned(),
        ),
        ("NODE_PATH", at("empty")),
        ("NODE_ENV", "production".to_owned()),
        ("NODE_REPL_HISTORY", named_files[1].clone()),
        ("V8_FLAGS", "--allow-natives-syntax --no-jitless".to_owned()),
        // gro's: build time only, or read only by a feature this build lacks
        (
            "GRO_PRICES_OFF",
            "landlock,seccomp,deny,locale,utc_check".to_owned(),
        ),
        ("GRO_PHASES_FILE", named_files[0].clone()),
        ("GRO_SNAPSHOT_DIR", at("empty")),
        ("GRO_COMMIT", "hostile".to_owned()),
        ("PUBLIC_LOG_LEVEL", "debug".to_owned()),
        // Rust's and tokio's
        ("RUST_LOG", "trace".to_owned()),
        ("RUST_BACKTRACE", "full".to_owned()),
        ("RUST_MIN_STACK", "1099511627776".to_owned()),
        ("RUST_LIB_BACKTRACE", "1".to_owned()),
        ("TOKIO_WORKER_THREADS", "0".to_owned()),
        // glibc's allocator and the dynamic loader: a library to preload
        // that doesn't exist (the loader warns on stderr and goes on), an
        // empty search path, eager binding
        (
            "GLIBC_TUNABLES",
            "glibc.malloc.perturb=165:glibc.malloc.mmap_threshold=4096".to_owned(),
        ),
        ("MALLOC_PERTURB_", "165".to_owned()),
        ("MALLOC_ARENA_MAX", "1".to_owned()),
        ("LD_PRELOAD", "/nonexistent/libgro_hostile.so".to_owned()),
        ("LD_LIBRARY_PATH", at("empty")),
        ("LD_BIND_NOW", "1".to_owned()),
        // ICU's data and zone files, and glibc's zone directory
        ("ICU_DATA", at("icu")),
        ("ICU_TIMEZONE_FILES_DIR", at("icu")),
        ("TZDIR", at("icu")),
        // the locale, Turkish's dotted capital I among it
        ("LANG", "de_DE.UTF-8".to_owned()),
        ("LC_ALL", "tr_TR.UTF-8".to_owned()),
        ("LC_NUMERIC", "fr_FR.UTF-8".to_owned()),
        ("LC_TIME", "ja_JP.UTF-8".to_owned()),
        ("LC_COLLATE", "sv_SE.UTF-8".to_owned()),
        ("LANGUAGE", "fr:de".to_owned()),
        // color
        ("FORCE_COLOR", "3".to_owned()),
        ("CLICOLOR_FORCE", "1".to_owned()),
        ("COLORTERM", "truecolor".to_owned()),
        ("TERM", "xterm-256color".to_owned()),
        // the home, temporary, and XDG directories, and the search path
        ("HOME", at("home")),
        ("TMPDIR", at("tmp")),
        ("XDG_CACHE_HOME", at("cache")),
        ("XDG_CONFIG_HOME", at("config")),
        ("XDG_RUNTIME_DIR", at("runtime")),
        ("PATH", at("empty")),
    ];
    let config = scratch.path("project/config.js");
    let run_in = |env: &[(&str, String)]| {
        let mut command = command("eval", &[&config, "--graph"], &[]);
        command.env_clear().env("TZ", "UTC");
        for (name, value) in env {
            command.env(name, value);
        }
        parse(spawn(command, None))
    };
    let minimal = run_in(&[]);
    assert_eq!(minimal.code, 0, "{}\n{}", minimal.stdout, minimal.stderr);
    assert_eq!(
        minimal.envelope["value"]["number"], "1,234,567.891",
        "{}",
        minimal.stdout
    );
    assert_eq!(
        minimal.envelope["value"]["upper"], "I",
        "{}",
        minimal.stdout
    );
    // stderr too, but for the dynamic loader's warning for the missing
    // preload, written at exec, before eval can clear anything
    let stderr_noise = ["LD_PRELOAD"];
    let quiet: Vec<(&str, String)> = hostile
        .iter()
        .filter(|(name, _)| !stderr_noise.contains(name))
        .cloned()
        .collect();
    let quiet = run_in(&quiet);
    assert_eq!(
        (quiet.code, quiet.stdout.as_str(), quiet.stderr.as_str()),
        (
            minimal.code,
            minimal.stdout.as_str(),
            minimal.stderr.as_str()
        )
    );
    let hostile = run_in(&hostile);
    assert_eq!(
        (hostile.code, hostile.stdout.as_str()),
        (minimal.code, minimal.stdout.as_str()),
        "{}",
        hostile.stderr
    );
    for file in named_files {
        assert!(!Path::new(&file).exists(), "{file}");
    }
    for dir in ["home", "tmp", "cache", "config", "runtime", "icu", "empty"] {
        assert_eq!(
            std::fs::read_dir(elsewhere.join(dir)).unwrap().count(),
            0,
            "{dir}"
        );
    }
}

/// `deno_core` appends each module import it records to the file
/// `DENO_SNAPSHOT_IMPORT_GRAPH` names, opening it at the first import, and
/// panics when it can't: inside the box it never can, so an eval whose
/// module imports another must ignore the variable, cleared with the rest
/// of its environment, writing the envelope a run without it writes and no
/// file.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn deno_cores_import_graph_variable_is_ignored() {
    let scratch = host_project("import_graph");
    let graph = scratch.0.join("import_graph.jsonl");
    let config = scratch.path("project/config.js");
    let run_in = |env: &[(&str, &Path)]| {
        let mut command = command("eval", &[&config], &[]);
        command.env_clear().env("TZ", "UTC");
        for (name, value) in env {
            command.env(name, value);
        }
        spawn(command, None)
    };
    let minimal = run_in(&[]);
    assert_eq!(minimal.code, 0, "{}\n{}", minimal.stdout, minimal.stderr);
    let set = run_in(&[("DENO_SNAPSHOT_IMPORT_GRAPH", &graph)]);
    assert_eq!(
        (set.code, set.stdout.as_str()),
        (minimal.code, minimal.stdout.as_str()),
        "{}",
        set.stderr
    );
    assert!(!graph.exists());
}
