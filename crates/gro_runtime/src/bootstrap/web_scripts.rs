//! The web extensions' scripts the runtime can load, read as text, and the
//! ops each reads: what holds [`WEB_OPS`] and [`WEB_CLASSES`] to the scripts
//! in both directions.
//!
//! A script of `deno_web` reads its ops once, when it loads, by destructuring
//! `core.ops`, so a name it reads that the runtime doesn't register is
//! `undefined` there and fails only where the script calls it. The walk
//! ([`loadable`]) starts from the scripts gro's own sources load (the
//! bootstrap, and any a builtin's implementation or backing script names) and
//! follows every script a walked one names, at its top level (the snapshot
//! then holds it) or inside a function (a residual script, loaded when the
//! function first runs).

use std::collections::BTreeMap;

use deno_core::ExtensionFileSourceCode;

use super::{WEB_CLASSES, WEB_OPS, declared_extensions};

/// The bootstrap's source, the one of gro's that loads web scripts today.
const BOOTSTRAP: &str = include_str!("../../js/bootstrap.js");

/// Every source of gro's extension, where the walk starts: the bootstrap,
/// the builtins' implementations, and their backing scripts.
fn gro_sources() -> Vec<String> {
    declared_extensions()
        .iter()
        .filter(|extension| extension.name == "gro")
        .flat_map(|extension| {
            let files = extension.esm_files.iter();
            files.chain(extension.lazy_loaded_js_files.iter())
        })
        .map(|file| file.load().unwrap().as_str().to_owned())
        .collect()
}

/// The `ext:` specifiers `source` writes as string literals.
fn ext_literals(source: &str) -> Vec<&str> {
    let mut literals = Vec::new();
    for quote in ['"', '\''] {
        let mut rest = source;
        while let Some(start) = rest.find(&format!("{quote}ext:")) {
            let literal = &rest[start + 1..];
            let end = literal.find(quote).unwrap();
            literals.push(&literal[..end]);
            rest = &literal[end..];
        }
    }
    literals
}

/// Every lazy script the web extensions declare, by specifier, with its
/// file on the build machine.
fn declared_scripts() -> BTreeMap<&'static str, &'static str> {
    declared_extensions()
        .iter()
        .filter(|extension| extension.name != "gro")
        .flat_map(|extension| extension.lazy_loaded_js_files.iter())
        .map(|file| match file.code {
            ExtensionFileSourceCode::LoadedFromFsDuringSnapshot(path) => (file.specifier, path),
            _ => panic!("{} isn't read from its crate's path", file.specifier),
        })
        .collect()
}

/// The source of each script of the web extensions the runtime can load, by
/// specifier.
pub fn loadable() -> BTreeMap<&'static str, String> {
    let declared = declared_scripts();
    let web: Vec<String> = declared_extensions()
        .iter()
        .filter(|extension| extension.name != "gro")
        .map(|extension| format!("ext:{}/", extension.name))
        .collect();
    let mut loadable = BTreeMap::new();
    let mut named: Vec<String> = gro_sources()
        .iter()
        .flat_map(|source| ext_literals(source))
        .map(ToOwned::to_owned)
        .collect();
    while let Some(specifier) = named.pop() {
        if !web.iter().any(|prefix| specifier.starts_with(prefix)) {
            continue;
        }
        let (specifier, path) = declared
            .get_key_value(specifier.as_str())
            .unwrap_or_else(|| panic!("{specifier} is named and not declared"));
        if loadable.contains_key(specifier) {
            continue;
        }
        let source = std::fs::read_to_string(path).unwrap();
        named.extend(ext_literals(&source).into_iter().map(ToOwned::to_owned));
        loadable.insert(*specifier, source);
    }
    loadable
}

/// Whether `line` is a comment's: a line comment, or inside a block comment
/// written with a leading `*` on each line, as these scripts write them.
fn is_comment(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("//") || line.starts_with("/*") || line.starts_with('*')
}

/// Whether `line` holds `word` as a whole identifier.
fn holds_word(line: &str, word: &str) -> bool {
    let is_identifier = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    line.match_indices(word).any(|(at, _)| {
        !line[..at].ends_with(is_identifier) && !line[at + word.len()..].starts_with(is_identifier)
    })
}

/// The names `source` reads from `core.ops`, each by destructuring it in a
/// statement that ends `} = core.ops;`, a line comment among the names
/// passed over.
///
/// # Panics
///
/// Panics when a line outside a comment names `ops` any other way (a member
/// read, an alias, a computed key), which this reader can't follow.
pub fn core_ops_reads<'a>(specifier: &str, source: &'a str) -> Vec<&'a str> {
    const READ: &str = "} = core.ops;";
    for line in source.lines() {
        assert!(
            is_comment(line) || !holds_word(line, "ops") || line.trim_end().ends_with(READ),
            "{specifier} names `ops` outside a destructuring of `core.ops`: {line}"
        );
    }
    let mut reads = Vec::new();
    let mut rest = source;
    while let Some(end) = rest.find(READ) {
        let start = rest[..end].rfind('{').unwrap();
        reads.extend(
            rest[start + 1..end]
                .lines()
                .map(|line| line.split("//").next().unwrap_or_default())
                .flat_map(|line| line.split(','))
                .map(|property| property.split(':').next().unwrap_or_default().trim())
                .filter(|name| !name.is_empty()),
        );
        rest = &rest[end + READ.len()..];
    }
    reads
}

/// Every name a loadable script reads from `core.ops`, with the scripts
/// that read it.
pub fn loadable_reads() -> BTreeMap<String, Vec<&'static str>> {
    let mut reads: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
    for (specifier, source) in loadable() {
        for name in core_ops_reads(specifier, &source) {
            reads.entry(name.to_owned()).or_default().push(specifier);
        }
    }
    reads
}

/// The walk starts from every source of gro's extension, the bootstrap
/// among them, and reaches more than they name: a script another loads, and
/// one loaded only inside a function.
#[test]
fn the_walk_follows_the_scripts_a_script_names() {
    let sources = gro_sources();
    assert!(sources.iter().any(|source| source == BOOTSTRAP));
    assert!(sources.len() > 1);
    let loadable = loadable();
    for specifier in [
        "ext:deno_web/01_console.js",
        "ext:deno_web/00_infra.js",
        "ext:deno_web/13_message_port.js",
        "ext:deno_webidl/00_webidl.js",
    ] {
        assert!(loadable.contains_key(specifier), "{specifier}");
    }
    assert!(!BOOTSTRAP.contains("13_message_port.js"));
    assert!(loadable.len() < declared_scripts().len());
}

/// The reader takes each destructured name, a renamed one by the name it
/// reads, and refuses any other use of `ops`.
#[test]
fn the_reader_takes_destructured_names_and_refuses_other_reads() {
    let source = "const { core } = __bootstrap;
const {
  op_a,
  // a comment among the names, with a comma
  Class: Renamed, // and one after a name
} = core.ops;
// the ops a comment names are no read
const { op_b } = core.ops;
";
    assert_eq!(core_ops_reads("x", source), ["op_a", "Class", "op_b"]);
    for other in [
        "core.ops.op_a();",
        "const ops = core.ops;",
        "const { ops } = core;",
        "core.ops[name]();",
    ] {
        let read = std::panic::catch_unwind(|| core_ops_reads("x", other).len());
        assert!(read.is_err(), "{other}");
    }
    assert!(core_ops_reads("x", "defineEnumerableProps(Event, props);").is_empty());
}

/// The kept lists are the scripts' reads, in both directions: every name a
/// loadable script reads from `core.ops` is registered, the web extensions'
/// by being on [`WEB_OPS`] or [`WEB_CLASSES`], and every name on those is
/// one a loadable script reads. So an op a script comes to read fails here
/// until it's kept, and a kept op nothing reads any more until it's dropped.
#[test]
fn the_kept_web_ops_are_the_ones_a_loadable_script_reads() {
    let reads = loadable_reads();
    let registered: Vec<&str> = include_str!("../../tests/op_names.txt").lines().collect();
    let declared: Vec<&str> = declared_extensions()
        .iter()
        .filter(|extension| extension.name != "gro")
        .flat_map(|extension| {
            let classes = extension.objects.iter().map(|class| class.name.0);
            classes.chain(extension.ops.iter().map(|op| op.name))
        })
        .collect();
    let is_kept = |name: &str| WEB_OPS.contains(&name) || WEB_CLASSES.contains(&name);
    for (name, scripts) in &reads {
        if declared.contains(&name.as_str()) {
            assert!(is_kept(name), "{scripts:?} read {name}, which isn't kept");
        } else {
            assert!(
                registered.contains(&name.as_str()),
                "{scripts:?} read {name}, which no extension registers"
            );
        }
    }
    for name in WEB_OPS.iter().chain(&WEB_CLASSES) {
        assert!(
            reads.contains_key(*name),
            "{name} is kept, and no loadable script reads it"
        );
    }
}

/// Every op and class the web extensions declare and the runtime drops,
/// pinned in `tests/dropped_op_names.txt` as the registered ones are in
/// `tests/op_names.txt`, a class by its constructor and its methods: a
/// `deno_web` upgrade that adds an op or a class adds it here, since it's on
/// neither kept list, and fails until it's reviewed. One a loadable script
/// reads then joins the kept list (the test above fails until it does), and
/// `REFUSED_OPS` too when it compiles code, imports, or reads the clock or
/// randomness; one no script reads stays dropped, which no list of refusals
/// names, since eval refuses only ops the runtime registers. To accept a
/// change deliberately, write the list the assertion prints over the file,
/// one name a line.
#[test]
fn the_dropped_web_ops_are_pinned() {
    let mut dropped: Vec<&str> = Vec::new();
    for extension in declared_extensions() {
        if extension.name == "gro" {
            continue;
        }
        for class in extension.objects.iter() {
            if !WEB_CLASSES.contains(&class.name.0) {
                // the runtime lists a constructor by its class's name
                dropped.extend(class.constructor.map(|_| class.name.0));
                let methods = class.methods.iter().chain(class.static_methods);
                dropped.extend(methods.map(|op| op.name));
            }
        }
        let ops = extension.ops.iter().map(|op| op.name);
        dropped.extend(ops.filter(|name| !WEB_OPS.contains(name)));
    }
    let mut listed = dropped.join("\n");
    listed.push('\n');
    assert_eq!(
        listed,
        include_str!("../../tests/dropped_op_names.txt"),
        "the dropped ops differ from tests/dropped_op_names.txt; update it only if the change \
         is deliberate (see this test's doc)"
    );
}
