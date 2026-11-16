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
pub mod native;

use deno_core::error::ModuleLoaderError as JsErrorBox;
use deno_core::{OpDecl, OpState, op2, v8};

use error_class::{ENV_ERROR, ErrorClasses, FS_ERROR};
use exit::{ExitState, NOT_A_CODE, to_exit_code};
use grant::require_authority;
use handle::{BLAKE3_HASHER, ENV_HANDLE, FS_DIR, FS_LISTING, FS_READ_DIR, FS_ROOT};
use native::ASYNC_ITERATOR;
use native::native_members;

use crate::loader::RefusalLog;
use crate::{ArgsGrant, EnvGrant, EnvVars, ExitGrant, FsGrant};

/// What `exit`'s functions throw in a runtime started without the exit
/// host module's state, which only a runtime made outside a run's start is: a
/// run that grants `exit` holds it.
const NO_EXIT_STATE: &str = "exit is granted, but this runtime was started without its state";

/// What `cwd` throws when the working directory's path isn't UTF-8.
const CWD_NOT_UTF8: &str = "the working directory's path isn't UTF-8, so Gro can't name it";

/// What the namespace op and `EnvError` throw in a runtime started without
/// the run's error classes, which only a runtime made outside a run's start
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
/// the value natively and throws through the scope. The check still comes
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
    /// ([`ErrorClasses::class`]), extending the realm's `Error` the
    /// namespace op held.
    value EnvError(state; scope) for "@fuzdev/gro/env_error.ts" class ENV_ERROR(family) {
        let Some(classes) = state.try_borrow::<ErrorClasses>() else {
            throw_error(scope, NO_ERROR_CLASSES);
            return None;
        };
        classes.class(scope, family).map(Into::into)
    }

    /// `is_env_error(error, kind?)`, whether `error` is an `EnvError`, of
    /// `kind` when it's given ([`error_class::is_of_family`]).
    fn is_env_error(state; scope, error, kind) for "@fuzdev/gro/env_error.ts" {
        let found = error_class::is_of_family(scope, &ENV_ERROR, error, kind).ok()?;
        Some(v8::Boolean::new(scope, found).into())
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
    /// ([`ErrorClasses::class`]), extending the realm's `Error` the
    /// namespace op held.
    value FsError(state; scope) for "@fuzdev/gro/fs_error.ts" class FS_ERROR(family) {
        let Some(classes) = state.try_borrow::<ErrorClasses>() else {
            throw_error(scope, NO_ERROR_CLASSES);
            return None;
        };
        classes.class(scope, family).map(Into::into)
    }

    /// `is_fs_error(error, kind?)`, whether `error` is an `FsError`, of
    /// `kind` when it's given ([`error_class::is_of_family`]).
    fn is_fs_error(state; scope, error, kind) for "@fuzdev/gro/fs_error.ts" {
        let found = error_class::is_of_family(scope, &FS_ERROR, error, kind).ok()?;
        Some(v8::Boolean::new(scope, found).into())
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

    /// `fs.project`, an `FsDir` on the project's root ([`fs::project`]).
    value project(state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts" on FS_ROOT
        handle FS_DIR(prototype)
    {
        fs::project(state, scope, prototype)
    }

    /// `fs.open_dir(path)` ([`fs::open_dir`]).
    async open_dir(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts" on FS_ROOT
        throws FS_ERROR(errors)
    {
        fs::open_dir(state, scope, path, errors)
    }

    /// `fs.real_path(path)` ([`fs::real_path`]).
    async real_path(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts" on FS_ROOT
        throws FS_ERROR(errors)
    {
        fs::real_path(state, scope, path, errors)
    }

    /// `fs.temp_dir(prefix?)`, not built ([`fs::unbuilt`]).
    async temp_dir(state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts" on FS_ROOT {
        fs::unbuilt(state, scope, "temp_dir", None)
    }

    /// `dir.dir(path)`, called on `handle`, an `FsReadDir`
    /// ([`fs::dir`]).
    fn dir(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts" on FS_READ_DIR(handle)
        throws FS_ERROR(errors)
    {
        fs::dir(state, scope, &FS_READ_DIR, handle, path, errors)
    }

    /// `dir.stat(path, options?)`, called on `handle` ([`fs::stat`]).
    async stat(state, _granted: FsGrant; scope, path, options) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::stat(state, scope, handle, path, options, errors)
    }

    /// `dir.exists(path)`, called on `handle` ([`fs::exists`]).
    async exists(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::exists(state, scope, handle, path, errors)
    }

    /// `dir.read(path)`, called on `handle` ([`fs::read`]).
    async read(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::read(state, scope, handle, path, fs::Contents::Bytes, false, errors)
    }

    /// `dir.read_text(path)`, called on `handle` ([`fs::read`]).
    async read_text(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::read(state, scope, handle, path, fs::Contents::Text, false, errors)
    }

    /// `dir.read_if_exists(path)`, called on `handle` ([`fs::read`]).
    async read_if_exists(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::read(state, scope, handle, path, fs::Contents::Bytes, true, errors)
    }

    /// `dir.read_text_if_exists(path)`, called on `handle` ([`fs::read`]).
    async read_text_if_exists(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::read(state, scope, handle, path, fs::Contents::Text, true, errors)
    }

    /// `dir.list(path?)`, called on `handle`, which the listing's first
    /// `next()` reads ([`fs::list`]).
    fn list(state, _granted: FsGrant; scope, [path]) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle)
    {
        fs::list(state, scope, handle, path)
    }

    /// A listing's `next()`, called on `listing` ([`fs::listing_next`]).
    async next(state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts"
        on FS_LISTING(listing) throws FS_ERROR(errors)
    {
        fs::listing_next(state, scope, listing, errors)
    }

    /// A listing's `return(value)`, called on `listing`
    /// ([`fs::listing_return`]).
    async return(state, _granted: FsGrant; scope, value) for "@fuzdev/gro/fs.ts"
        on FS_LISTING(listing)
    {
        fs::listing_return(state, scope, listing, value)
    }

    /// A listing's `throw(error)`, called on `listing`
    /// ([`fs::listing_throw`]).
    async throw(state, _granted: FsGrant; scope, error) for "@fuzdev/gro/fs.ts"
        on FS_LISTING(listing)
    {
        fs::listing_throw(state, scope, listing, error)
    }

    /// A listing's `[Symbol.asyncIterator]()`, which returns its receiver,
    /// whatever it is, as the realm's async iterators' does, reading
    /// nothing of it.
    fn [ASYNC_ITERATOR](state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts"
        on FS_LISTING(listing)
    {
        Some(listing)
    }

    /// `dir.read_link(path)`, called on `handle` ([`fs::read_link`]).
    async read_link(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::read_link(state, scope, handle, path, errors)
    }

    /// `dir.stat_sync(path, options?)`, called on `handle` ([`fs::stat`]).
    fn stat_sync(state, _granted: FsGrant; scope, path, options) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::stat(state, scope, handle, path, options, errors)
    }

    /// `dir.exists_sync(path)`, called on `handle` ([`fs::exists`]).
    fn exists_sync(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::exists(state, scope, handle, path, errors)
    }

    /// `dir.read_text_sync(path)`, called on `handle` ([`fs::read`]).
    fn read_text_sync(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts"
        on FS_READ_DIR(handle) throws FS_ERROR(errors)
    {
        fs::read(state, scope, handle, path, fs::Contents::Text, false, errors)
    }

    /// `dir.dir(path)`, called on `handle`, an `FsDir` ([`fs::dir`]).
    fn dir(state, _granted: FsGrant; scope, path) for "@fuzdev/gro/fs.ts" on FS_DIR(handle)
        throws FS_ERROR(errors)
    {
        fs::dir(state, scope, &FS_DIR, handle, path, errors)
    }

    /// `dir.readonly()`, called on `handle` ([`fs::readonly`]).
    fn readonly(state, _granted: FsGrant; scope) for "@fuzdev/gro/fs.ts" on FS_DIR(handle) {
        fs::readonly(state, scope, handle)
    }

    /// `dir.write(path, data, options?)`, refused ([`fs::refuse_write`]).
    async write(state, _granted: FsGrant; scope, path, _data, _options) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::refuse_write(state, scope, handle, &[path], errors)
    }

    /// `dir.mkdir(path, options?)`, refused ([`fs::refuse_write`]).
    async mkdir(state, _granted: FsGrant; scope, path, _options) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::refuse_write(state, scope, handle, &[path], errors)
    }

    /// `dir.remove(path, options?)`, refused ([`fs::refuse_write`]).
    async remove(state, _granted: FsGrant; scope, path, _options) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::refuse_write(state, scope, handle, &[path], errors)
    }

    /// `dir.rename(from, to)`, refused ([`fs::refuse_write`]).
    async rename(state, _granted: FsGrant; scope, from, to) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::refuse_write(state, scope, handle, &[from, to], errors)
    }

    /// `dir.copy(from, to, options?)`, refused ([`fs::refuse_write`]).
    async copy(state, _granted: FsGrant; scope, from, to, _options) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::refuse_write(state, scope, handle, &[from, to], errors)
    }

    /// `dir.set_times(path, times, options?)`, refused
    /// ([`fs::refuse_write`]).
    async set_times(state, _granted: FsGrant; scope, path, _times, _options)
        for "@fuzdev/gro/fs.ts" on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::refuse_write(state, scope, handle, &[path], errors)
    }

    /// `dir.set_mode(path, mode)`, refused ([`fs::refuse_write`]).
    async set_mode(state, _granted: FsGrant; scope, path, _mode) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::refuse_write(state, scope, handle, &[path], errors)
    }

    /// `dir.symlink(path, target)`, refused ([`fs::refuse_write`]).
    async symlink(state, _granted: FsGrant; scope, path, _target) for "@fuzdev/gro/fs.ts"
        on FS_DIR(handle) throws FS_ERROR(errors)
    {
        fs::refuse_write(state, scope, handle, &[path], errors)
    }
}
