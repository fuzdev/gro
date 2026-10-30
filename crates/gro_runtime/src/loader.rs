//! The `deno_core` module loader: a thin adapter over [`crate::resolver`] that
//! reads the resolved modules from disk, stripping TypeScript's types in
//! process.
//!
//! Static imports, `import()`, and `import.meta.resolve` all reach
//! [`resolver::resolve`]. The entry module is resolved before the runtime
//! starts ([`resolver::resolve_entry`]), so its URL arrives here already
//! final, with the roots its graph starts with.
//!
//! The loader owns the graph's [`Roots`] and the [`Root`] each URL resolved
//! inside, and passes the roots into every resolution. A bare specifier's
//! package root is admitted when it resolves. Admitting a root around a
//! module already recorded inside an outer one is an error naming both
//! ([`ResolveErrorKind::RootConflict`]), since otherwise the import order
//! would decide the module's origin; resolving a URL again inside the same
//! root changes nothing. `import.meta.resolve` runs the same resolution and
//! admits and records nothing, so importing the URL it returns resolves it
//! again, as an absolute specifier: an installed package's file is then a
//! path into `node_modules`, refused.
//!
//! The loader also records each URL's [`FileId`], the device and inode
//! numbers and birth time the resolver took when it looked the file up, and
//! reads a module only from a file it opened and found to have that identity:
//! it opens the path without blocking, compares the open file's identity with
//! the record, and reads the source from that same open file, never from the
//! path again. The first resolution's record stands: resolving the URL again
//! changes nothing, and a URL already loaded is served from `deno_core`'s
//! module map. A file, or a directory on its path, swapped between resolving
//! and loading is refused, a FIFO among them, except a file remade where the
//! filesystem records no birth time, or within the same tick of its clock as
//! the original's birth: the identity then falls back to the device and
//! inode numbers, which ext4 reuses for a file made after one is removed.
//! What it can't see is a swap inside the resolver's own lookup, between
//! canonicalizing the path and reading its identity; the lookup reads that
//! identity without following symlinks, so a last component swapped for a
//! symlink there is refused, but a swapped directory above it isn't caught.
//!
//! A TypeScript module is stripped by `tsv_ts`, which blanks every type-only
//! range rather than rewriting the code, so every surviving character keeps
//! its line and column and V8's positions are the source's without a
//! sourcemap. TypeScript with runtime semantics (an `enum`, a parameter
//! property) is refused, not transformed: the load fails naming the construct,
//! a fix, and its position, in the `at <url>:<line>:<column>` form V8's frames
//! print. Like every load failure, it reaches JS as a `TypeError`, which
//! `import()` can catch.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use deno_core::error::ModuleLoaderError;
use deno_core::{
    ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader, ModuleSource,
    ModuleSourceCode, ModuleSpecifier, ModuleType, RequestedModuleType, ResolutionKind,
};
use tsv_ts::{ErrorKind, ErrorReport, Goal, StripError, UnsupportedKind};
use url::Url;

use crate::resolver::{
    self, DiskTree, FileId, ModuleFormat, Resolution, ResolveError, ResolveErrorKind, Root, Roots,
    SourceTree,
};

/// Resolves through the strict resolver over the disk, and loads `.js` and
/// `.mjs` files as ES modules, and `.ts` and `.mts` files as ES modules
/// stripped of their types.
#[derive(Debug)]
pub struct Loader {
    /// The roots admitted so far, and the root and file of each URL resolved.
    graph: RefCell<Graph>,
}

impl Loader {
    /// A loader for the graph of `entry`, already resolved with `roots`.
    pub fn new(entry: &Resolution, roots: Roots) -> Self {
        Self {
            graph: RefCell::new(Graph::new(entry, roots)),
        }
    }

    /// Resolves `specifier`, imported by the module at `referrer`, reading
    /// `tree`. With `admit`, the result's root is admitted and its URL
    /// recorded; without, as for `import.meta.resolve`, nothing changes.
    fn resolve_in(
        &self,
        tree: &impl SourceTree,
        specifier: &str,
        referrer: &str,
        admit: bool,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        let importer = Url::parse(referrer).map_err(|error| {
            ModuleLoaderError::type_error(format!(
                "cannot resolve '{specifier}': the importer '{referrer}' is not a URL: {error}"
            ))
        })?;
        let mut graph = self.graph.borrow_mut();
        // every importer was resolved here first, the entry by `Loader::new`
        if !graph.records.contains_key(&importer) {
            return Err(ModuleLoaderError::type_error(format!(
                "cannot resolve '{specifier}': the importer '{referrer}' was never resolved"
            )));
        }
        let error = |kind| {
            ModuleLoaderError::type_error(
                ResolveError {
                    specifier: specifier.to_owned(),
                    importer: referrer.to_owned(),
                    kind,
                }
                .to_string(),
            )
        };
        let resolution = resolver::resolve(tree, specifier, &importer, &graph.roots)
            .map_err(|error| ModuleLoaderError::type_error(error.to_string()))?;
        if admit {
            graph.admit(&resolution).map_err(error)?;
        }
        Ok(resolution.url)
    }

    /// Loads the module at `specifier`, a URL resolved here, from the file
    /// its resolution found.
    fn load_resolved(
        &self,
        specifier: &ModuleSpecifier,
        requested: &RequestedModuleType,
    ) -> Result<ModuleSource, ModuleLoaderError> {
        let file_id = self
            .graph
            .borrow()
            .records
            .get(specifier)
            .map(|record| record.file_id);
        // `deno_core` loads only what `resolve` returned, which recorded it
        let Some(file_id) = file_id else {
            return Err(ModuleLoaderError::type_error(format!(
                "cannot load {specifier}: it was never resolved"
            )));
        };
        load(specifier, requested, file_id)
    }
}

/// What a graph has resolved: the roots it admitted, and the root and file
/// of each URL.
#[derive(Debug)]
struct Graph {
    /// The project's root, and every package root a bare specifier resolved
    /// into.
    roots: Roots,
    /// What each URL resolved to, the entry's included.
    records: HashMap<Url, Record>,
}

/// What a URL resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    /// The root it resolved inside.
    root: Root,
    /// Its file's identity, the one loading it must find.
    file_id: FileId,
}

impl Record {
    /// The record of `resolution`.
    fn new(resolution: &Resolution) -> Self {
        Self {
            root: Root {
                dir: resolution.root.clone(),
                origin: resolution.origin.clone(),
            },
            file_id: resolution.file_id,
        }
    }
}

impl Graph {
    /// The graph of `entry`, resolved with `roots`.
    fn new(entry: &Resolution, roots: Roots) -> Self {
        Self {
            roots,
            records: HashMap::from([(entry.url.clone(), Record::new(entry))]),
        }
    }

    /// Admits the root of `resolution` and records its URL inside it, with
    /// its file, changing nothing on an error.
    ///
    /// A new root around a module already recorded inside an outer one is
    /// a conflict, never first-wins: the module's origin would depend on
    /// which import ran first. A URL resolved again inside the same root is
    /// no change, since `deno_core` can resolve a specifier twice, and its
    /// first record stands: a load checks that file, and a URL already
    /// loaded is served from `deno_core`'s module map, never read again.
    /// Resolved again inside another root, it is a conflict.
    fn admit(&mut self, resolution: &Resolution) -> Result<(), ResolveErrorKind> {
        let record = Record::new(resolution);
        let root = &record.root;
        let conflict =
            |path: PathBuf, recorded: &Root, admitted: &Root| ResolveErrorKind::RootConflict {
                path,
                recorded: Box::new(recorded.clone()),
                admitted: Box::new(admitted.clone()),
            };
        if self.roots.get(&root.dir).is_none() {
            for (url, Record { root: recorded, .. }) in &self.records {
                let Ok(path) = url.to_file_path() else {
                    continue;
                };
                // one nested in the recorded root would now hold the module
                if root.dir != recorded.dir
                    && root.dir.starts_with(&recorded.dir)
                    && root.contains(&path)
                {
                    return Err(conflict(path, recorded, root));
                }
            }
        }
        if let Some(recorded) = self.records.get(&resolution.url) {
            let path = || resolution.url.to_file_path().unwrap_or_default();
            if recorded.root.dir != root.dir {
                return Err(conflict(path(), &recorded.root, root));
            }
            // the first record stands
            return Ok(());
        }
        self.roots.admit(root.clone());
        self.records.insert(resolution.url.clone(), record);
        Ok(())
    }
}

impl ModuleLoader for Loader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        if kind == ResolutionKind::MainModule {
            // the entry, already resolved by `resolver::resolve_entry`
            return Url::parse(specifier)
                .map_err(|error| ModuleLoaderError::type_error(error.to_string()));
        }
        self.resolve_in(&DiskTree, specifier, referrer, true)
    }

    /// Resolves as an import does, roots and all, but admits no root and
    /// records no URL: the URL it returns isn't imported, and importing it
    /// resolves it again, as an absolute specifier.
    fn import_meta_resolve(
        &self,
        specifier: &str,
        referrer: &str,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        self.resolve_in(&DiskTree, specifier, referrer, false)
    }

    fn load(
        &self,
        specifier: &ModuleSpecifier,
        _referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        ModuleLoadResponse::Sync(self.load_resolved(specifier, &options.requested_module_type))
    }
}

/// Reads the module at `specifier`, a URL the resolver produced for the file
/// `file_id` names, stripping a TypeScript module's types.
fn load(
    specifier: &ModuleSpecifier,
    requested: &RequestedModuleType,
    file_id: FileId,
) -> Result<ModuleSource, ModuleLoaderError> {
    // TODO: JSON modules (`with {type: 'json'}`) join with the module formats past JS
    if *requested != RequestedModuleType::None {
        return Err(ModuleLoaderError::type_error(format!(
            "cannot load {specifier}: import attributes are not supported yet"
        )));
    }
    let path = specifier.to_file_path().map_err(|()| {
        ModuleLoaderError::type_error(format!("cannot load {specifier}: not a local file"))
    })?;
    // the resolver read the same table, so a refusal here means the URL skipped it
    let format = resolver::module_format(&path).map_err(|kind| {
        ModuleLoaderError::type_error(format!("cannot load {specifier}: {kind}"))
    })?;
    let io_error =
        |error: io::Error| ModuleLoaderError::generic(format!("cannot load {specifier}: {error}"));
    // read from the file checked, never the path again, which can change in between
    let Some(code) = resolver::read_checked(&path, file_id, None).map_err(io_error)? else {
        return Err(ModuleLoaderError::type_error(format!(
            "cannot load {specifier}: the file at its path is not the one resolved, so it isn't \
             loaded"
        )));
    };
    let code = match format {
        ModuleFormat::JavaScript => ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
        ModuleFormat::TypeScript => {
            // invalid UTF-8 reads as U+FFFD, as V8 decodes a JS module's bytes
            let source = String::from_utf8(code)
                .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
            // the class `deno_core` drops: it rethrows every load failure to JS as a
            // `TypeError` (`js_class_and_message_to_exception`)
            let code = strip_types(source, specifier)
                .map_err(|message| ModuleLoaderError::new("SyntaxError", message))?;
            ModuleSourceCode::String(code.into())
        }
    };
    Ok(ModuleSource::new(
        ModuleType::JavaScript,
        code,
        specifier,
        None,
    ))
}

/// Strips the types from `source`, the TypeScript module at `url`, returning
/// the JS, or the message the load fails with: what was wrong, a fix for a
/// refused construct, and the position V8's frames would print,
/// `\n    at <url>:<line>:<column>`.
///
/// Source with nothing type-only comes back as the same `String`, uncopied.
///
/// The strip recurses with the source's nesting, on the caller's stack, which
/// must hold tsv's contract (`tsv_ts::STACK_SIZE`; see `run_main`). Input
/// past tsv's depth limits fails the load rather than overflowing it.
fn strip_types(source: String, url: &Url) -> Result<String, String> {
    // a module goal only: V8 evaluates every module as one, so no script retry
    let stripped = match tsv_ts::strip_with_goal(&source, Goal::Module) {
        Ok(stripped) => match stripped.code {
            Cow::Borrowed(_) => None,
            Cow::Owned(code) => Some(code),
        },
        Err(error) => return Err(strip_error_message(&error, &source, url)),
    };
    Ok(stripped.unwrap_or(source))
}

/// The message of a failed strip: the summary (a parse error's says it's
/// one, and a depth limit's that it's a limit), a fix for a refusal, and the
/// position as a V8 frame prints it (1-based line and UTF-16 column).
fn strip_error_message(error: &StripError, source: &str, url: &Url) -> String {
    const PARSE_PREFIX: &str = "cannot parse the TypeScript: ";
    const STRIP_PREFIX: &str = "cannot strip the TypeScript: ";
    let (prefix, report, fix) = match error {
        // a depth limit is a bound on the input, not a verdict on its grammar
        StripError::Parse(parse) if parse.kind() == ErrorKind::TooDeep => {
            (STRIP_PREFIX, parse.report(), None)
        }
        StripError::Parse(parse) => (PARSE_PREFIX, parse.report(), None),
        // the summary is the construct's own `cannot strip ...`
        StripError::Unsupported { kind, span } => {
            ("", Some(kind.report(*span, source)), fix(*kind))
        }
        // a failure a later tsv adds, until it gets an arm: its one-line
        // `Display` (which carries any position it has) rather than
        // `StripError::message`'s caret frame, which no other load error prints
        _ => (STRIP_PREFIX, None, None),
    };
    // no report, so no `at` line: a source past the 4 GiB cap, or a failure
    // without an arm
    let Some(ErrorReport { summary, start, .. }) = report else {
        return format!("{prefix}{error}");
    };
    let mut message = format!("{prefix}{summary}");
    if let Some(Fix { how, erasable }) = fix {
        message.push_str(". Fix: ");
        message.push_str(how);
        if erasable {
            message.push_str(" (TypeScript's erasableSyntaxOnly option reports these)");
        }
    }
    format!(
        "{message}\n    at {url}:{}:{}",
        start.line,
        start.column + 1
    )
}

/// How to write a refused construct as TypeScript whose types strip away.
#[derive(Debug, Clone, Copy)]
struct Fix {
    /// The rewrite.
    how: &'static str,
    /// Whether TypeScript's `erasableSyntaxOnly` option reports the construct,
    /// so a project can catch the rest of them before running.
    erasable: bool,
}

/// The fix for a refused construct, if gro knows one: a construct a later tsv
/// refuses has none until it gets an arm.
const fn fix(kind: UnsupportedKind) -> Option<Fix> {
    let (how, erasable) = match kind {
        UnsupportedKind::Enum => (
            "use a union of literal types, or an object with 'as const'",
            true,
        ),
        UnsupportedKind::ValueNamespace => ("use the module itself, or a plain object", true),
        UnsupportedKind::ParameterProperty => (
            "declare the field in the class body and assign it in the constructor",
            true,
        ),
        UnsupportedKind::ImportEquals => ("use an import declaration", true),
        UnsupportedKind::ExportEquals => ("use 'export default'", true),
        UnsupportedKind::TypeAssertion => ("write the assertion as 'x as T'", false),
        UnsupportedKind::AssertionGrouping => {
            ("wrap the assertion in parentheses: '(x as T)'", false)
        }
        _ => return None,
    };
    Some(Fix { how, erasable })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::resolver::test_tree::MemoryTree;

    /// A project at `/p` with `bar` installed and `lnk` linked to a checkout
    /// inside the project.
    fn tree() -> MemoryTree {
        MemoryTree {
            files: BTreeMap::from([
                ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
                ("/p/main.ts", ""),
                (
                    "/p/packages/lnk/package.json",
                    r#"{"name": "lnk", "type": "module", "exports": {"./x.ts": "./x.ts"}}"#,
                ),
                ("/p/packages/lnk/x.ts", ""),
                (
                    "/p/node_modules/bar/package.json",
                    r#"{"exports": "./i.mjs"}"#,
                ),
                ("/p/node_modules/bar/i.mjs", ""),
            ]),
            links: BTreeMap::from([("/p/node_modules/lnk", "/p/packages/lnk")]),
            ..MemoryTree::default()
        }
    }

    const MAIN: &str = "file:///p/main.ts";

    fn new_loader(tree: &MemoryTree) -> Loader {
        let (entry, roots) =
            resolver::resolve_entry(tree, Path::new("/p/main.ts"), Path::new("/")).unwrap();
        Loader::new(&entry, roots)
    }

    fn state(loader: &Loader) -> (Roots, HashMap<Url, Record>) {
        let graph = loader.graph.borrow();
        (graph.roots.clone(), graph.records.clone())
    }

    /// A file reached both by a relative specifier and through its package
    /// is refused in either order, never loaded under the first one's root.
    #[test]
    fn a_root_conflict_is_refused_in_either_order() {
        let tree = tree();
        // the package first: the relative specifier is refused by the resolver
        let loader = new_loader(&tree);
        loader.resolve_in(&tree, "lnk/x.ts", MAIN, true).unwrap();
        let error = loader
            .resolve_in(&tree, "./packages/lnk/x.ts", MAIN, true)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "cannot resolve './packages/lnk/x.ts' from file:///p/main.ts: it resolves to \
                 /p/packages/lnk/x.ts, inside package 'lnk' at /p/packages/lnk, which only a \
                 bare specifier reaches"
            ),
            "{error}"
        );
        // the relative specifier first: admitting the package is the conflict
        let loader = new_loader(&tree);
        loader
            .resolve_in(&tree, "./packages/lnk/x.ts", MAIN, true)
            .unwrap();
        let before = state(&loader);
        let error = loader
            .resolve_in(&tree, "lnk/x.ts", MAIN, true)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "cannot resolve 'lnk/x.ts' from file:///p/main.ts: /p/packages/lnk/x.ts was \
                 resolved inside the project at /p, and package 'lnk' at /p/packages/lnk was \
                 then admitted around it, so its origin would depend on the import order. Fix: \
                 import it by name ('lnk' or a subpath it exports) everywhere, not by a relative \
                 path"
            ),
            "{error}"
        );
        // nothing was admitted, so it fails the same way again
        assert_eq!(state(&loader), before);
        assert!(loader.resolve_in(&tree, "lnk/x.ts", MAIN, true).is_err());
    }

    /// `deno_core` can resolve a specifier twice: the same URL inside the
    /// same root changes nothing.
    #[test]
    fn resolving_twice_is_no_change() {
        let tree = tree();
        let loader = new_loader(&tree);
        let first = loader.resolve_in(&tree, "bar", MAIN, true).unwrap();
        let after_first = state(&loader);
        let second = loader.resolve_in(&tree, "bar", MAIN, true).unwrap();
        assert_eq!(first, second);
        assert_eq!(state(&loader), after_first);
        assert_eq!(after_first.0.iter().count(), 2);
        assert_eq!(after_first.1.len(), 2);
    }

    /// `import.meta.resolve` resolves as an import does but admits and
    /// records nothing, so importing what it returned for a package is a
    /// path into `node_modules`, refused.
    #[test]
    fn import_meta_resolve_changes_nothing() {
        let tree = tree();
        let loader = new_loader(&tree);
        let before = state(&loader);
        let url = loader.resolve_in(&tree, "bar", MAIN, false).unwrap();
        assert_eq!(url.as_str(), "file:///p/node_modules/bar/i.mjs");
        assert_eq!(state(&loader), before);
        let error = loader
            .resolve_in(&tree, url.as_str(), MAIN, true)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("inside node_modules below /p") && error.contains("import it by name"),
            "{error}"
        );
        // refused as an import is
        assert!(
            loader
                .resolve_in(&tree, "./node_modules/bar/i.mjs", MAIN, false)
                .is_err()
        );
        // and an importer must have been resolved here
        let error = loader
            .resolve_in(&tree, "./main.ts", url.as_str(), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("was never resolved"), "{error}");
    }

    /// Files on disk in a fresh directory under the system's temp directory,
    /// removed on drop, under a named `package.json`, so the project root
    /// is the directory itself. Every directory is `0o755`, so a umask of 0
    /// can't make one world-writable.
    struct DiskFixtures(PathBuf);

    impl DiskFixtures {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            use std::os::unix::fs::PermissionsExt as _;
            let dir =
                std::env::temp_dir().join(format!("gro_loader_{}_{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let package_json = [("package.json", r#"{"name": "fixture", "type": "module"}"#)];
            for (file, content) in files.iter().chain(&package_json) {
                let path = dir.join(file);
                let parent = path.parent().unwrap();
                std::fs::create_dir_all(parent).unwrap();
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
                std::fs::write(path, content).unwrap();
            }
            Self(dir.canonicalize().unwrap())
        }

        fn path(&self, file: &str) -> PathBuf {
            self.0.join(file)
        }

        fn url(&self, file: &str) -> Url {
            Url::from_file_path(self.path(file)).unwrap()
        }

        /// A loader for the graph of `entry`.
        fn loader(&self, entry: &str) -> Loader {
            let (entry, roots) =
                resolver::resolve_entry(&DiskTree, Path::new(entry), &self.0).unwrap();
            Loader::new(&entry, roots)
        }

        /// Replaces the file at `file` with a new one holding `content`,
        /// written beside it first and renamed over it, so the old file's
        /// inode is still in use when the new one is made and can't be
        /// reused for it.
        fn replace(&self, file: &str, content: &str) {
            let replacement = self.path(&format!("{file}.replacement"));
            std::fs::write(&replacement, content).unwrap();
            std::fs::rename(replacement, self.path(file)).unwrap();
        }

        /// Puts a symlink to `target` at `file`, made beside it and renamed
        /// over whatever is there.
        fn link(&self, file: &str, target: &str) {
            let link = self.path(&format!("{file}.link"));
            std::os::unix::fs::symlink(target, &link).unwrap();
            std::fs::rename(link, self.path(file)).unwrap();
        }
    }

    impl Drop for DiskFixtures {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SOURCE: &str = "export const a = 1;\n";

    fn load_disk(loader: &Loader, url: &Url) -> Result<String, String> {
        loader
            .load_resolved(url, &RequestedModuleType::None)
            .map(|source| String::from_utf8(source.code.as_bytes().to_vec()).unwrap())
            .map_err(|error| error.to_string())
    }

    fn assert_refused(result: Result<String, String>, url: &Url) {
        let error = result.unwrap_err();
        assert!(
            error.contains(&format!(
                "cannot load {url}: the file at its path is not the one resolved, so it isn't \
                 loaded"
            )),
            "{error}"
        );
    }

    /// The control: a file left alone between resolving and loading loads,
    /// the entry's and an import's alike.
    #[test]
    fn an_unchanged_file_loads() {
        let fixtures = DiskFixtures::new("unchanged", &[("main.ts", ""), ("a.js", SOURCE)]);
        let loader = fixtures.loader("main.ts");
        let main = fixtures.url("main.ts");
        let url = loader
            .resolve_in(&DiskTree, "./a.js", main.as_str(), true)
            .unwrap();
        assert_eq!(load_disk(&loader, &url).unwrap(), SOURCE);
        assert_eq!(load_disk(&loader, &main).unwrap(), "");
        // resolving it again finds the same file
        loader
            .resolve_in(&DiskTree, "./a.js", main.as_str(), true)
            .unwrap();
        // and a URL never resolved here isn't loaded
        let error = load_disk(&loader, &fixtures.url("package.json")).unwrap_err();
        assert!(error.contains("it was never resolved"), "{error}");
    }

    /// A file replaced by a rename after it was resolved isn't loaded, for an
    /// import and the entry, and resolving its URL again keeps the first
    /// record, so the load is still refused.
    #[test]
    fn a_file_renamed_over_is_refused() {
        let fixtures = DiskFixtures::new("renamed", &[("main.ts", ""), ("a.js", SOURCE)]);
        let loader = fixtures.loader("main.ts");
        let main = fixtures.url("main.ts");
        let url = loader
            .resolve_in(&DiskTree, "./a.js", main.as_str(), true)
            .unwrap();
        fixtures.replace("a.js", "export const a = 2;\n");
        assert_refused(load_disk(&loader, &url), &url);
        // resolving it again keeps the first file, which the load still checks
        loader
            .resolve_in(&DiskTree, "./a.js", main.as_str(), true)
            .unwrap();
        assert_refused(load_disk(&loader, &url), &url);
        fixtures.replace("main.ts", "");
        assert_refused(load_disk(&loader, &main), &main);
    }

    /// A directory on the path swapped for a symlink to a twin directory
    /// holding a file of the same name and content: the path now opens
    /// another file, refused.
    #[test]
    fn a_directory_swapped_for_a_link_is_refused() {
        let fixtures = DiskFixtures::new(
            "component",
            &[("main.ts", ""), ("sub/a.js", SOURCE), ("twin/a.js", SOURCE)],
        );
        let loader = fixtures.loader("main.ts");
        let url = loader
            .resolve_in(
                &DiskTree,
                "./sub/a.js",
                fixtures.url("main.ts").as_str(),
                true,
            )
            .unwrap();
        std::fs::rename(fixtures.path("sub"), fixtures.path("old_sub")).unwrap();
        fixtures.link("sub", "twin");
        assert_refused(load_disk(&loader, &url), &url);
    }

    /// The file swapped for a symlink to an identical file: the open follows
    /// it to a file with another identity, refused.
    #[test]
    fn a_file_swapped_for_a_link_is_refused() {
        let fixtures = DiskFixtures::new(
            "last_component",
            &[("main.ts", ""), ("a.js", SOURCE), ("copy.js", SOURCE)],
        );
        let loader = fixtures.loader("main.ts");
        let url = loader
            .resolve_in(&DiskTree, "./a.js", fixtures.url("main.ts").as_str(), true)
            .unwrap();
        fixtures.link("a.js", "copy.js");
        assert_refused(load_disk(&loader, &url), &url);
    }

    /// A file removed and made again after it was resolved isn't loaded,
    /// though ext4 hands the new file the old one's freed inode number.
    #[test]
    fn a_file_removed_and_remade_is_refused() {
        let fixtures = DiskFixtures::new("remade", &[("main.ts", ""), ("a.js", SOURCE)]);
        let loader = fixtures.loader("main.ts");
        let url = loader
            .resolve_in(&DiskTree, "./a.js", fixtures.url("main.ts").as_str(), true)
            .unwrap();
        // past the filesystem clock's tick, so the birth times differ
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::remove_file(fixtures.path("a.js")).unwrap();
        std::fs::write(fixtures.path("a.js"), "export const a = 2;\n").unwrap();
        assert_refused(load_disk(&loader, &url), &url);
    }

    /// A FIFO put in the file's place after it was resolved is refused
    /// without blocking on the open. The load runs on its own thread, so a
    /// blocking open fails the test by its timeout rather than hanging it.
    #[test]
    fn a_fifo_swapped_in_is_refused_without_blocking() {
        let fixtures = DiskFixtures::new("fifo", &[("main.ts", ""), ("a.js", SOURCE)]);
        let loader = fixtures.loader("main.ts");
        let url = loader
            .resolve_in(&DiskTree, "./a.js", fixtures.url("main.ts").as_str(), true)
            .unwrap();
        let fifo = fixtures.path("a.js.fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        std::fs::rename(fifo, fixtures.path("a.js")).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let load_url = url.clone();
        std::thread::spawn(move || {
            let _ = send.send(load_disk(&loader, &load_url));
        });
        let Ok(result) = receive.recv_timeout(std::time::Duration::from_secs(5)) else {
            // a writer releases the blocked open, so the thread ends
            drop(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(fixtures.path("a.js")),
            );
            panic!("opening a FIFO in the file's place blocked");
        };
        assert_refused(result, &url);
    }

    fn url() -> Url {
        Url::parse("file:///p/a.ts").unwrap()
    }

    fn strip(source: &str) -> Result<String, String> {
        strip_types(source.to_owned(), &url())
    }

    #[test]
    fn types_blank_in_place() {
        assert_eq!(
            strip("const a: string = 'x';\nexport type T = 1;\nf<T>(a as T);").unwrap(),
            "const a         = 'x';\n                  \nf   (a     );"
        );
    }

    #[test]
    fn source_with_nothing_to_strip_is_not_copied() {
        let source = "export const a = 1;\n".to_owned();
        let pointer = source.as_ptr();
        let code = strip_types(source, &url()).unwrap();
        assert_eq!(code.as_ptr(), pointer);
    }

    #[test]
    fn a_refusal_names_the_construct_its_fix_and_its_position() {
        assert_eq!(
            strip("const a = 1;\n  enum E { A }\n").unwrap_err(),
            "cannot strip an enum. Fix: use a union of literal types, or an object with 'as \
             const' (TypeScript's erasableSyntaxOnly option reports these)\n    at \
             file:///p/a.ts:2:3"
        );
        assert!(
            strip("class C { constructor(private a: number) {} }")
                .unwrap_err()
                .starts_with("cannot strip a parameter property. Fix: declare the field")
        );
    }

    #[test]
    fn a_parse_error_says_so_and_has_its_position() {
        let message = strip("const a: = 1;").unwrap_err();
        assert!(
            message.starts_with("cannot parse the TypeScript: "),
            "{message}"
        );
        assert!(
            message.ends_with("\n    at file:///p/a.ts:1:10"),
            "{message}"
        );
        assert_eq!(message.lines().count(), 2, "{message}");
    }

    #[test]
    fn a_depth_limit_says_it_is_one_and_has_its_position() {
        let nested = format!("const a = {}1{};", "(".repeat(255), ")".repeat(255));
        let chained = format!("const a = 1{};", " + 1".repeat(7000));
        // input up to the limits needs tsv's stack in a debug build, past the
        // harness's 2 MiB threads
        let (nested, message) =
            tsv_ts::run_with_stack(|| (strip(&nested), strip(&chained))).unwrap();
        assert_eq!(
            nested.unwrap_err(),
            "cannot strip the TypeScript: Nesting exceeds tsv's depth limit of 256 levels\n    \
             at file:///p/a.ts:1:266"
        );
        let message = message.unwrap_err();
        assert!(
            message.starts_with(
                "cannot strip the TypeScript: Chain of operators, member accesses, calls, \
                 assertions or type suffixes exceeds tsv's length limit\n    at file:///p/a.ts:1:"
            ),
            "{message}"
        );
        assert_eq!(message.lines().count(), 2, "{message}");
    }

    #[test]
    fn only_erasable_refusals_name_the_option() {
        let message = strip("const a = -x as number ** 2;").unwrap_err();
        assert!(
            message.starts_with("cannot strip ")
                && message.contains("Fix: wrap the assertion in parentheses: '(x as T)'\n"),
            "{message}"
        );
        assert!(!message.contains("erasableSyntaxOnly"), "{message}");
        let message = strip("namespace N { export const a = 1; }").unwrap_err();
        assert_eq!(
            message.matches("erasableSyntaxOnly").count(),
            1,
            "{message}"
        );
    }

    #[test]
    fn columns_count_utf16_units() {
        // the astral character is two UTF-16 units, as V8 counts columns
        assert!(
            strip("const s = '\u{1F600}'; enum E {}")
                .unwrap_err()
                .ends_with(":1:17"),
        );
    }

    #[test]
    fn the_goal_is_a_module() {
        // `with` is a sloppy script's; a module refuses it
        assert!(strip("with (a) {}").is_err());
    }
}
