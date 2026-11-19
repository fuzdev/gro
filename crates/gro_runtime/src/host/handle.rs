//! Handles: the objects a host module's methods act through, made by the
//! host and branded by it, so a method recognizes one it's called on and
//! refuses anything else.
//!
//! A handle is an ordinary object the host makes of a prototype, the frozen
//! null-prototype record of native methods the host made with the row's
//! namespace (`native.rs`), with the handle's state under a private symbol
//! of its kind, the brand, and then frozen. A private symbol made through
//! V8's API is a key no JS can name, enumerate, or read: `Reflect.ownKeys`
//! doesn't list it, a proxy's traps never see it, and no program can make
//! one. So a look-alike, an object of the same prototype, a copy of a
//! handle's own properties, and a proxy of a handle all lack the brand, and
//! a method refuses each: it checks its receiver is an object, not a proxy,
//! that holds its kind's brand, and reads the state from there, natively,
//! running no code of a caller's, as a member handed another row's handle
//! would check it. A handle's prototype is fixed (it's frozen), so a
//! child handle takes the prototype its parent has.
//!
//! A method decides from its handle's state, never from the run's grants,
//! which gate only that the run holds the row at all: what a handle may
//! reach was fixed when it was made, and a handle made from it only narrows
//! it.
//!
//! The state is a value of the row's choosing, which no JS ever holds:
//! `env`'s is a record of the names a handle may read and their values,
//! and `blake3`'s hasher's is a number naming its native state, a Rust
//! value in a table of the run's `OpState`, which a weak reference to the
//! handle drops when the handle is collected (`blake3.rs`).
//! A handle of another row's kind is no handle of this one's, and a row
//! that must recognize another's handle (`spawn` taking an `fs` directory)
//! reads it with that row's kind.
//!
//! A kind can extend another, its `parent` (`fs`'s `FsDir` over
//! `FsReadDir`): its prototype's own prototype is the parent's, and a
//! handle of it holds the parent's brand beside its own, with the same
//! state, so the parent's methods take it and the child's refuse a handle
//! of the parent. A kind's `fields` are the own data each of its handles
//! holds (an `fs` directory's `path`), given by what makes the handle and
//! defined before it's frozen; a child's handles hold its parent's first.
//! A kind whose methods take no receiver is a record's (`fs`'s root,
//! `Fs`): the host makes the one object of it, of its members, with the
//! namespace, and no handle of it.

use deno_core::v8;

/// A kind of handle: the name its brand, a private symbol, is made for.
/// Each row's own, so no row's member takes another's handle for one of its
/// own.
#[derive(Debug, PartialEq, Eq)]
pub struct HandleKind {
    /// The private symbol's name, in V8's one namespace of API private
    /// symbols, qualified so no other embedder code's collides: the row's
    /// specifier, `#`, and the interface of its Node module that the
    /// kind's handles implement, or of TypeScript's own when the module
    /// exports none of the name (`AsyncIterableIterator`, what `fs`'s
    /// `list` returns), which `src/test/builtin_types.test.ts`
    /// holds the kind's methods to.
    pub brand: &'static str,
    /// The kind it extends, whose prototype its prototype's is and whose
    /// brand its handles hold too, if any.
    pub parent: Option<&'static Self>,
    /// The own data each of its handles holds, beside its parent's.
    pub fields: &'static [Field],
}

/// A field of a kind of handle: own data each of its handles holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Field {
    /// Its name.
    pub name: &'static str,
    /// The kind of handle its value is, where it's one (or `null`), as
    /// the Node interface types it; `None` for any other data.
    pub handle: Option<&'static HandleKind>,
}

impl Field {
    /// A field of data that is no handle.
    pub const fn data(name: &'static str) -> Self {
        Self { name, handle: None }
    }
}

/// The env host module's handles, its `Env`.
pub const ENV_HANDLE: HandleKind = HandleKind {
    brand: "@fuzdev/gro/env.ts#Env",
    parent: None,
    fields: &[],
};

/// The blake3 host module's hashers, its `Blake3Hasher`.
pub const BLAKE3_HASHER: HandleKind = HandleKind {
    brand: "@fuzdev/gro/blake3.ts#Blake3Hasher",
    parent: None,
    fields: &[],
};

/// The fs host module's root, its `Fs`: a record, of no handle, whose
/// functions take no receiver.
pub const FS_ROOT: HandleKind = HandleKind {
    brand: "@fuzdev/gro/fs.ts#Fs",
    parent: None,
    fields: &[],
};

/// The fs host module's read-only directories, its `FsReadDir`, each
/// holding its directory's absolute path as its `path`.
pub const FS_READ_DIR: HandleKind = HandleKind {
    brand: "@fuzdev/gro/fs.ts#FsReadDir",
    parent: None,
    fields: &[Field::data("path")],
};

/// The fs host module's directories, its `FsDir`, extending `FsReadDir`.
pub const FS_DIR: HandleKind = HandleKind {
    brand: "@fuzdev/gro/fs.ts#FsDir",
    parent: Some(&FS_READ_DIR),
    fields: &[],
};

/// The fs host module's temporary directories, what `temp_dir` returns: an
/// `FsDir` that is a TypeScript `AsyncDisposable` too, as the Node module's
/// `NodeFsTempDir` is, extending `FsDir`.
pub const FS_TEMP_DIR: HandleKind = HandleKind {
    brand: "@fuzdev/gro/fs.ts#AsyncDisposable",
    parent: Some(&FS_DIR),
    fields: &[],
};

/// The fs host module's listings, what `list` returns: an async iterator
/// of a directory's entries, a TypeScript `AsyncIterableIterator`, as the
/// Node module's async generator is.
pub const FS_LISTING: HandleKind = HandleKind {
    brand: "@fuzdev/gro/fs.ts#AsyncIterableIterator",
    parent: None,
    fields: &[],
};

/// The spawn host module's roots, its `Spawn`: the run's, made from its
/// grant, and each narrowed from one.
pub const SPAWN: HandleKind = HandleKind {
    brand: "@fuzdev/gro/spawn.ts#Spawn",
    parent: None,
    fields: &[],
};

/// The spawn host module's programs, its `SpawnProgram`, each holding the
/// name it was asked for as its `name` and its pin's real path as its
/// `path`.
pub const SPAWN_PROGRAM: HandleKind = HandleKind {
    brand: "@fuzdev/gro/spawn.ts#SpawnProgram",
    parent: None,
    fields: &[Field::data("name"), Field::data("path")],
};

/// The spawn host module's started children, its `SpawnChild`, each
/// holding its `pid`, its `closed` promise, its output streams, and its
/// `stdin`.
pub const SPAWN_CHILD: HandleKind = HandleKind {
    brand: "@fuzdev/gro/spawn.ts#SpawnChild",
    parent: None,
    fields: &[
        Field::data("pid"),
        Field::data("closed"),
        Field {
            name: "stdout",
            handle: Some(&SPAWN_OUTPUT),
        },
        Field {
            name: "stderr",
            handle: Some(&SPAWN_OUTPUT),
        },
        Field {
            name: "stdin",
            handle: Some(&SPAWN_STDIN),
        },
    ],
};

/// The spawn host module's children's stdin, its `SpawnStdin`.
pub const SPAWN_STDIN: HandleKind = HandleKind {
    brand: "@fuzdev/gro/spawn.ts#SpawnStdin",
    parent: None,
    fields: &[],
};

/// The spawn host module's children's piped output streams, each a
/// TypeScript `AsyncIterable` of chunks, as the Node module's are.
pub const SPAWN_OUTPUT: HandleKind = HandleKind {
    brand: "@fuzdev/gro/spawn.ts#AsyncIterable",
    parent: None,
    fields: &[],
};

/// The iterator of a child's output stream, what its
/// `[Symbol.asyncIterator]()` returns, a TypeScript `AsyncIterator`, as the
/// Node module's is.
pub const SPAWN_OUTPUT_ITERATOR: HandleKind = HandleKind {
    brand: "@fuzdev/gro/spawn.ts#AsyncIterator",
    parent: None,
    fields: &[],
};

impl HandleKind {
    /// The kind and each it extends, the nearest first.
    pub fn lineage(&'static self) -> impl Iterator<Item = &'static Self> {
        std::iter::successors(Some(self), |kind| kind.parent)
    }

    /// The names of the own data its handles hold, its farthest ancestor's
    /// first.
    pub fn all_fields(&'static self) -> Vec<&'static str> {
        let mut lineage: Vec<&Self> = self.lineage().collect();
        lineage.reverse();
        lineage
            .into_iter()
            .flat_map(|kind| kind.fields.iter().map(|field| field.name))
            .collect()
    }
}

/// The brand of `kind`: the same private symbol every time, which V8 keeps
/// for the isolate.
fn brand<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, kind.brand)?;
    Some(v8::Private::for_api(scope, Some(name)))
}

/// What making a handle throws when what makes it gives other own data
/// than its kind's fields, naming the kind.
fn not_its_fields(kind: &HandleKind) -> String {
    format!(
        "a handle of {} holds one value for each of its fields",
        kind.brand
    )
}

/// Makes a handle of `kind`: a new object whose prototype is `prototype`,
/// holding `own`, a value for each of the kind's fields in order
/// ([`HandleKind::all_fields`]), as its own data, and `state` under the
/// brand of the kind and of each kind it extends, then frozen. `None` when
/// V8 failed to make it, or with an `Error` thrown when `own` isn't one
/// value a field, an exception pending.
pub fn make<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &'static HandleKind,
    prototype: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
    own: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Object>> {
    let fields = kind.all_fields();
    if fields.len() != own.len() {
        super::throw_error(scope, &not_its_fields(kind));
        return None;
    }
    let mut names = Vec::with_capacity(fields.len());
    for field in fields {
        names.push(v8::String::new(scope, field)?.into());
    }
    let handle = v8::Object::with_prototype_and_properties(scope, prototype.into(), &names, own);
    for kind in kind.lineage() {
        handle.set_private(scope, brand(scope, kind)?, state)?;
    }
    handle.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    Some(handle)
}

/// A handle of `kind`, as a member is handed it: the object `value` is and
/// the state its brand holds, or `None` when `value` is anything else (not
/// an object, a proxy, or an object without the brand). Reads nothing a
/// caller can answer.
pub fn read<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
    value: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Value>)> {
    if value.is_proxy() {
        return None;
    }
    let handle = v8::Local::<v8::Object>::try_from(value).ok()?;
    let brand = brand(scope, kind)?;
    if handle.has_private(scope, brand) != Some(true) {
        return None;
    }
    Some((handle, handle.get_private(scope, brand)?))
}

/// The prototype `handle`, a handle [`read`] returned, was made of, for a
/// child handle to take: fixed, since a handle is frozen.
pub fn prototype_of<'s>(
    scope: &v8::PinScope<'s, '_>,
    handle: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::Local::<v8::Object>::try_from(handle.get_prototype(scope)?).ok()
}
