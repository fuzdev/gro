//! Tests for the box: reads outside it are breaches.

use super::*;
use crate::confine::BoxRoots;

/// A loader over `/p/main.ts` in `tree`, its reads checked against the
/// box `roots`.
fn boxed_loader(tree: &MemoryTree, roots: &[&str]) -> Loader {
    let roots = roots.iter().map(PathBuf::from).collect();
    new_loader(tree).with_rules(Rules {
        box_roots: Some(BoxRoots::for_test(roots)),
        ..Rules::default()
    })
}

/// A `package.json` read outside the box is refused before it is read,
/// and the loader records the breach; inside it, nothing is.
#[test]
fn a_read_outside_the_box_is_a_breach() {
    let mut tree = tree();
    tree.files.insert(
        "/q/ext/package.json",
        r#"{"name": "ext", "type": "module", "exports": "./x.js"}"#,
    );
    tree.files.insert("/q/ext/x.js", "");
    tree.links.insert("/p/node_modules/ext", "/q/ext");
    let resolve = |loader: &Loader, specifier: &str| {
        let boxed = Boxed {
            check: &loader.box_check,
            tree: &tree,
        };
        loader.resolve_reached(&boxed, specifier, MAIN, STATIC)
    };

    let inside = boxed_loader(&tree, &["/p", "/q/ext"]);
    assert!(resolve(&inside, "ext").is_ok());
    assert!(resolve(&inside, "bar").is_ok());
    assert_eq!(inside.breach(), None);

    let outside = boxed_loader(&tree, &["/p"]);
    assert!(resolve(&outside, "bar").is_ok());
    assert_eq!(outside.breach(), None);
    let error = resolve(&outside, "ext").unwrap_err();
    assert!(error.to_string().contains("outside the box"), "{error}");
    assert_eq!(outside.breach(), Some(Path::new("/q/ext/package.json")));
    // the first breach stands
    let _ = resolve(&outside, "ext");
    assert_eq!(outside.breach(), Some(Path::new("/q/ext/package.json")));
}

/// A `package.json` the pre-pass read before the box existed is served
/// from that read only inside the box: symlinked out of it, the walk up
/// a bare import makes is refused as a breach, as a fresh read would be.
#[test]
fn a_read_the_prepass_kept_is_checked_against_the_box() {
    let mut tree = tree();
    let project = tree.files.remove("/p/package.json").unwrap();
    tree.files.insert("/q/pkg.json", project);
    tree.links.insert("/p/package.json", "/q/pkg.json");
    let mut reads = Reads::default();
    let (entry, roots) = resolver::resolve_entry(
        &RecordingTree::new(&tree, &mut reads),
        Path::new("/p/main.ts"),
        Path::new("/"),
    )
    .unwrap();
    assert!(reads.hash(Path::new("/q/pkg.json")).is_some());
    let loader = Loader::new(&entry, roots, Recording::Full)
        .with_reads(reads)
        .with_rules(Rules {
            box_roots: Some(BoxRoots::for_test(vec![PathBuf::from("/p")])),
            ..Rules::default()
        });
    let boxed = Boxed {
        check: &loader.box_check,
        tree: &tree,
    };
    let error = loader
        .resolve_reached(&boxed, "bar", MAIN, STATIC)
        .unwrap_err();
    assert!(error.to_string().contains("outside the box"), "{error}");
    assert_eq!(loader.breach(), Some(Path::new("/q/pkg.json")));
}

/// A module outside the box isn't read, and the loader records the
/// breach; with no box, reads aren't checked.
#[test]
fn a_module_outside_the_box_is_not_read() {
    let fixtures = DiskFixtures::new("box", &[("main.js", SOURCE)]);
    let url = fixtures.url("main.js");
    let unboxed = fixtures.loader("main.js");
    assert_eq!(load_disk(&unboxed, &url).as_deref(), Ok(SOURCE));
    assert_eq!(unboxed.breach(), None);
    let inside = fixtures.loader("main.js").with_rules(Rules {
        box_roots: Some(BoxRoots::for_test(vec![fixtures.dir().to_owned()])),
        ..Rules::default()
    });
    assert_eq!(load_disk(&inside, &url).as_deref(), Ok(SOURCE));
    assert_eq!(inside.breach(), None);
    let elsewhere = fixtures.dir().with_file_name("elsewhere");
    let outside = fixtures.loader("main.js").with_rules(Rules {
        box_roots: Some(BoxRoots::for_test(vec![elsewhere])),
        ..Rules::default()
    });
    let error = load_disk(&outside, &url).unwrap_err();
    assert!(error.contains("outside the box"), "{error}");
    assert_eq!(outside.breach(), Some(fixtures.path("main.js").as_path()));
}
