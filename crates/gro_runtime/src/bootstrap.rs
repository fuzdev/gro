//! gro's bootstrap extension: installs the pure web globals from `deno_web` and
//! hides the host's internals from user code.
//!
//! Its JS (`js/bootstrap.js`) is embedded in the binary at compile time. The
//! embedding (`ascii_str_include!`) asserts in a constant that the source is
//! 7-bit ASCII, so a non-ASCII character fails the build rather than reaching
//! V8, which reads extension sources as one-byte strings.
//!
//! `deno_webidl` and `deno_web`'s own sources aren't embedded: `deno_core` reads them
//! from their crates' paths on the build machine when they first load, so this
//! runtime runs only from its build tree until a startup snapshot carries them.

use std::borrow::Cow;
use std::io::IsTerminal;
use std::sync::OnceLock;

use deno_core::{Extension, ExtensionFileSource, ascii_str_include, op2};

/// The bootstrap module's specifier.
const BOOTSTRAP_SPECIFIER: &str = "ext:gro/bootstrap.js";

/// The bootstrap module, embedded.
const BOOTSTRAP_ESM: &[ExtensionFileSource] = &[ExtensionFileSource::new(
    BOOTSTRAP_SPECIFIER,
    ascii_str_include!("../js/bootstrap.js"),
)];

/// Whether console output to the process's stdout, or with `stderr` its
/// stderr, is colored. The one place every color input is read: the stream
/// must be a terminal, and neither `NO_COLOR` nor `CLAUDECODE` may be set to a
/// non-empty value (an agent's transcript shouldn't carry ANSI codes). The
/// host reads the environment, once; user code never sees it.
fn use_color(stderr: bool) -> bool {
    static ENV_ALLOWS_COLOR: OnceLock<bool> = OnceLock::new();
    let env_allows_color = *ENV_ALLOWS_COLOR.get_or_init(|| {
        let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        !set("NO_COLOR") && !set("CLAUDECODE")
    });
    env_allows_color
        && if stderr {
            std::io::stderr().is_terminal()
        } else {
            std::io::stdout().is_terminal()
        }
}

/// [`use_color`] for the console, asked per write.
#[op2(fast)]
fn op_gro_color(stderr: bool) -> bool {
    use_color(stderr)
}

deno_core::extension!(
    gro,
    deps = [deno_webidl, deno_web],
    ops = [op_gro_color],
    customizer = |extension: &mut Extension| {
        // the macro's own `esm` list reads files from the build tree at runtime
        extension.esm_files = Cow::Borrowed(BOOTSTRAP_ESM);
        extension.esm_entry_point = Some(BOOTSTRAP_SPECIFIER);
    },
);

/// The runtime's extensions, in dependency order: `deno_webidl`, `deno_web`, and
/// gro's bootstrap last, since it reads both.
pub fn extensions() -> Vec<Extension> {
    vec![
        deno_webidl::deno_webidl::init(),
        deno_web::deno_web::init(
            std::sync::Arc::new(deno_web::BlobStore::default()),
            None,
            false,
            deno_web::InMemoryBroadcastChannel::default(),
        ),
        gro::init(),
    ]
}
