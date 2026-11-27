//! Tests for the run's box's steps that hold without installing it: what a
//! failed namespace makes of the run, the network form each network
//! builds its walls for, and the rules a box keeps once installed. The box
//! itself is tested from inside a process holding it, harness off
//! (`gro_cli`'s `tests/run_box.rs`).

use std::os::fd::{AsFd as _, AsRawFd as _};
use std::path::Path;

use gro_sys::namespace::{MapStep, NamespaceError, Refusal};

use super::Scratch;
use crate::confine::plan::{RuleSource, ToolDir};
use crate::confine::rules::{BoxRules, Kind, Need, Rights};
use crate::confine::run::{RunNetwork, RunStepError, network_refused};
use crate::confine::{ConfineError, NetForm, Threads};

/// A namespace failed for another thread fails the run as the caller's
/// bug, and one whose maps failed fails it stranded, naming the write;
/// neither falls back to the host's network, which only a refusal does.
#[test]
fn a_failed_namespace_falls_back_only_when_refused() {
    match network_refused(NamespaceError::Threads { count: 2 }) {
        Err(RunStepError::Threads(Threads {
            count: 2,
            first_step: false,
        })) => {}
        other => panic!("not refused for its threads: {other:?}"),
    }
    let stranded = NamespaceError::Stranded {
        step: MapStep::UidMap,
        source: std::io::Error::from_raw_os_error(libc::EPERM),
    };
    match network_refused(stranded) {
        Err(RunStepError::Stranded {
            step: MapStep::UidMap,
            source,
        }) => assert_eq!(source.raw_os_error(), Some(libc::EPERM)),
        other => panic!("not stranded: {other:?}"),
    }
}

/// The run's own namespace builds the walls of [`NetForm::Namespace`], and
/// the host's network, whatever refused the namespace or where a run that
/// starts no program takes none, those of [`NetForm::InetRefused`].
#[test]
fn each_network_builds_its_form_s_walls() {
    assert_eq!(RunNetwork::Namespace.form(), NetForm::Namespace);
    assert_eq!(RunNetwork::Host.form(), NetForm::InetRefused);
    for (cause, errno) in [(Refusal::AppArmor, None), (Refusal::Filtered, Some(1))] {
        assert_eq!(
            RunNetwork::InetRefused { cause, errno }.form(),
            NetForm::InetRefused
        );
    }
}

/// The rules a box keeps once installed are its rules' paths, kinds,
/// rights, and sources, in order, each beside the handle it opened, and the
/// absent ones' paths.
#[test]
fn an_installed_box_keeps_its_rules_as_data() {
    let scratch = Scratch::new("installed");
    scratch.dir("out");
    scratch.file("package.json");
    let root = std::fs::File::open(scratch.path()).unwrap();
    let mut rules = BoxRules::new();
    let tool_dir = RuleSource::ToolDir(ToolDir::SvelteKit);
    for (path, rights, need, source) in [
        (".", Rights::READ, Need::Required, RuleSource::Project),
        (
            "out",
            Rights::READ | Rights::WRITE,
            Need::Required,
            tool_dir,
        ),
        (
            "package.json",
            Rights::READ,
            Need::Required,
            RuleSource::Entry,
        ),
        ("missing", Rights::READ, Need::Optional, RuleSource::Project),
    ] {
        rules
            .beneath(
                root.as_fd(),
                scratch.path(),
                Path::new(path),
                rights,
                need,
                source,
            )
            .unwrap();
    }
    let (installed, absent) = rules.into_installed();
    let kept: Vec<_> = installed
        .iter()
        .map(|(rule, handle)| {
            // each handed back with the handle it opened
            assert_eq!(
                std::fs::read_link(format!("/proc/self/fd/{}", handle.as_raw_fd()))
                    .unwrap()
                    .components()
                    .collect::<std::path::PathBuf>(),
                rule.path().components().collect::<std::path::PathBuf>()
            );
            (
                rule.path().to_owned(),
                rule.kind(),
                rule.rights(),
                rule.source(),
            )
        })
        .collect();
    assert_eq!(
        kept,
        [
            (
                scratch.path().join("."),
                Kind::Directory,
                Rights::READ,
                RuleSource::Project
            ),
            (
                scratch.path().join("out"),
                Kind::Directory,
                Rights::READ | Rights::WRITE,
                tool_dir
            ),
            (
                scratch.path().join("package.json"),
                Kind::File,
                Rights::READ,
                RuleSource::Entry
            ),
        ]
    );
    assert_eq!(absent, [scratch.path().join("missing")]);
}

/// A refusal of what the box would miss names each descriptor, in the
/// singular or the plural, and a standard stream by its name.
#[test]
fn a_held_descriptor_s_refusal_names_it() {
    for (error, says) in [
        (
            ConfineError::SocketHeld { fds: vec![5] },
            "holds a socket (descriptor 5),",
        ),
        (
            ConfineError::SocketHeld { fds: vec![5, 7] },
            "holds sockets (descriptors 5 and 7),",
        ),
        (
            ConfineError::HidingHeld { fds: vec![3, 4, 9] },
            "holds io_urings (descriptors 3, 4, and 9),",
        ),
        (
            ConfineError::StdioSocket { fds: vec![0] },
            "stdin is a socket that isn't a connected unix stream,",
        ),
        (
            ConfineError::StdioSocket { fds: vec![1, 2] },
            "stdout and stderr are sockets that aren't connected unix streams,",
        ),
        (
            ConfineError::StdioSocket { fds: vec![0] },
            "; give it a file, a pipe, or /dev/null",
        ),
    ] {
        let message = error.to_string();
        assert!(message.contains(says), "{message}");
    }
}
