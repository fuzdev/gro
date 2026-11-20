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
//! and the state of a handle with native state, `blake3`'s hasher and
//! `tsv`'s ignore stack, is a number naming a Rust value in a
//! [`NativeTable`] of the run's `OpState`, which a weak reference to the
//! handle drops when the handle is collected.
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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

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

/// The tsv host module's ignore stacks, its `TsvIgnoreStack`.
pub const TSV_IGNORE_STACK: HandleKind = HandleKind {
    brand: "@fuzdev/gro/tsv.ts#TsvIgnoreStack",
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

/// The native state behind a kind of handle, one Rust value per handle, in
/// a table of the run's `OpState`: a handle's state is a number naming its
/// value, held beside a weak reference to the handle, whose finalizer takes
/// the value out once the handle is collected, so a value lives as long as
/// its handle can be reached, and no longer, with no call to end it.
///
/// No borrow of the table is held across a call into V8, which is what
/// could collect a handle ([`with`](Self::with) hands its step the value
/// alone), so a finalizer always finds the table free; were it ever held,
/// the finalizer leaves the value to the run's end rather than fail. The
/// finalizers run after V8 collects, which allocation triggers in any loop,
/// yielding or not. Each value is boxed, so the table holds a pointer per
/// handle, and is counted as external memory to V8 while it lives, its size
/// and what it [`grow`](Self::grow)s by, so collections come as the native
/// state grows, not only as the handles do. What a row grows a value by is
/// its own estimate (a `tsv` ignore stack's, its layers' text, not the rules
/// parsed from it), nothing is ever subtracted before the value is dropped,
/// and nothing bounds a value's memory but the process's.
pub struct NativeTable<T> {
    /// Each value, by the bits of its number, beside the weak reference to
    /// its handle whose finalizer removes it.
    table: RefCell<HashMap<u64, NativeEntry<T>>>,
    /// The number the next handle gets, an integer exact as a JS number:
    /// never one a handle had before.
    next: Cell<f64>,
}

impl<T> Default for NativeTable<T> {
    fn default() -> Self {
        Self {
            table: RefCell::default(),
            next: Cell::new(0.0),
        }
    }
}

/// A value in a [`NativeTable`].
struct NativeEntry<T> {
    /// The value, boxed, so the table holds a pointer per handle and
    /// growing the table copies only that.
    value: Box<T>,
    /// The bytes it's counted as to V8.
    counted: i64,
    /// Its handle, weakly: dropped with the entry, which cancels the
    /// finalizer, and the finalizer drops the entry once the handle is
    /// collected.
    _handle: v8::Weak<v8::Object>,
}

impl<T: 'static> NativeTable<T> {
    /// How many values the table holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.table.borrow().len()
    }

    /// Makes a handle of `kind` of `prototype` whose state names `value`,
    /// which the table holds until the handle is collected. `None` with an
    /// exception pending when V8 failed to make it.
    pub fn make<'s>(
        self: &Rc<Self>,
        scope: &mut v8::PinScope<'s, '_>,
        kind: &'static HandleKind,
        prototype: v8::Local<'s, v8::Object>,
        value: T,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let number = self.next.get();
        self.next.set(number + 1.0);
        let id = number.to_bits();
        let held = v8::Number::new(scope, number);
        let made = make(scope, kind, prototype, held.into(), &[])?;
        let finalized = Rc::clone(self);
        let weak = v8::Weak::with_finalizer(
            scope,
            made,
            Box::new(move |isolate: &mut v8::Isolate| finalized.remove(isolate, id)),
        );
        let counted = i64::try_from(size_of::<T>()).unwrap_or(i64::MAX);
        self.table.borrow_mut().insert(
            id,
            NativeEntry {
                value: Box::new(value),
                counted,
                _handle: weak,
            },
        );
        scope.adjust_amount_of_external_allocated_memory(counted);
        Some(made)
    }

    /// Runs `step` on the value `id` names, `None` when the table holds
    /// none. `step` is given the value alone, so no call into V8 runs while
    /// the table is borrowed.
    pub fn with<R>(&self, id: u64, step: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut table = self.table.try_borrow_mut().ok()?;
        table.get_mut(&id).map(|entry| step(&mut entry.value))
    }

    /// Counts `bytes` more of native state to the value `id` names, when
    /// the table holds it, telling `isolate`.
    pub fn grow(&self, isolate: &mut v8::Isolate, id: u64, bytes: usize) {
        let bytes = i64::try_from(bytes).unwrap_or(i64::MAX);
        let grown = self
            .table
            .try_borrow_mut()
            .ok()
            .and_then(|mut table| {
                let entry = table.get_mut(&id)?;
                entry.counted = entry.counted.saturating_add(bytes);
                Some(())
            })
            .is_some();
        if grown {
            isolate.adjust_amount_of_external_allocated_memory(bytes);
        }
    }

    /// Takes the value `id` names out of the table, when the table is free,
    /// which it always is when a finalizer runs, and tells `isolate` its
    /// native state is gone.
    fn remove(&self, isolate: &mut v8::Isolate, id: u64) {
        let removed = self
            .table
            .try_borrow_mut()
            .ok()
            .and_then(|mut table| table.remove(&id));
        if let Some(entry) = &removed {
            isolate.adjust_amount_of_external_allocated_memory(-entry.counted);
        }
        // dropped once the table's borrow ended: dropping its weak
        // reference reaches the isolate
        drop(removed);
    }
}

/// The number the handle of `kind` `value` is holds, naming its value in a
/// [`NativeTable`], or `None` when `value` is no such handle.
pub fn read_number(
    scope: &v8::PinScope<'_, '_>,
    kind: &HandleKind,
    value: v8::Local<'_, v8::Value>,
) -> Option<u64> {
    let (_, held) = read(scope, kind, value)?;
    // the host's own number, keyed by its bits
    Some(
        v8::Local::<v8::Number>::try_from(held)
            .ok()?
            .value()
            .to_bits(),
    )
}
