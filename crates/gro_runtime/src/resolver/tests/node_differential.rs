//! The resolver against Node's: the tree `tests/fixtures/resolution/cases.json`
//! describes, built under the system's temporary directory as the oracle
//! builds it, each case resolved from its importer and held to what Node
//! resolved, `expected.json`, which `oracle.js` there writes under Node and
//! Gro's own tests check against Node, so neither side needs the other's
//! runtime.
//!
//! A case Node resolves resolves to the same URL, relative to the tree,
//! unless its `departure` says why `fgro` differs by design: it refuses the
//! case, or resolves it to the other URL the case's `fgro_url` names. A case
//! Node refuses is refused too. Either refusal is the kind the case's `fgro`
//! names, and a case `fgro` resolves names none.

use std::path::Path;

use serde_json::Value;
use url::Url;

use super::from_entry;
use crate::resolver::DiskTree;
use crate::test_support::TestDir;

/// `name`'s JSON, from the fixture directory.
fn read_fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/resolution")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_resolver_resolves_what_node_does() {
    let cases = read_fixture("cases.json");
    let expected = read_fixture("expected.json");
    let dir = TestDir::in_system_temp("resolution_differential");
    let root = dir.path();
    for (path, content) in cases["files"].as_object().unwrap() {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content.as_str().unwrap()).unwrap();
    }
    for (path, target) in cases["links"].as_object().unwrap() {
        std::os::unix::fs::symlink(target.as_str().unwrap(), root.join(path)).unwrap();
    }
    let root_url = Url::from_directory_path(root).unwrap();
    let cases = cases["cases"].as_array().unwrap();
    let expected = expected.as_array().unwrap();
    assert_eq!(
        cases.len(),
        expected.len(),
        "expected.json is stale: regenerate it with `node oracle.js --write`"
    );
    let mut failures = Vec::new();
    for (case, expected) in cases.iter().zip(expected) {
        let importer = case["importer"].as_str().unwrap();
        let specifier = case["specifier"].as_str().unwrap();
        assert_eq!(
            (
                expected["importer"].as_str(),
                expected["specifier"].as_str()
            ),
            (Some(importer), Some(specifier)),
            "expected.json is stale: regenerate it with `node oracle.js --write`"
        );
        let node = expected["node"]["url"].as_str();
        let departure = case["departure"].as_str();
        let refused_as = case["fgro"].as_str();
        let fgro_url = case["fgro_url"].as_str();
        let result = from_entry(
            &DiskTree,
            specifier,
            &Url::from_file_path(root.join(importer)).unwrap(),
        );
        let label = format!("'{specifier}' from {importer}");
        match result {
            Ok(resolution) => {
                let url = resolution.url.as_str();
                let url = url.strip_prefix(root_url.as_str()).unwrap_or(url);
                // a departure that resolves otherwise names its URL, which Node's would make stale
                let agrees = fgro_url.map_or_else(
                    || node == Some(url) && departure.is_none(),
                    |fgro_url| fgro_url == url && departure.is_some() && node != Some(url),
                );
                if !agrees || refused_as.is_some() {
                    let expects = match (fgro_url, refused_as) {
                        (Some(fgro_url), _) => fgro_url.to_owned(),
                        (None, Some(kind)) => format!("a refusal as {kind}"),
                        (None, None) if departure.is_some() => "a refusal".to_owned(),
                        (None, None) => "Node's URL".to_owned(),
                    };
                    failures.push(format!(
                        "{label}: fgro resolved {url}, where the case expects {expects}, and Node {}",
                        expected["node"]
                    ));
                }
            }
            Err(error) => {
                // the variant's name, its Debug form up to its fields
                let debug = format!("{:?}", error.kind);
                let kind = debug
                    .split(|char: char| !char.is_alphanumeric())
                    .next()
                    .unwrap_or_default();
                if let Some(fgro_url) = fgro_url {
                    failures.push(format!(
                        "{label}: fgro refused it ({error}), not {fgro_url}"
                    ));
                } else if node.is_some() && departure.is_none() {
                    failures.push(format!(
                        "{label}: fgro refused it ({error}), Node resolved {}, and no departure \
                         is declared",
                        expected["node"]
                    ));
                } else if node.is_none() && departure.is_some() {
                    failures.push(format!(
                        "{label}: a departure is declared, but Node refused it too ({})",
                        expected["node"]
                    ));
                } else if refused_as != Some(kind) {
                    failures.push(format!(
                        "{label}: fgro refused it as {kind}, not {refused_as:?}: {error}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
