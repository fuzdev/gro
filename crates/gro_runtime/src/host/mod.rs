//! The host's half of the builtins: the native members every builtin's
//! namespace holds (`native.rs`), the functions and values that reach past
//! pure computation on a host module's behalf, and the one host op a row's
//! script asks for its namespace by.
//!
//! Every native member is declared through `native_members!`, whose first
//! step is a check in `grant.rs`: [`require`](grant::require) of the grant
//! it names, the one place a [`Granted`](grant::Granted) token is made,
//! refusing a run that doesn't hold it, or, for a grant-free module's
//! member, [`require_authority`], which refuses only a runtime that holds
//! no authority at all, eval's, so every run's importers reach it and an
//! eval reaches none.
//!
//! The host op, `op_gro_namespace`, is declared through `host_ops!`, whose
//! first step is [`require_authority`], and which lists it in `HOST_OPS`:
//! it serves every row, the row its calling script's, and checks the row's
//! grant before it makes anything. The runtime's tests hold the rest: every
//! op gro's extension registers, whatever its name, is either a host op or
//! on the bootstrap's ungated list, and each host op refuses a runtime with
//! no authority.
//!
//! Nothing the host serves is a `deno_core` async op: not the host op, not
//! an `async` member, whose promise the host makes and settles before
//! returning it (`native::settle`), so nothing of the call is left to the
//! event loop, and not a `later` member, whose promise the host settles in
//! a later turn through `deno_core`'s task queue, counted while it's
//! pending so the event loop waits for it (`later.rs`).
//!
//! The loader refuses an ungranted builtin at link time already, and every
//! builtin in an eval, so a refusal here is the second check, for a path to
//! a member past the loader.

pub mod blake3;
pub mod env;
pub mod error_class;
pub mod exit;
pub mod fs;
mod grant;
pub mod handle;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no shipped member is declared `later` yet; the form's tests declare theirs"
    )
)]
pub mod later;
pub mod native;
pub mod spawn;

use deno_core::error::ModuleLoaderError as JsErrorBox;
use deno_core::{OpDecl, OpState, op2, v8};

use error_class::{ENV_ERROR, FS_ERROR, SPAWN_ERROR};
use exit::{ExitState, NOT_A_CODE, to_exit_code};
use grant::require_authority;
use handle::{
    BLAKE3_HASHER, ENV_HANDLE, FS_DIR, FS_LISTING, FS_READ_DIR, FS_ROOT, FS_TEMP_DIR, SPAWN,
    SPAWN_CHILD, SPAWN_PROGRAM, SPAWN_STDIN,
};
use native::native_members;
use native::{ASYNC_DISPOSE, ASYNC_ITERATOR};

use crate::loader::RefusalLog;
use crate::{ArgsGrant, EnvGrant, EnvVars, ExitGrant, FsGrant, SpawnGrant};

/// What `exit`'s functions throw in a runtime started without the exit
/// host module's state, which only a runtime made outside a run's start is: a
/// run that grants `exit` holds it.
const NO_EXIT_STATE: &str = "exit is granted, but this runtime was started without its state";

/// What `cwd` throws when the working directory's path isn't UTF-8.
const CWD_NOT_UTF8: &str = "the working directory's path isn't UTF-8, so Gro can't name it";

/// What the host throws where it needs the run's error classes in a runtime
/// started without them, which only a runtime made outside a run's start
/// is.
const NO_ERROR_CLASSES: &str = "this runtime was started without the run's error classes";

/// What the native members make of the run, put in the runtime's
/// `OpState` with the run's [`Authority`](crate::Authority) when `fgro
/// run`'s runtime starts: data the embedder gives, none of it read from the
/// process here.
#[derive(Debug, Clone, Default)]
pub struct HostData {
    /// The script's arguments.
    pub args: Vec<String>,
    /// The working directory the run was started in, unset when its path
    /// isn't UTF-8, which a JS string can't hold.
    pub cwd: Option<String>,
    /// The runtime's name and version, as the `runtime_version` host module
    /// exports it.
    pub runtime_version: String,
    /// The environment variables the embedder read for the env host
    /// module's root handle, of which it holds the names the run grants.
    pub env: EnvVars,
    /// The project's root, the entry's, a real path: the fs host module's
    /// first root, opened when the run first makes the fs namespace.
    pub project: Option<std::path::PathBuf>,
    /// The OS's temporary directory, as the embedder names it for a run
    /// whose `fs` grant makes temporary directories
    /// ([`RuntimeOptions::temp_dir`](crate::RuntimeOptions::temp_dir)).
    pub temp_dir: Option<std::path::PathBuf>,
    /// The run's spawn state, made as it starts, for a run that grants
    /// `spawn` ([`spawn::SpawnRun`]).
    pub spawn: Option<std::rc::Rc<spawn::SpawnRun>>,
}

/// Declares host ops, each an `#[op2]` whose first step is
/// [`require_authority`], refusing before its body runs in a runtime that
/// holds no authority (eval's), and lists them in `HOST_OPS` (for the
/// tests) and [`host_ops`]. The one host op serves every row and checks the
/// row's grant itself, once it knows the row (`native::namespace`).
///
/// An op is written `fn name(state; scope, args...) { body }`. Each
/// argument is the JS value as it was passed, a `v8::Local<'s, v8::Value>`,
/// never converted on the way in: `#[op2]`'s typed arguments convert, and
/// a conversion can run the caller's code or truncate, so the body reads
/// the value natively and throws through the scope, an error V8 makes from
/// the realm's intrinsic ([`throw_type_error`]) rather than returning an
/// `Err`, which `deno_core` would turn into an error through a JS callback,
/// running code the program can patch. The check still comes
/// first, before the body sees an argument. The body is the op's result, a
/// value made in the scope, `'s` the scope's lifetime, which the macro
/// declares; an op that throws returns `undefined` beside the exception,
/// which V8 throws in its place. An op that takes the scope has no
/// signature a fast call could serve, so it's slow.
macro_rules! host_ops {
    ($(
        $(#[doc = $doc:literal])*
        fn $name:ident($state:ident; $scope:ident $(, $arg:ident)*) $body:block
    )*) => {
        $(
            $(#[doc = $doc])*
            #[op2]
            fn $name<'s>(
                $state: &OpState,
                $scope: &v8::PinScope<'s, '_>
                $(, $arg: v8::Local<'s, v8::Value>)*
            ) -> Result<v8::Local<'s, v8::Value>, JsErrorBox> {
                require_authority($state).map_err(|refused| refused.into_js(stringify!($name)))?;
                Ok($body)
            }
        )*

        /// Every host op's name, in declaration order.
        #[cfg(test)]
        pub const HOST_OPS: &[&str] = &[$(stringify!($name)),*];

        /// Every host op, for the bootstrap's extension.
        pub fn host_ops() -> Vec<OpDecl> {
            vec![$($name()),*]
        }
    };
}

/// Throws `message` as a `TypeError` through `scope`, for a value an op
/// refuses: the realm's own, made by V8 from its intrinsic, whatever
/// `globalThis.TypeError` has become; a `RangeError` when `message` is
/// longer than V8's longest string, as [`string`] throws, so a throw is
/// never lost.
pub fn throw_type_error(scope: &v8::PinScope<'_, '_>, message: &str) {
    if let Some(message) = string(scope, message) {
        let error = v8::Exception::type_error(scope, message);
        scope.throw_exception(error);
    }
}

/// Throws `message` as a `RangeError` through `scope`, the realm's own, as
/// [`throw_type_error`] throws a `TypeError`.
pub fn throw_range_error(scope: &v8::PinScope<'_, '_>, message: &str) {
    if let Some(message) = v8::String::new(scope, message) {
        let error = v8::Exception::range_error(scope, message);
        scope.throw_exception(error);
    }
}

/// Throws `message` as an `Error` through `scope`, the realm's own, as
/// [`throw_type_error`] throws a `TypeError`, a `RangeError` as it does.
pub fn throw_error(scope: &v8::PinScope<'_, '_>, message: &str) {
    if let Some(message) = string(scope, message) {
        let error = v8::Exception::error(scope, message);
        scope.throw_exception(error);
    }
}

/// `text` as a JS string, or `None` with a `RangeError` thrown, as the
/// language throws one, when it's longer than V8's longest string, for
/// which V8 throws nothing.
fn string<'s>(scope: &v8::PinScope<'s, '_>, text: &str) -> Option<v8::Local<'s, v8::String>> {
    let string = v8::String::new(scope, text);
    if string.is_none() {
        throw_range_error(scope, "Invalid string length");
    }
    string
}

/// `text` as a property name.
fn name<'s>(scope: &v8::PinScope<'s, '_>, text: &str) -> Option<v8::Local<'s, v8::Name>> {
    v8::String::new(scope, text).map(Into::into)
}

/// A read that threw, its exception pending.
struct Thrown;

/// The value of `object`'s own data property `key`, `None` when it has
/// none, or an accessor, whose getter isn't run, or is a proxy.
fn own_data<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
) -> Result<Option<v8::Local<'s, v8::Value>>, Thrown> {
    if object.is_proxy() {
        return Ok(None);
    }
    let descriptor = object
        .get_own_property_descriptor(scope, key)
        .ok_or(Thrown)?;
    // a descriptor V8 made is an ordinary object with its fields as its own
    // data properties, so reading the one present reads nothing else
    let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) else {
        return Ok(None);
    };
    let value_key = name(scope, "value").ok_or(Thrown)?;
    if !descriptor
        .has_own_property(scope, value_key)
        .ok_or(Thrown)?
    {
        return Ok(None);
    }
    descriptor
        .get(scope, value_key.into())
        .map(Some)
        .ok_or(Thrown)
}

host_ops! {
    /// The namespace of the row whose backing script calls it: the host's,
    /// its members native functions and values made the first time the run
    /// asks and held, so every later ask, a rerun of a first import ended
    /// early among them, gets the same ones, once the row's grant is
    /// checked; the script hands over its importer's scope, and the realm's
    /// `Error`, from `primordials`, when it's a row of an error family's
    /// ([`native::namespace`]).
    fn op_gro_namespace(state; scope, importer_scope, error) {
        native::namespace(state, scope, importer_scope, error)
            .map_or_else(|| v8::undefined(scope).into(), Into::into)
    }
}

native_members! {
    /// `args`, the script's arguments: a frozen array of strings, one for
    /// every importer.
    value args(state, _granted: ArgsGrant; scope) for "@fuzdev/gro/args.ts" {
        let mut elements = Vec::new();
        for arg in &state.borrow::<HostData>().args {
            elements.push(string(scope, arg)?.into());
        }
        let args = v8::Array::new_with_elements(scope, &elements);
        args.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
        Some(args.into())
    }

    /// `cwd()`, the working directory the run was started in, or, when its
    /// path isn't UTF-8, which a JS string can't hold, an `Error` thrown
    /// saying so.
    fn cwd(state; scope) for "@fuzdev/gro/cwd.ts" {
        let cwd = state.borrow::<HostData>().cwd.as_deref();
        let Some(cwd) = cwd else {
            throw_error(scope, CWD_NOT_UTF8);
            return None;
        };
        string(scope, cwd).map(Into::into)
    }

    /// `exit(code?)`: ends the run now, by the embedder's step, with
    /// `code`, or, given `undefined`, with the code `set_exit_code` set,
    /// else 0. Any other value throws a `TypeError` and ends nothing.
    fn exit(state, _granted: ExitGrant; scope, code) for "@fuzdev/gro/exit.ts" {
        let code = if code.is_undefined() {
            Some(None)
        } else {
            to_exit_code(code).map(Some)
        };
        match (code, state.try_borrow::<ExitState>(), state.try_borrow::<RefusalLog>()) {
            (None, ..) => throw_type_error(scope, NOT_A_CODE),
            (Some(code), Some(exit), Some(refusals)) => exit.exit(code, refusals),
            (Some(_), ..) => throw_error(scope, NO_EXIT_STATE),
        }
        None
    }

    /// `set_exit_code(code)`: sets the code the run exits with when it
    /// ends on its own, or throws a `TypeError` for a value that is no
    /// code.
    fn set_exit_code(state, _granted: ExitGrant; scope, code) for "@fuzdev/gro/exit.ts" {
        match (to_exit_code(code), state.try_borrow::<ExitState>()) {
            (None, _) => throw_type_error(scope, NOT_A_CODE),
            (Some(code), Some(exit)) => exit.set_code(code),
            (Some(_), None) => throw_error(scope, NO_EXIT_STATE),
        }
        None
    }

    /// `EnvError`, the env host module's error class, the host's
    /// ([`error_class::exported_class`]), extending the realm's `Error` the
    /// namespace op held.
    value EnvError(state; scope) for "@fuzdev/gro/env_error.ts" class ENV_ERROR(family) {
        error_class::exported_class(state, scope, family)
    }

    /// `is_env_error(error, kind?)`, whether `error` is an `EnvError`, of
    /// `kind` when it's given ([`error_class::guard`]).
    fn is_env_error(state; scope, error, kind) for "@fuzdev/gro/env_error.ts" {
        error_class::guard(scope, &ENV_ERROR, error, kind)
    }

    /// `env`, the root handle, made of the env handles' prototype from the
    /// run's grant ([`env::root`]).
    value env(state, _granted: EnvGrant; scope) for "@fuzdev/gro/env.ts"
        handle ENV_HANDLE(prototype)
    {
        env::root(state, scope, prototype).map(Into::into)
    }

    /// `env.get(name)`, called on `handle` ([`env::get`]).
    fn get(state, _granted: EnvGrant; scope, name) for "@fuzdev/gro/env.ts" on ENV_HANDLE(handle)
        throws ENV_ERROR(errors)
    {
        env::get(state, scope, handle, name, errors)
    }

    /// `env.get_all()`, called on `handle` ([`env::get_all`]).
    fn get_all(state, _granted: EnvGrant; scope) for "@fuzdev/gro/env.ts" on ENV_HANDLE(handle) {
        env::get_all(scope, handle)
    }

    /// `env.narrow(names)`, called on `handle` ([`env::narrow`]).
    fn narrow(state, _granted: EnvGrant; scope, names) for "@fuzdev/gro/env.ts" on ENV_HANDLE(handle)
        throws ENV_ERROR(errors)
    {
        env::narrow(state, scope, handle, names, errors)
    }

    /// `pid`, the process's id, asked of the OS when the run first imports
    /// the module.
    value pid(state; scope) for "@fuzdev/gro/pid.ts" {
        Some(v8::Integer::new_from_unsigned(scope, std::process::id()).into())
    }

    /// `runtime_version`, the runtime's name and version.
    value runtime_version(state; scope) for "@fuzdev/gro/runtime_version.ts" {
        string(scope, &state.borrow::<HostData>().runtime_version).map(Into::into)
    }

    /// `hash(data)`, the BLAKE3 digest of `data` ([`blake3::hash`]).
    fn hash(state; scope, data) for "@fuzdev/gro/blake3.ts" {
        blake3::hash(scope, data)
    }

    /// `create_hasher()`, a new hasher ([`blake3::hasher`]).
    fn create_hasher(state; scope) for "@fuzdev/gro/blake3.ts" {
        blake3::hasher(state, scope, blake3::Mode::Plain)
    }

    /// `create_keyed_hasher(key)`, a new hasher keyed by `key`
    /// ([`blake3::hasher`]).
    fn create_keyed_hasher(state; scope, key) for "@fuzdev/gro/blake3.ts" {
        blake3::hasher(state, scope, blake3::Mode::Keyed(key))
    }

    /// `create_derive_key_hasher(context)`, a new hasher deriving a key in
    /// `context` ([`blake3::hasher`]).
    fn create_derive_key_hasher(state; scope, context) for "@fuzdev/gro/blake3.ts" {
        blake3::hasher(state, scope, blake3::Mode::DeriveKey(context))
    }

    /// `hasher.update(data)`, called on `hasher` ([`blake3::update`]).
    fn update(state; scope, data) for "@fuzdev/gro/blake3.ts" on BLAKE3_HASHER(hasher) {
        blake3::update(state, scope, hasher, data);
        None
    }

    /// `hasher.finalize()`, called on `hasher` ([`blake3::finalize`]).
    fn finalize(state; scope) for "@fuzdev/gro/blake3.ts" on BLAKE3_HASHER(hasher) {
        blake3::finalize(state, scope, hasher)
    }

    /// `FsError`, the fs host module's error class, the host's
    /// ([`error_class::exported_class`]), extending the realm's `Error` the
    /// namespace op held.
    value FsError(state; scope) for "@fuzdev/gro/fs_error.ts" class FS_ERROR(family) {
        error_class::exported_class(state, scope, family)
    }

    /// `is_fs_error(error, kind?)`, whether `error` is an `FsError`, of
    /// `kind` when it's given ([`error_class::guard`]).
    fn is_fs_error(state; scope, error, kind) for "@fuzdev/gro/fs_error.ts" {
        error_class::guard(scope, &FS_ERROR, error, kind)
    }

    /// `is_own_fs_error(value)`, whether `value` is an `FsError` the host
    /// made, by its brand ([`error_class::is_own`]).
    fn is_own_fs_error(state; scope, value) for "@fuzdev/gro/fs_error.ts" {
        let found = error_class::is_own(scope, &FS_ERROR, value);
        Some(v8::Boolean::new(scope, found).into())
    }

    /// `fs`, the root: the record of its members, the host's, of no handle.
    value fs(state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts" handle FS_ROOT(root) {
        Some(root.into())
    }

    /// `fs.project`, an `FsDir` on the project's root ([`fs::dirs::project`]).
    value project(state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts" on FS_ROOT
        handle FS_DIR(prototype)
    {
        fs::dirs::project(state, scope, prototype)
    }

    /// `fs.open_dir(path)` ([`fs::dirs::open_dir`]).
    async open_dir(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts" on FS_ROOT
        throws FS_ERROR(errors)
    {
        fs::dirs::open_dir(state, scope, path, errors)
    }

    /// `fs.real_path(path)` ([`fs::dirs::real_path`]).
    async real_path(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts" on FS_ROOT
        throws FS_ERROR(errors)
    {
        fs::dirs::real_path(state, scope, path, errors)
    }

    /// `fs.temp_dir(prefix?)` ([`fs::dirs::temp_dir`]).
    async temp_dir(state, _granted: FsGrant; scope, [prefix]) for "@fuzdev/gro/fs.ts" on FS_ROOT
        throws FS_ERROR(errors)
    {
        fs::dirs::temp_dir(state, scope, prefix, errors)
    }

    /// `dir.dir(path)`, called on `handle`, an `FsReadDir`
    /// ([`fs::dirs::dir`]).
    fn dir(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts" on FS_READ_DIR(handle)
        throws FS_ERROR(errors)
    {
        fs::dirs::dir(state, scope, &FS_READ_DIR, handle, path, errors)
    }

    /// `dir.stat(path, options?)`, called on `handle` ([`fs::reads::stat`]).
    async stat(state, _granted: FsGrant; scope, path, options) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::stat(state, scope, handle, path, options, errors)
    }

    /// `dir.exists(path)`, called on `handle` ([`fs::reads::exists`]).
    async exists(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::exists(state, scope, handle, path, errors)
    }

    /// `dir.read(path)`, called on `handle` ([`fs::reads::read`]).
    async read(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::read(state, scope, handle, path, fs::reads::Contents::Bytes, false, errors)
    }

    /// `dir.read_text(path)`, called on `handle` ([`fs::reads::read`]).
    async read_text(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::read(state, scope, handle, path, fs::reads::Contents::Text, false, errors)
    }

    /// `dir.read_if_exists(path)`, called on `handle` ([`fs::reads::read`]).
    async read_if_exists(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::read(state, scope, handle, path, fs::reads::Contents::Bytes, true, errors)
    }

    /// `dir.read_text_if_exists(path)`, called on `handle` ([`fs::reads::read`]).
    async read_text_if_exists(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::read(state, scope, handle, path, fs::reads::Contents::Text, true, errors)
    }

    /// `dir.list(path?)`, called on `handle`, which the listing's first
    /// `next()` reads ([`fs::listing::list`]).
    fn list(state, _granted: FsGrant; scope, [path]) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle)
    {
        fs::listing::list(state, scope, handle, path)
    }

    /// A listing's `next()`, called on `listing` ([`fs::listing::listing_next`]).
    async next(state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts"
        on FS_LISTING(listing) throws FS_ERROR(errors)
    {
        fs::listing::listing_next(state, scope, listing, errors)
    }

    /// A listing's `return(value)`, called on `listing`
    /// ([`fs::listing::listing_return`]).
    async return(state, _granted: FsGrant; scope, value) for "@fuzdev/gro/fs.ts"
        on FS_LISTING(listing)
    {
        fs::listing::listing_return(state, scope, listing, value)
    }

    /// A listing's `throw(error)`, called on `listing`
    /// ([`fs::listing::listing_throw`]).
    async throw(state, _granted: FsGrant; scope, error) for "@fuzdev/gro/fs.ts"
        on FS_LISTING(listing)
    {
        fs::listing::listing_throw(state, scope, listing, error)
    }

    /// A listing's `[Symbol.asyncIterator]()`, which returns its receiver,
    /// whatever it is, as the realm's async iterators' does, reading
    /// nothing of it.
    fn [ASYNC_ITERATOR](state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts"
        on FS_LISTING(listing)
    {
        Some(listing)
    }

    /// `dir.read_link(path)`, called on `handle` ([`fs::reads::read_link`]).
    async read_link(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::read_link(state, scope, handle, path, errors)
    }

    /// `dir.stat_sync(path, options?)`, called on `handle` ([`fs::reads::stat`]).
    fn stat_sync(state, _granted: FsGrant; scope, path, options) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::stat(state, scope, handle, path, options, errors)
    }

    /// `dir.exists_sync(path)`, called on `handle` ([`fs::reads::exists`]).
    fn exists_sync(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::exists(state, scope, handle, path, errors)
    }

    /// `dir.read_text_sync(path)`, called on `handle` ([`fs::reads::read`]).
    fn read_text_sync(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::reads::read(state, scope, handle, path, fs::reads::Contents::Text, false, errors)
    }

    /// `dir.dir(path)`, called on `handle`, an `FsDir` ([`fs::dirs::dir`]).
    fn dir(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts" on FS_DIR(handle)
        throws FS_ERROR(errors)
    {
        fs::dirs::dir(state, scope, &FS_DIR, handle, path, errors)
    }

    /// `dir.readonly()`, called on `handle` ([`fs::dirs::readonly`]).
    fn readonly(state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts" on FS_DIR(handle) {
        fs::dirs::readonly(state, scope, handle)
    }

    /// `dir.write(path, data, options?)` ([`fs::writes::write`]).
    async write(state, _granted: FsGrant; scope, path, data, options) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::writes::write(state, scope, handle, path, data, options, errors)
    }

    /// `dir.mkdir(path, options?)` ([`fs::writes::mkdir`]).
    async mkdir(state, _granted: FsGrant; scope, path, options) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::writes::mkdir(state, scope, handle, path, options, errors)
    }

    /// `dir.remove(path, options?)` ([`fs::writes::remove`]).
    async remove(state, _granted: FsGrant; scope, path, options) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::writes::remove(state, scope, handle, path, options, errors)
    }

    /// `dir.rename(from, to)` ([`fs::writes::rename`]).
    async rename(state, _granted: FsGrant; scope, from, to) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::writes::rename(state, scope, handle, from, to, errors)
    }

    /// `dir.copy(from, to, options?)` ([`fs::writes::copy`]).
    async copy(state, _granted: FsGrant; scope, from, to, options) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::writes::copy(state, scope, handle, from, to, options, errors)
    }

    /// A temporary directory's `[Symbol.asyncDispose]()`, called on
    /// `handle` ([`fs::dirs::dispose`]).
    async [ASYNC_DISPOSE](state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts"
        on FS_TEMP_DIR(handle) throws FS_ERROR(errors)
    {
        fs::dirs::dispose(state, scope, handle, errors)
    }

    /// `dir.set_times(path, times, options?)` ([`fs::writes::set_times`]).
    async set_times(state, _granted: FsGrant; scope, path, times, options)
        for "@fuzdev/gro/fs.ts" on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::writes::set_times(state, scope, handle, path, times, options, errors)
    }

    /// `dir.set_mode(path, mode)` ([`fs::writes::set_mode`]).
    async set_mode(state, _granted: FsGrant; scope, path, mode) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::writes::set_mode(state, scope, handle, path, mode, errors)
    }

    /// `dir.symlink(path, target)` ([`fs::writes::symlink`]).
    async symlink(state, _granted: FsGrant; scope, path, target) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::writes::symlink(state, scope, handle, path, target, errors)
    }
    /// `SpawnError`, the spawn host module's error class, the host's
    /// ([`error_class::exported_class`]), extending the realm's `Error` the
    /// namespace op held.
    value SpawnError(state; scope) for "@fuzdev/gro/spawn_error.ts" class SPAWN_ERROR(family) {
        error_class::exported_class(state, scope, family)
    }

    /// `is_spawn_error(error, kind?)`, whether `error` is a `SpawnError`,
    /// of `kind` when it's given ([`error_class::guard`]).
    fn is_spawn_error(state; scope, error, kind) for "@fuzdev/gro/spawn_error.ts" {
        error_class::guard(scope, &SPAWN_ERROR, error, kind)
    }

    /// `spawn`, the run's root, made of the roots' prototype ([`spawn::root`]).
    value spawn(state, _granted: SpawnGrant; scope) for "@fuzdev/gro/spawn.ts"
        handle SPAWN(prototype)
    {
        spawn::root(scope, prototype).map(Into::into)
    }

    /// `spawn.program(name, options?)`, called on `handle` ([`spawn::program`]).
    fn program(state, _granted: SpawnGrant; scope, name, options) for "@fuzdev/gro/spawn.ts"
        on SPAWN(handle) throws SPAWN_ERROR(errors)
    {
        spawn::program(state, scope, handle, name, options, spawn::Missing::Throws, errors)
    }

    /// `spawn.find(name, options?)`, called on `handle` ([`spawn::program`]).
    fn find(state, _granted: SpawnGrant; scope, name, options) for "@fuzdev/gro/spawn.ts"
        on SPAWN(handle) throws SPAWN_ERROR(errors)
    {
        spawn::program(state, scope, handle, name, options, spawn::Missing::Null, errors)
    }

    /// `spawn.program_in(dir, name, options?)`, called on `handle`
    /// ([`spawn::program_in`]).
    fn program_in(state, _granted: SpawnGrant; scope, dir, name, options)
        for "@fuzdev/gro/spawn.ts" on SPAWN(handle) throws SPAWN_ERROR(errors)
    {
        spawn::program_in(state, scope, "program_in", handle, dir, name, options, errors)
    }

    /// `spawn.find_in(dir, name, options?)`, called on `handle`
    /// ([`spawn::program_in`]).
    fn find_in(state, _granted: SpawnGrant; scope, dir, name, options) for "@fuzdev/gro/spawn.ts"
        on SPAWN(handle) throws SPAWN_ERROR(errors)
    {
        spawn::program_in(state, scope, "find_in", handle, dir, name, options, errors)
    }

    /// `spawn.narrow(names)`, called on `handle` ([`spawn::narrow`]).
    fn narrow(state, _granted: SpawnGrant; scope, names) for "@fuzdev/gro/spawn.ts"
        on SPAWN(handle) throws SPAWN_ERROR(errors)
    {
        spawn::narrow(state, scope, handle, names, errors)
    }

    /// `program.run(args, options?)`, not built ([`spawn::unbuilt`]).
    async run(state, _granted: SpawnGrant; scope, _args, _options) for "@fuzdev/gro/spawn.ts"
        on SPAWN_PROGRAM(handle)
    {
        spawn::unbuilt(state, scope, "run", &SPAWN_PROGRAM, handle)
    }

    /// `program.out(args, options?)`, not built ([`spawn::unbuilt`]).
    async out(state, _granted: SpawnGrant; scope, _args, _options) for "@fuzdev/gro/spawn.ts"
        on SPAWN_PROGRAM(handle)
    {
        spawn::unbuilt(state, scope, "out", &SPAWN_PROGRAM, handle)
    }

    /// `program.out_bytes(args, options?)`, not built ([`spawn::unbuilt`]).
    async out_bytes(state, _granted: SpawnGrant; scope, _args, _options)
        for "@fuzdev/gro/spawn.ts" on SPAWN_PROGRAM(handle)
    {
        spawn::unbuilt(state, scope, "out_bytes", &SPAWN_PROGRAM, handle)
    }

    /// `program.start(args, options?)`, not built ([`spawn::unbuilt`]).
    async start(state, _granted: SpawnGrant; scope, _args, _options) for "@fuzdev/gro/spawn.ts"
        on SPAWN_PROGRAM(handle)
    {
        spawn::unbuilt(state, scope, "start", &SPAWN_PROGRAM, handle)
    }

    /// `child.kill(signal?)`, not built ([`spawn::unbuilt`]).
    fn kill(state, _granted: SpawnGrant; scope, [_signal]) for "@fuzdev/gro/spawn.ts"
        on SPAWN_CHILD(handle)
    {
        spawn::unbuilt(state, scope, "kill", &SPAWN_CHILD, handle)
    }

    /// A child's `[Symbol.asyncDispose]()`, not built ([`spawn::unbuilt`]).
    async [ASYNC_DISPOSE](state, _granted: SpawnGrant; scope) for "@fuzdev/gro/spawn.ts"
        on SPAWN_CHILD(handle)
    {
        spawn::unbuilt(state, scope, "[Symbol.asyncDispose]", &SPAWN_CHILD, handle)
    }

    /// `stdin.write(data)`, not built ([`spawn::unbuilt`]).
    async write(state, _granted: SpawnGrant; scope, _data) for "@fuzdev/gro/spawn.ts"
        on SPAWN_STDIN(handle)
    {
        spawn::unbuilt(state, scope, "write", &SPAWN_STDIN, handle)
    }

    /// `stdin.close()`, not built ([`spawn::unbuilt`]).
    async close(state, _granted: SpawnGrant; scope) for "@fuzdev/gro/spawn.ts"
        on SPAWN_STDIN(handle)
    {
        spawn::unbuilt(state, scope, "close", &SPAWN_STDIN, handle)
    }
}
