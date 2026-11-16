//! Each builtin row's backing script, made here from the row: the text
//! `deno_core` compiles, the first time a run imports the row, as the body
//! of a function of its bootstrap object, `__bootstrap`. No row has a
//! script of its own: the namespace, and every member and value in it, are
//! the host's (`host/native.rs`), so a script only asks for its namespace,
//! and the row decides one thing of it, whether it hands the realm's
//! `Error` over ([`hands_over_error`]). So there are two templates, with
//! the `Error` and without, each called with its row's scope argument
//! ([`scope_argument`]):
//!
//! ```js
//! "use strict"; return ((function (bootstrap) {
//! const { core, primordials } = bootstrap;
//! const { op_gro_namespace } = core.ops;
//! const { Error } = primordials;
//! return (_scope) => op_gro_namespace(_scope, Error);
//! })(__bootstrap)(undefined));
//! ```
//!
//! and the same without the `primordials` and the `Error`. One function in
//! two levels, both called at once. The outer level takes the one op,
//! `op_gro_namespace`, from `core.ops`, and, from a row that hands it
//! over, the realm's `Error` from `primordials`, taken before any user code
//! ran; `deno_core` gave both before any user code could. The inner level
//! takes the scope of its importer's grant and asks the host for the
//! namespace, handing over the scope, and the `Error` where the outer level
//! took it. Why the shape is safe:
//!
//! - A first import can come after user code has patched the global
//!   intrinsics, so the script names no global but `undefined`, which
//!   strict code can't have replaced, and reads no member of an intrinsic:
//!   each read is a destructuring of what `deno_core` handed it, which
//!   reads a property and calls nothing, iterates nothing, and converts
//!   nothing.
//! - It hands on nothing: the script's value is what the op returns, so no
//!   row's script hands on the bootstrap object, `core`, `core.ops`, or the
//!   op, reads another op, or lets its inner level escape, and only a row
//!   that hands it over reads `primordials` at all, a read that costs a
//!   first import.
//! - The op knows the row by the script that calls it, the one
//!   `deno_core` compiled under the row's backing specifier
//!   ([`BACKING_SPECIFIERS`](super::BACKING_SPECIFIERS)), never by what the
//!   script passes, so every row's script can be the same text.
//! - Neither level stores anything outside its own bindings, and the op is
//!   a get-or-make, so a first import cut short at any call leaves nothing
//!   half-made and runs the whole script again at the next import.
//!
//! Every piece is a literal of this module, 7-bit ASCII, as `deno_core`
//! takes a residual script's text on trust: the build fails on a const
//! piece that isn't, and a test holds every row's script to the two texts,
//! byte for byte, which holds the wrapping `format!` adds as well.

use crate::host::native::hands_over_error;
use crate::resolver::{BUILTINS, Builtin, Capability};

/// What `deno_core` wraps a lazy script in to compile it as a function's
/// body, and takes a script that starts with [`WRAP_OPEN`] to be wrapped
/// in already, compiling it as it is, an external string V8 never copies.
/// A test holds the two to `deno_core`'s own wrapping.
const WRAP_OPEN: &str = "\"use strict\"; return (";

/// The end of the wrapping [`WRAP_OPEN`] starts.
const WRAP_CLOSE: &str = ");";

/// The outer level's body in a row that hands the realm's `Error` over.
const WITH_ERROR: &str = "const { core, primordials } = bootstrap;
const { op_gro_namespace } = core.ops;
const { Error } = primordials;
return (_scope) => op_gro_namespace(_scope, Error);";

/// The outer level's body in every other row.
const WITHOUT_ERROR: &str = "const { core } = bootstrap;
const { op_gro_namespace } = core.ops;
return (_scope) => op_gro_namespace(_scope);";

// `deno_core` takes a residual script's text as ASCII, unchecked, so the
// pieces every backing script is made of are held to it as the build
// compiles: the wrapping, both bodies, and the scope argument of every
// row's grant. The test below holds each whole script to its text.
const _: () = {
    assert!(
        WRAP_OPEN.is_ascii()
            && WRAP_CLOSE.is_ascii()
            && WITH_ERROR.is_ascii()
            && WITHOUT_ERROR.is_ascii()
    );
    let mut at = 0;
    while at < BUILTINS.len() {
        assert!(scope_argument(BUILTINS[at].grant).is_ascii());
        at += 1;
    }
};

/// What a backing script passes its inner level as the scope of the
/// importer's grant, a JS expression over the script's `__bootstrap`: the
/// one place a builtin's scope comes from. Every inner level is called
/// with `undefined` today. A grant-free module has no grant to scope, and
/// neither `args` nor `exit` scopes anything. `env` does, but one origin
/// holds it, the project, every module of a confined run's that may import
/// it (an unconfined run's are given every grant whole), so the host makes
/// its root handle from the run's grant, which carries its scope natively,
/// and every method decides from the handle it's called on, never from the
/// run's grants. An instance per origin hands the origin's scope here, a
/// native value the host gives back to the namespace op, which keys the
/// origin's namespace by it and makes the origin's root from it.
const fn scope_argument(grant: Option<Capability>) -> &'static str {
    match grant {
        None | Some(Capability::Args | Capability::Exit | Capability::Env | Capability::Fs) => {
            "undefined"
        }
    }
}

/// `row`'s backing script: the template its row gives, with the realm's
/// `Error` when the row hands it over ([`hands_over_error`]), called with
/// its grant's scope ([`scope_argument`]), and wrapped as `deno_core` wraps
/// a lazy script, so it compiles the script as it is. `deno_core` keeps
/// what the script returns, the namespace, only once both levels have
/// returned.
pub fn backing_script(row: &Builtin) -> String {
    let body = if hands_over_error(row.specifier) {
        WITH_ERROR
    } else {
        WITHOUT_ERROR
    };
    let scope = scope_argument(row.grant);
    format!("{WRAP_OPEN}(function (bootstrap) {{\n{body}\n}})(__bootstrap)({scope}){WRAP_CLOSE}")
}

#[cfg(test)]
mod tests {
    use super::{WRAP_CLOSE, WRAP_OPEN, backing_script};
    use crate::resolver::BUILTINS;

    /// The backing script of a row that hands the realm's `Error` over.
    const WITH_ERROR: &str = "\"use strict\"; return ((function (bootstrap) {
const { core, primordials } = bootstrap;
const { op_gro_namespace } = core.ops;
const { Error } = primordials;
return (_scope) => op_gro_namespace(_scope, Error);
})(__bootstrap)(undefined));";

    /// The backing script of every other row.
    const WITHOUT_ERROR: &str = "\"use strict\"; return ((function (bootstrap) {
const { core } = bootstrap;
const { op_gro_namespace } = core.ops;
return (_scope) => op_gro_namespace(_scope);
})(__bootstrap)(undefined));";

    /// The rows whose members declare an error family, as
    /// `tests/native_members.txt` pins the table: each line naming one
    /// has `throws <Family>` or `class <Family>` in it.
    fn declaring_rows() -> Vec<&'static str> {
        include_str!("../../tests/native_members.txt")
            .lines()
            .filter(|line| line.contains(" throws ") || line.contains(" class "))
            .filter_map(|line| line.split(' ').next())
            .collect()
    }

    /// Every row's backing script is, byte for byte, one of the two texts
    /// the module's doc shows: with the realm's `Error` exactly for the
    /// rows whose members the pinned table shows declaring an error family,
    /// and without it for every other. So a change to what a script reads,
    /// hands on, or is called with fails here, and only a row that hands
    /// the `Error` over reads it. Each text is 7-bit ASCII.
    #[test]
    fn every_row_s_backing_script_is_its_template() {
        let declaring = declaring_rows();
        assert!(!declaring.is_empty(), "a row hands the realm's Error over");
        for row in BUILTINS {
            let expected = if declaring.contains(&row.specifier) {
                WITH_ERROR
            } else {
                WITHOUT_ERROR
            };
            assert_eq!(backing_script(row), expected, "{}", row.specifier);
        }
        assert!(
            BUILTINS
                .iter()
                .any(|row| !declaring.contains(&row.specifier)),
            "a row hands over no Error"
        );
        assert!(WITH_ERROR.is_ascii() && WITHOUT_ERROR.is_ascii());
    }

    /// The wrapping a backing script is made with is `deno_core`'s own of a
    /// lazy script, so `deno_core` compiles a backing script as it is and
    /// never wraps one again.
    #[test]
    fn a_backing_script_is_wrapped_as_deno_core_wraps_a_lazy_script() {
        let function = "(function () {})()";
        assert_eq!(
            deno_core::wrap_lazy_ext_script(function),
            format!("{WRAP_OPEN}{function}{WRAP_CLOSE}")
        );
    }
}
