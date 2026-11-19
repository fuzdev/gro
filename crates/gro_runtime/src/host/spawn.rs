//! The native half of the spawn host module: its roots, the programs they
//! make handles of, and every check of a call.
//!
//! A root is a handle of [`SPAWN`]'s kind (`handle.rs`) whose state names
//! its scope, by its place in the run's table of scopes ([`SpawnRun`]), and
//! the set of names it may make programs of: `null` for the run's root, a
//! value of the row's namespace, which names the project's scope, the one a
//! run has; and a frozen null-prototype record of names for each root
//! `narrow` makes, which names its parent's scope. A scope is what its
//! roots may make: in a confined run, the programs `--allow-run=` granted,
//! each pinned as the run started ([`pins`]), with the variables
//! `--allow-run-env=` forwards to it; in an unconfined one, any program,
//! found when it's asked for, passed any variable. Every method decides
//! from the root it's called on, reading the scope its state names, never
//! the run's as a whole: a narrowed root holds names its scope held when it
//! was narrowed, and refuses what the Node module's narrowed root refuses
//! (`env`, `pass_env: 'all'`, a `pass_env` glob, and `program_in`). A
//! scope's exec roots are its own too ([`exec_roots`]), opened as the run
//! starts, beside the pins.
//!
//! A program is a handle of [`SPAWN_PROGRAM`]'s kind, holding the name it
//! was asked for and its pin's real path as its `name` and `path`, and, as
//! its state, the place of its pin in the run's table and the environment
//! its children are given, made once, when it's made: the base set the Node
//! module gives (`PATH`, `HOME`, `TMPDIR`, and `TERM` where the embedder
//! read them, `LC_ALL=C`, and `NO_COLOR` and `FORCE_COLOR`), then what
//! `pass_env` names, then `env`'s values, a `PATH` from any of them keeping
//! only its absolute entries. Its variables are read from what the embedder
//! read for children
//! ([`RuntimeOptions::spawn_env`](crate::RuntimeOptions::spawn_env)), never
//! from the process, and no member returns them.
//!
//! A call is checked as the Node module checks it, in the same order and
//! with the same messages: the receiver, the name (a string, no NUL,
//! well-formed, not empty), the options, read once (`pass_env`'s names and
//! globs, `omit_env` only with `'all'`, `env`'s names and values), a
//! narrowed root's refusals, the root's set, then, in a confined run, that
//! the program's grant forwards each variable `pass_env` names, a name of
//! the base set, which every child is given, held without one, a glob only
//! within a glob it grants, and `'all'` never; then the pin. An argument is
//! read as the value it is, converting nothing: an options record's fields
//! are its own data, and an array's elements their own data, so a getter or
//! a proxy is a `TypeError` where the Node module would run it, and an
//! element given through a getter is none.
//!
//! A refusal for a grant is `not_granted`, recorded in the run's record,
//! which fails the run caught or not, with the grant that would allow it
//! where one would: the program, refused by the run's root, or the variable
//! to forward, refused by any root, since the program's grant decides it;
//! never a name a narrowed root's set lacks, which no grant widens.
//!
//! `program_in` and `find_in` check their handle, name, and options, and
//! then the directory, by its real path, against the exec roots of the
//! root's scope: one the scope holds none of is `not_granted`, needing an
//! exec root, and beneath one, the name resolves from the exec root's own
//! descriptor, by the handle's rules, and pins as a granted program does
//! ([`exec_roots`]). An unconfined scope's exec root is `/`.
//!
//! A program's `run`, `out`, and `out_bytes` launch a child and settle in a
//! later turn ([`launch`]), its watch a task on the event loop's own
//! runtime, and the run's table of children killed when the run ends
//! ([`watch`]). `start` launches one the same way and settles with its
//! `SpawnChild` ([`start`]), whose state and long-lived task are
//! [`child`]'s.

pub mod child;
pub mod exec_roots;
pub mod launch;
pub mod pins;
pub mod start;
pub mod watch;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use deno_core::{OpState, v8};

use super::error_class::{DeclaredFamily, ErrorClasses};
use super::handle::{self, FS_READ_DIR, HandleKind, SPAWN, SPAWN_PROGRAM};
use super::{
    HostData, NO_ERROR_CLASSES, name, native, own_data, string, throw_error, throw_type_error,
};
use crate::loader::RefusalLog;
use crate::loader::refused::{NOT_GRANTED, NotGranted, SpawnNeed};
use crate::{Authority, CHILD_BASE_ENV, EnvPattern, EnvPatternFault, EnvVars, Grants};
use exec_roots::ExecRoot;
use pins::{Fault, Pin};
use watch::Children;

/// The spawn namespace's specifier, which holds the run's root as its
/// `spawn`.
const MODULE: &str = "@fuzdev/gro/spawn.ts";

/// What a method called on anything but a handle of its kind throws.
const NOT_A_HANDLE: &str =
    "spawn's methods are called on a spawn handle of their kind, not another object";

/// What `program_in` and `find_in` throw for a directory that isn't an fs
/// handle, as the Node module's registry throws it.
const NOT_AN_FS_HANDLE: &str = "expected an fs directory handle from this copy of Gro";

/// What a member throws in a runtime started without the run's spawn
/// state, which only a runtime made outside a run's start is.
const NO_SPAWN_STATE: &str = "this runtime was started without the spawn host module's state";

/// What a refused call throws in a runtime started without the run's error
/// classes or its record of refusals, which only a runtime made outside a
/// run's start is: a refusal that couldn't fail the run is an error itself.
const NO_REFUSAL_STATE: &str = "spawn refused a call, but this runtime was started without the \
                                run's record of refusals, so the refusal can't fail the run";

/// What a root names that no scope of the run's is at, which no root the
/// host made does.
const NO_SCOPE: &str = "this spawn root names no scope the run holds";

/// The place of the project's scope in a run's table, which the run's root
/// names: the one scope a run has.
const PROJECT_SCOPE: usize = 0;

/// The run's spawn state, made as it starts, before any module evaluates:
/// the variables a child may be given, the project's root, which a name
/// holding `/` resolves beneath, and its directory, a child's working
/// directory when a call names none, the run's table of scopes, the run's
/// pins, and its children not reaped yet.
#[derive(Debug)]
pub struct SpawnRun {
    /// The project's directory, opened as a path only as the run started,
    /// or why it couldn't be, which fails a launch that needs it.
    project_dir: Result<OwnedFd, String>,
    /// The children launched and not reaped yet, which the run kills when
    /// it ends.
    children: Arc<Children>,
    /// Whether the signal handler that kills them is installed.
    guarded: Cell<bool>,
    /// The variables a child may be given, as the embedder read them.
    env: EnvVars,
    /// The project's root, a real path, unset when it isn't UTF-8.
    project: Option<String>,
    /// The run's scopes, each by the place a root's state names: the
    /// project's at [`PROJECT_SCOPE`].
    scopes: Vec<Scope>,
    /// The run's pins, each program's place its handle holds: the granted
    /// ones, pinned as the run started, then, in an unconfined run, each
    /// file pinned since, once for each way it's run.
    pins: RefCell<Vec<Pin>>,
    /// The run's started children, by the number their handles name
    /// ([`start::StartedChildren`]).
    started: Rc<start::StartedChildren>,
}

/// What the roots naming a scope may make: its programs, and the exec
/// roots `program_in` and `find_in` find one beneath.
#[derive(Debug)]
struct Scope {
    /// The programs it makes by name.
    programs: Programs,
    /// Its exec roots, each opened as the run started: an unconfined
    /// scope's one, `/`, or those its grant names.
    exec_roots: Vec<ExecRoot>,
}

/// The programs a scope makes by name.
#[derive(Debug)]
enum Programs {
    /// Any program, found when it's asked for, passed any variable.
    Unconfined,
    /// The programs granted, by name.
    Granted(BTreeMap<String, GrantedProgram>),
}

/// A program the run grants.
#[derive(Debug)]
struct GrantedProgram {
    /// Its pin's place in the run's table, or why it pinned nothing.
    pin: Result<usize, Fault>,
    /// What its `pass_env` may name.
    forward: Vec<EnvPattern>,
}

impl SpawnRun {
    /// The spawn state of a run whose authority is `authority`, `env` the
    /// variables the embedder read for children, `project` the project's
    /// root: none for a run that doesn't grant `spawn`; for one whose grant
    /// names programs, each pinned now, by the rules [`pins::resolve`]
    /// mirrors, a bare name found on `env`'s `PATH`, and each exec root it
    /// names opened now and held to the identity it was granted with
    /// ([`ExecRoot::open`]); for an unconfined one, `/` its exec root.
    pub fn start(authority: &Authority, env: &EnvVars, project: &Path) -> Option<Self> {
        let grant = match authority {
            Authority::Unconfined => None,
            Authority::Confined(Grants { spawn: None, .. }) => return None,
            Authority::Confined(Grants {
                spawn: Some(grant), ..
            }) => Some(grant),
        };
        let project_path = project;
        let project = project.to_str().map(ToOwned::to_owned);
        let mut pins = Vec::new();
        let scope = grant.map_or_else(
            || Scope {
                programs: Programs::Unconfined,
                exec_roots: vec![ExecRoot::whole()],
            },
            |grant| Scope {
                programs: Programs::Granted(
                    grant
                        .programs
                        .iter()
                        .map(|(name, program)| {
                            let pin = pin(name, env, project.as_deref()).map(|pin| {
                                pins.push(pin);
                                pins.len() - 1
                            });
                            let granted = GrantedProgram {
                                pin,
                                forward: program.forward.clone(),
                            };
                            (name.clone(), granted)
                        })
                        .collect(),
                ),
                exec_roots: grant
                    .exec_roots
                    .iter()
                    .map(|(path, root)| ExecRoot::open(path, &root.dir, root.forward.clone()))
                    .collect(),
            },
        );
        let project_dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
            .open(project_path)
            .map(OwnedFd::from)
            .map_err(|error| {
                format!(
                    "the project's directory {} can't be opened: {error}",
                    project_path.display()
                )
            });
        Some(Self {
            project_dir,
            children: Arc::default(),
            guarded: Cell::new(false),
            env: env.clone(),
            project,
            scopes: vec![scope],
            pins: RefCell::new(pins),
            started: Rc::default(),
        })
    }

    /// Kills every child launched and not reaped yet, each group it leads
    /// with it ([`Children::kill_all`]): what a run's end, and its `exit`,
    /// do before the process goes on, so no child outlives the run that
    /// launched it.
    pub fn kill_children(&self) {
        self.children.kill_all();
    }

    /// Installs, at the run's first launch, the signal handler that kills
    /// its groups and passes a `SIGTERM` on to its other children when a
    /// signal ends the process (`gro_sys::guard_children_on_signal`).
    ///
    /// # Errors
    ///
    /// Returns why it couldn't be installed.
    fn guard_on_signal(&self) -> Result<(), String> {
        if !self.guarded.get() {
            gro_sys::guard_children_on_signal()
                .map_err(|error| format!("its signal handler couldn't be installed: {error}"))?;
            self.guarded.set(true);
        }
        Ok(())
    }

    /// The descriptor of the project's directory, a child's working
    /// directory when a call names none.
    ///
    /// # Errors
    ///
    /// Returns why it couldn't be opened as the run started.
    fn project_dir(&self) -> Result<RawFd, String> {
        self.project_dir
            .as_ref()
            .map(AsRawFd::as_raw_fd)
            .map_err(Clone::clone)
    }

    /// The scope `root` names, or a plain `Error` for a place the run's
    /// table doesn't hold.
    fn scope(&self, root: &Root<'_>) -> Result<&Scope, Failure> {
        self.scopes
            .get(root.scope)
            .ok_or_else(|| Failure::Plain(NO_SCOPE.to_owned()))
    }

    /// `name` resolved and pinned now, for an unconfined run's root, and
    /// its place in the table ([`SpawnRun::hold_pin`]).
    fn pin_now(&self, name: &str) -> Result<usize, Fault> {
        let found = pin(name, &self.env, self.project.as_deref())?;
        Ok(self.hold_pin(found))
    }

    /// The place of `found` in the run's table: an entry already there
    /// that pins the same file the same way, or a new one.
    fn hold_pin(&self, found: Pin) -> usize {
        let mut pins = self.pins.borrow_mut();
        if let Some(at) = pins.iter().position(|held| held.same(&found)) {
            return at;
        }
        pins.push(found);
        pins.len() - 1
    }
}

/// `name` resolved by [`pins::resolve`], a bare one on `env`'s `PATH` and
/// one holding `/` beneath `project`, which it can't be when the project's
/// root has no path a string can name.
fn pin(name: &str, env: &EnvVars, project: Option<&str>) -> Result<Pin, Fault> {
    match project {
        Some(project) => pins::resolve(name, env.get("PATH"), project),
        None if name.contains('/') => Err(Fault::NotGranted(format!(
            "{name} resolves beneath the project, whose root has no path a string can name"
        ))),
        None => pins::resolve(name, env.get("PATH"), "/"),
    }
}

/// The `kind` of a `SpawnError` the module throws, of the closed set
/// `spawn_error.ts`'s `SpawnErrorKind` declares; `not_granted` is a
/// refusal's ([`Failure::NotGranted`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpawnErrorKind {
    /// No program at the name.
    NotFound,
    /// What's at the name can't be run.
    NotExecutable,
    /// The program didn't start.
    LaunchFailed,
}

impl SpawnErrorKind {
    /// The name a `SpawnError` carries as its `kind`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::NotExecutable => "not_executable",
            Self::LaunchFailed => "launch_failed",
        }
    }
}

/// Why a call failed, before it's thrown.
enum Failure {
    /// A `TypeError` of this message: an argument or receiver that's none.
    Type(String),
    /// A `SpawnError` of `kind` about `program`, its `cause` an `Error` of
    /// this message, whose own `cause`, when there's `inner`, is an `Error`
    /// of that message: what the OS said.
    Spawn {
        kind: SpawnErrorKind,
        program: String,
        cause: String,
        inner: Option<String>,
    },
    /// `not_granted` about `program`, its `cause` an `Error` of this
    /// message, recorded as it's thrown with what would have allowed it.
    NotGranted {
        program: String,
        cause: String,
        needed: Option<SpawnNeed>,
    },
    /// A plain `Error` of this message: what isn't the call's to fix.
    Plain(String),
    /// What V8 threw, already pending.
    Thrown,
}

impl Failure {
    /// The failure a call asking for `name` meets for `fault`, its pin's.
    fn of(name: &str, fault: Fault) -> Self {
        match fault {
            Fault::NotFound => Self::Spawn {
                kind: SpawnErrorKind::NotFound,
                program: name.to_owned(),
                cause: format!("no program {name}"),
                inner: None,
            },
            Fault::NotExecutable(cause) => Self::Spawn {
                kind: SpawnErrorKind::NotExecutable,
                program: name.to_owned(),
                cause,
                inner: None,
            },
            Fault::Unresolved { cause, why } => Self::Spawn {
                kind: SpawnErrorKind::NotExecutable,
                program: name.to_owned(),
                cause,
                inner: Some(why),
            },
            Fault::NotGranted(cause) => Self::NotGranted {
                program: name.to_owned(),
                cause,
                needed: None,
            },
        }
    }
}

/// Throws `failure` through `scope`: a `SpawnError` of `errors`, the
/// family the member declares, its refusal recorded in the run's record
/// first.
fn throw(state: &OpState, scope: &v8::PinScope<'_, '_>, errors: DeclaredFamily, failure: Failure) {
    let mut inner = None;
    let (kind, program, cause) = match failure {
        Failure::Thrown => return,
        Failure::Type(message) => return throw_type_error(scope, &message),
        Failure::Plain(message) => return throw_error(scope, &message),
        Failure::Spawn {
            kind,
            program,
            cause,
            inner: why,
        } => {
            inner = why;
            (kind.as_str(), program, cause)
        }
        Failure::NotGranted {
            program,
            cause,
            needed,
        } => {
            let Some(log) = state.try_borrow::<RefusalLog>() else {
                return throw_error(scope, NO_REFUSAL_STATE);
            };
            // recorded as it's thrown, so the run fails whatever the
            // program does with the error
            log.record_not_granted(NotGranted::Spawn(needed), &program);
            (NOT_GRANTED, program, cause)
        }
    };
    let Some(classes) = state.try_borrow::<ErrorClasses>() else {
        return throw_error(scope, NO_ERROR_CLASSES);
    };
    let Some(target) = string(scope, &program) else {
        return;
    };
    let Some(cause) = string(scope, &cause) else {
        return;
    };
    let cause = v8::Exception::error(scope, cause);
    if let Some(inner) = inner {
        // what the OS said, the cause's own `cause`, as an `Error`'s
        // options would define it: not enumerable
        let (Some(object), Some(key), Some(inner)) = (
            cause.to_object(scope),
            name(scope, "cause"),
            string(scope, &inner),
        ) else {
            return;
        };
        let inner = v8::Exception::error(scope, inner);
        if object
            .define_own_property(scope, key, inner, v8::PropertyAttribute::DONT_ENUM)
            .is_none()
        {
            return;
        }
    }
    classes.throw(scope, errors, kind, target, Some(cause));
}

/// `result` as a member returns it, its failure thrown.
fn finish<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    errors: DeclaredFamily,
    result: Result<v8::Local<'s, v8::Value>, Failure>,
) -> Option<v8::Local<'s, v8::Value>> {
    result
        .map_err(|failure| throw(state, scope, errors, failure))
        .ok()
}

/// The run's spawn state, or a plain `Error` in a runtime made outside a
/// run's start.
fn run_state(state: &OpState) -> Result<&SpawnRun, Failure> {
    state
        .try_borrow::<HostData>()
        .and_then(|data| data.spawn.as_deref())
        .ok_or_else(|| Failure::Plain(NO_SPAWN_STATE.to_owned()))
}

/// The keys of a root's state: the place of its scope in the run's table,
/// and its names, `null` for the run's root.
const SCOPE: &str = "scope";

/// See [`SCOPE`].
const NAMES: &str = "names";

/// The keys of a program's state: its pin's place in the run's table, and
/// its children's environment.
const PIN: &str = "pin";

/// See [`PIN`].
const ENV: &str = "env";

/// The run's root, made of `prototype`, the roots' prototype the host is
/// making the namespace with: its state naming the project's scope and
/// its names `null`, so it makes what that scope allows. `None` with an
/// exception pending when a step failed.
pub fn root<'s>(
    scope: &v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    make_root(scope, prototype, PROJECT_SCOPE, v8::null(scope).into())
}

/// A root of `prototype` whose state names the scope at `place` in the
/// run's table and the names `names`, a frozen record or `null`.
fn make_root<'s>(
    scope: &v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    place: usize,
    names: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let place = v8::Integer::new_from_unsigned(scope, u32::try_from(place).ok()?).into();
    let held = frozen_record(scope, &[(SCOPE, place), (NAMES, names)])?;
    handle::make(scope, &SPAWN, prototype, held.into(), &[])
}

/// A new frozen null-prototype record of `entries`.
fn frozen_record<'s>(
    scope: &v8::PinScope<'s, '_>,
    entries: &[(&str, v8::Local<'s, v8::Value>)],
) -> Option<v8::Local<'s, v8::Object>> {
    let mut keys = Vec::with_capacity(entries.len());
    let mut values = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        keys.push(name(scope, key)?);
        values.push(*value);
    }
    let null = v8::null(scope).into();
    let record = v8::Object::with_prototype_and_properties(scope, null, &keys, &values);
    record.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    Some(record)
}

/// A root, as a member reads the one it's handed.
struct Root<'s> {
    /// The root.
    object: v8::Local<'s, v8::Object>,
    /// The place of its scope in the run's table.
    scope: usize,
    /// The names it may make programs of, each `true`, or `None` for the
    /// run's root, which makes what its scope allows.
    names: Option<v8::Local<'s, v8::Object>>,
}

impl Root<'_> {
    /// Whether it was narrowed from another, so it refuses what the Node
    /// module's narrowed root refuses: every root but the run's holds names.
    const fn narrowed(&self) -> bool {
        self.names.is_some()
    }
}

/// The root `receiver` is, or a `TypeError` when it's none.
fn read_root<'s>(
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Result<Root<'s>, Failure> {
    let (object, held) = read_handle(scope, &SPAWN, receiver)?;
    // the host's own record, which no JS holds
    let field = |key: &str| {
        name(scope, key)
            .and_then(|key| held.get(scope, key.into()))
            .ok_or(Failure::Thrown)
    };
    let place = v8::Local::<v8::Integer>::try_from(field(SCOPE)?)
        .ok()
        .and_then(|place| usize::try_from(place.value()).ok())
        .ok_or_else(|| Failure::Type(NOT_A_HANDLE.to_owned()))?;
    Ok(Root {
        object,
        scope: place,
        names: v8::Local::<v8::Object>::try_from(field(NAMES)?).ok(),
    })
}

/// The handle of `kind` `receiver` is and its state, the host's record, or
/// a `TypeError` when it's none.
fn read_handle<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
    receiver: v8::Local<'s, v8::Value>,
) -> Result<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>), Failure> {
    let (object, held) = handle::read(scope, kind, receiver)
        .ok_or_else(|| Failure::Type(NOT_A_HANDLE.to_owned()))?;
    let held = v8::Local::<v8::Object>::try_from(held).map_err(|_| Failure::Thrown)?;
    Ok((object, held))
}

/// Whether a missing program is `null` or `not_found`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// `find`'s: `null`.
    Null,
    /// `program`'s: `not_found`.
    Throws,
}

/// `spawn.program(name, options?)` and `spawn.find(name, options?)` called
/// on `receiver`: a program handle for `name`, pinned, with the environment
/// the options make it, or, for a name that pins nothing found, `null` or
/// `not_found` as `missing` says; `errors`, `SpawnError`'s family as the
/// member declares it. `None` with the exception pending when it throws.
pub fn program<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    name: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    missing: Missing,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let run = run_state(state)?;
        let root = read_root(scope, receiver)?;
        let name = program_name(scope, name)?;
        let options = read_program_options(scope, options)?;
        check_narrowed_options(&root, &name, &options)?;
        let granted = allowed(scope, run, &root, &name, Reach::Program)?;
        let pin = match granted {
            Some(granted) => {
                check_forwarding(&name, &options, &granted.forward)?;
                granted.pin.clone()
            }
            None => run.pin_now(&name),
        };
        match pin {
            Ok(at) => make_program(state, scope, run, at, &name, &options),
            Err(Fault::NotFound) if missing == Missing::Null => Ok(v8::null(scope).into()),
            Err(fault) => Err(Failure::of(&name, fault)),
        }
    })();
    finish(state, scope, errors, result)
}

/// `spawn.program_in(dir, name, options?)` and `spawn.find_in(dir, name,
/// options?)`, `member` naming which, called on `receiver`: the directory
/// checked to be an fs handle, then the name, the options, and a narrowed
/// root's refusal; then the directory, by its real path, against the exec
/// roots of the scope the root names, `not_granted` needing one when none
/// holds it, and, in a confined scope, that the exec root's grant forwards
/// what `pass_env` names, as [`program`] checks a granted program's; then
/// `name` resolved beneath the directory from the exec root's own
/// descriptor and pinned ([`ExecRoot::resolve`]), a program handle for it,
/// or, for nothing runnable there, `null` or `not_found` as `missing`
/// says; `errors`, `SpawnError`'s family as the member declares it. `None`
/// with the exception pending when it throws.
#[expect(
    clippy::too_many_arguments,
    reason = "a member's receiver, its three arguments, and what the macro hands it"
)]
pub fn program_in<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    missing: Missing,
    receiver: v8::Local<'s, v8::Value>,
    dir: v8::Local<'s, v8::Value>,
    name: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let run = run_state(state)?;
        let root = read_root(scope, receiver)?;
        if handle::read(scope, &FS_READ_DIR, dir).is_none() {
            return Err(Failure::Type(NOT_AN_FS_HANDLE.to_owned()));
        }
        let name = program_name(scope, name)?;
        let options = read_program_options(scope, options)?;
        if root.narrowed() {
            return Err(Failure::NotGranted {
                cause: format!(
                    "a narrowed spawn can't run {name} beneath a directory: build the program at \
                     the root"
                ),
                program: name,
                needed: None,
            });
        }
        // the directory's real path, read across rows from the fs handle's
        // state
        let dir_path = super::fs::handle_dir_path(state, scope, &FS_READ_DIR, dir)
            .map_err(|_| Failure::Type(NOT_AN_FS_HANDLE.to_owned()))?;
        let held = run.scope(&root)?;
        let Some(exec_root) = exec_roots::holding(&held.exec_roots, &dir_path) else {
            return Err(Failure::NotGranted {
                cause: format!("{name} beneath {dir_path} is in no exec root the run grants"),
                program: name,
                needed: Some(SpawnNeed::ExecRoot),
            });
        };
        if let Some(forward) = &exec_root.forward {
            check_forwarding(&name, &options, forward)?;
        }
        match exec_root.resolve(&name, &dir_path) {
            Ok(Some(found)) => {
                let at = run.hold_pin(found);
                make_program(state, scope, run, at, &name, &options)
            }
            Ok(None) if missing == Missing::Null => Ok(v8::null(scope).into()),
            Ok(None) => Err(Failure::Spawn {
                kind: SpawnErrorKind::NotFound,
                cause: format!("no program {name} beneath {dir_path}"),
                program: name,
                inner: None,
            }),
            Err(fault) => Err(Failure::of(&name, fault)),
        }
    })();
    finish(state, scope, errors, result)
}

/// `spawn.narrow(names)` called on `receiver`: a new root of the
/// receiver's prototype, naming the receiver's scope, that may make
/// exactly `names`, each checked in order, and each one the receiver may
/// make, or `not_granted` is thrown for the first that isn't, as
/// [`program`] throws it. A name given twice is one name. `None` with the
/// exception pending when it throws.
pub fn narrow<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    names: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let run = run_state(state)?;
        let parent = read_root(scope, receiver)?;
        let list = array(names)
            .ok_or_else(|| Failure::Type("spawn.narrow takes an array of names".to_owned()))?;
        let allowed_names =
            v8::Object::with_prototype_and_properties(scope, v8::null(scope).into(), &[], &[]);
        let granted: v8::Local<v8::Value> = v8::Boolean::new(scope, true).into();
        for index in 0..list.length() {
            let element = element(scope, list, index)?;
            let name = program_name(scope, element)?;
            allowed(scope, run, &parent, &name, Reach::Narrow)?;
            let key = string(scope, &name).ok_or(Failure::Thrown)?;
            allowed_names
                .create_data_property(scope, key.into(), granted)
                .ok_or(Failure::Thrown)?;
        }
        allowed_names
            .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
            .ok_or(Failure::Thrown)?;
        let prototype = handle::prototype_of(scope, parent.object).ok_or(Failure::Thrown)?;
        make_root(scope, prototype, parent.scope, allowed_names.into())
            .map(Into::into)
            .ok_or(Failure::Thrown)
    })();
    finish(state, scope, errors, result)
}

/// What a name is checked for, which says the cause of a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// A program to make.
    Program,
    /// A name to narrow to.
    Narrow,
}

/// What the scope `root` names grants `name`, which `root` may make:
/// `None` in an unconfined scope, whose roots make any program; else the
/// program's grant.
///
/// # Errors
///
/// Returns `not_granted` for a name a narrowed root's set lacks, needing
/// nothing, since no grant widens it, and for one the scope's grant lacks,
/// needing the program, so granting it lets the call through; and a plain
/// `Error` for a scope the run's table doesn't hold.
fn allowed<'r>(
    scope: &v8::PinScope<'_, '_>,
    run: &'r SpawnRun,
    root: &Root<'_>,
    name: &str,
    reach: Reach,
) -> Result<Option<&'r GrantedProgram>, Failure> {
    if let Some(names) = root.names {
        let key = string(scope, name).ok_or(Failure::Thrown)?;
        if !names
            .has_own_property(scope, key.into())
            .ok_or(Failure::Thrown)?
        {
            let cause = match reach {
                Reach::Program => format!("{name} isn't in this spawn's set"),
                Reach::Narrow => {
                    format!("{name} isn't in this spawn's set, and narrow only shrinks")
                }
            };
            return Err(Failure::NotGranted {
                program: name.to_owned(),
                cause,
                needed: None,
            });
        }
    }
    match &run.scope(root)?.programs {
        Programs::Unconfined => Ok(None),
        Programs::Granted(programs) => programs.get(name).map(Some).ok_or_else(|| {
            Failure::NotGranted {
                program: name.to_owned(),
                cause: format!("{name} isn't a program the run grants"),
                // a narrowed root's names were all granted when it was made
                needed: (!root.narrowed()).then(|| SpawnNeed::Program(name.to_owned())),
            }
        }),
    }
}

/// `value` as a program's name, as the Node module checks one: a string,
/// no NUL byte, well-formed Unicode, not empty, else a `TypeError`.
fn program_name(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<String, Failure> {
    let text = checked_text(scope, value, "a spawn program name")?;
    if text.is_empty() {
        return Err(Failure::Type("a spawn program name is empty".to_owned()));
    }
    Ok(text)
}

/// `value` as text, as the Node module's `check_text` checks it, `what`
/// naming it: a string, no NUL byte, well-formed Unicode, else a
/// `TypeError`.
fn checked_text(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    what: &str,
) -> Result<String, Failure> {
    let Ok(text) = v8::Local::<v8::String>::try_from(value) else {
        return Err(Failure::Type(format!("{what} must be a string")));
    };
    let units = units(scope, text);
    if units.contains(&0) {
        return Err(Failure::Type(format!("{what} holds a NUL byte")));
    }
    String::from_utf16(&units)
        .map_err(|_| Failure::Type(format!("{what} isn't well-formed Unicode")))
}

/// `value` as a variable's name, as the Node module's `check_env_name`
/// checks one, `what` naming it: a string, not empty, no NUL byte,
/// well-formed Unicode, and no `=`, else a `TypeError` that never echoes
/// the text, as what follows a `=` can be a value.
fn env_name(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    what: &str,
) -> Result<String, Failure> {
    let Ok(text) = v8::Local::<v8::String>::try_from(value) else {
        return Err(Failure::Type(format!("{what} must be a string")));
    };
    let units = units(scope, text);
    let message = if units.is_empty() {
        "is empty"
    } else if units.contains(&0) {
        "holds a NUL byte"
    } else {
        match String::from_utf16(&units) {
            Err(_) => "isn't well-formed Unicode",
            Ok(name) if name.contains('=') => "holds a `=`",
            Ok(name) => return Ok(name),
        }
    };
    Err(Failure::Type(format!("{what} {message}")))
}

/// The UTF-16 code units of `text`.
fn units(scope: &v8::PinScope<'_, '_>, text: v8::Local<'_, v8::String>) -> Vec<u16> {
    let mut units = vec![0; text.length()];
    text.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    units
}

/// `value` as an array, when it's one and no proxy.
fn array(value: v8::Local<'_, v8::Value>) -> Option<v8::Local<'_, v8::Array>> {
    v8::Local::<v8::Array>::try_from(value)
        .ok()
        .filter(|_| !value.is_proxy())
}

/// The element of `list` at `index`, its own data, or `undefined` for a
/// hole or an accessor, whose getter isn't run.
fn element<'s>(
    scope: &v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Array>,
    index: u32,
) -> Result<v8::Local<'s, v8::Value>, Failure> {
    let key = string(scope, &index.to_string()).ok_or(Failure::Thrown)?;
    let element = own_data(scope, list.into(), key.into()).map_err(|_| Failure::Thrown)?;
    Ok(element.unwrap_or_else(|| v8::undefined(scope).into()))
}

/// What the options are read without running code of a caller's: own data
/// only, so a getter or a proxy is a `TypeError`, naming the option.
fn plain(key: &str) -> Failure {
    Failure::Type(format!(
        "spawn options are plain data, read without running code: {key}"
    ))
}

/// The value of `object`'s own property `key`, as the Node module's
/// `read_own` reads it: `None` when it has none; a `TypeError` for an
/// accessor or a proxy, since a member runs no code of a caller's.
fn own_option<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, Failure> {
    if object.is_proxy() {
        return Err(plain(key));
    }
    let name_key = name(scope, key).ok_or(Failure::Thrown)?;
    let descriptor = object
        .get_own_property_descriptor(scope, name_key)
        .ok_or(Failure::Thrown)?;
    let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) else {
        return Ok(None);
    };
    // a descriptor V8 made is an ordinary object of its fields
    let value_key = name(scope, "value").ok_or(Failure::Thrown)?;
    if !descriptor
        .has_own_property(scope, value_key)
        .ok_or(Failure::Thrown)?
    {
        return Err(plain(key));
    }
    descriptor
        .get(scope, value_key.into())
        .map(Some)
        .ok_or(Failure::Thrown)
}

/// What a program's `pass_env` names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PassEnv {
    /// These names and globs, in order.
    Patterns(Vec<EnvPattern>),
    /// The whole environment, but the names `omit_env` lists.
    All(BTreeSet<String>),
}

/// A program's options, each read once.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProgramOptions {
    /// `pass_env`, with `omit_env`.
    pass: PassEnv,
    /// `env`'s values, in order, when it's given.
    values: Option<Vec<(String, String)>>,
}

/// `options`, a program's, read and checked once, before its name
/// resolves, as the Node module's `read_program_options` reads them.
fn read_program_options(
    scope: &v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Value>,
) -> Result<ProgramOptions, Failure> {
    let empty = ProgramOptions {
        pass: PassEnv::Patterns(Vec::new()),
        values: None,
    };
    if options.is_undefined() {
        return Ok(empty);
    }
    let Some(object) = v8::Local::<v8::Object>::try_from(options)
        .ok()
        .filter(|_| !options.is_function() && !options.is_array())
    else {
        return Err(Failure::Type(
            "spawn program options are an object".to_owned(),
        ));
    };
    let pass = read_pass_env(scope, own_option(scope, object, "pass_env")?)?;
    let pass = read_omit_env(scope, own_option(scope, object, "omit_env")?, pass)?;
    let values = read_env_values(scope, own_option(scope, object, "env")?)?;
    Ok(ProgramOptions { pass, values })
}

/// A `pass_env` option, read element by element.
fn read_pass_env(
    scope: &v8::PinScope<'_, '_>,
    raw: Option<v8::Local<'_, v8::Value>>,
) -> Result<PassEnv, Failure> {
    let Some(raw) = raw.filter(|raw| !raw.is_undefined()) else {
        return Ok(PassEnv::Patterns(Vec::new()));
    };
    if raw.is_string() && raw.to_rust_string_lossy(scope) == "all" {
        return Ok(PassEnv::All(BTreeSet::new()));
    }
    let list = array(raw)
        .ok_or_else(|| Failure::Type("`pass_env` is an array of names or 'all'".to_owned()))?;
    let mut patterns = Vec::new();
    for index in 0..list.length() {
        let element = element(scope, list, index)?;
        let text = env_name(scope, element, "a `pass_env` entry")?;
        patterns.push(EnvPattern::parse(&text).map_err(|fault| match fault {
            EnvPatternFault::GlobAlone => {
                Failure::Type("`pass_env` glob needs a prefix: use 'all'".to_owned())
            }
            EnvPatternFault::GlobNotLast => {
                Failure::Type(format!("a `pass_env` glob ends in its one `*`: {text}"))
            }
            // what `env_name` passed names a variable
            EnvPatternFault::Name(_) => {
                Failure::Type("a `pass_env` entry names no variable".to_owned())
            }
        })?);
    }
    Ok(PassEnv::Patterns(patterns))
}

/// An `omit_env` option, read element by element, given only beside
/// `pass_env: 'all'`.
fn read_omit_env(
    scope: &v8::PinScope<'_, '_>,
    raw: Option<v8::Local<'_, v8::Value>>,
    pass: PassEnv,
) -> Result<PassEnv, Failure> {
    let Some(raw) = raw.filter(|raw| !raw.is_undefined()) else {
        return Ok(pass);
    };
    if !matches!(pass, PassEnv::All(_)) {
        return Err(Failure::Type(
            "`omit_env` is only valid with `pass_env: 'all'`".to_owned(),
        ));
    }
    let list =
        array(raw).ok_or_else(|| Failure::Type("`omit_env` is an array of names".to_owned()))?;
    let mut omitted = BTreeSet::new();
    for index in 0..list.length() {
        let element = element(scope, list, index)?;
        let name = env_name(scope, element, "an `omit_env` entry")?;
        if name.contains('*') {
            return Err(Failure::Type(format!(
                "an `omit_env` entry is an exact name, not a glob: {name}"
            )));
        }
        omitted.insert(name);
    }
    Ok(PassEnv::All(omitted))
}

/// An `env` option, read key by key: its own enumerable names, in order,
/// each value its own data.
fn read_env_values(
    scope: &v8::PinScope<'_, '_>,
    raw: Option<v8::Local<'_, v8::Value>>,
) -> Result<Option<Vec<(String, String)>>, Failure> {
    let Some(raw) = raw.filter(|raw| !raw.is_undefined()) else {
        return Ok(None);
    };
    let Some(record) = v8::Local::<v8::Object>::try_from(raw)
        .ok()
        .filter(|_| !raw.is_function() && !raw.is_array())
    else {
        return Err(Failure::Type(
            "`env` is a record of names to values".to_owned(),
        ));
    };
    if record.is_proxy() {
        return Err(plain("env"));
    }
    let keys = record
        .get_own_property_names(
            scope,
            v8::GetPropertyNamesArgs {
                mode: v8::KeyCollectionMode::OwnOnly,
                property_filter: v8::PropertyFilter::ONLY_ENUMERABLE
                    | v8::PropertyFilter::SKIP_SYMBOLS,
                index_filter: v8::IndexFilter::IncludeIndices,
                key_conversion: v8::KeyConversionMode::ConvertToString,
            },
        )
        .ok_or(Failure::Thrown)?;
    let mut values = Vec::new();
    for index in 0..keys.length() {
        let key = keys.get_index(scope, index).ok_or(Failure::Thrown)?;
        let name = env_name(scope, key, "an `env` name")?;
        let key = v8::Local::<v8::Name>::try_from(key).map_err(|_| Failure::Thrown)?;
        let value = own_data(scope, record, key)
            .map_err(|_| Failure::Thrown)?
            .ok_or_else(|| plain("env"))?;
        let value = checked_text(scope, value, &format!("the `env` value of {name}"))?;
        values.push((name, value));
    }
    Ok(Some(values))
}

/// Refuses on a narrowed root what would make a named program arbitrary
/// execution (`env`) or hand its holder the whole environment (`pass_env:
/// 'all'` and globs, which enumerate it), as the Node module does:
/// `not_granted`, needing nothing.
fn check_narrowed_options(
    root: &Root<'_>,
    name: &str,
    options: &ProgramOptions,
) -> Result<(), Failure> {
    if !root.narrowed() {
        return Ok(());
    }
    let refused = |cause: String| Failure::NotGranted {
        program: name.to_owned(),
        cause,
        needed: None,
    };
    if options.values.is_some() {
        return Err(refused(format!(
            "a narrowed spawn can't set env for {name}: build the program at the root"
        )));
    }
    match &options.pass {
        PassEnv::All(_) => Err(refused(format!(
            "a narrowed spawn can't pass the whole environment to {name}"
        ))),
        PassEnv::Patterns(patterns) => patterns
            .iter()
            .find(|pattern| matches!(pattern, EnvPattern::Prefix(_)))
            .map_or(Ok(()), |glob| {
                Err(refused(format!(
                    "a narrowed spawn can't pass the glob {glob} to {name}: name each variable"
                )))
            }),
    }
}

/// Refuses what a confined scope's grant of `name`, the patterns it
/// forwards, `forward`, doesn't: `pass_env: 'all'`, which only an
/// unconfined run passes, and each name or glob none of its patterns
/// holds but an exact name of the base set, which every child is given,
/// `not_granted`, needing the variable, which `--allow-run-env=` grants
/// whichever root the call was made on: a narrowed root passes on only
/// exact names, each one its program's grant decides. A program found
/// beneath an exec root is granted what the exec root forwards.
fn check_forwarding(
    name: &str,
    options: &ProgramOptions,
    forward: &[EnvPattern],
) -> Result<(), Failure> {
    match &options.pass {
        PassEnv::All(_) => Err(Failure::NotGranted {
            program: name.to_owned(),
            cause: format!(
                "{name} can't be passed the whole environment: only a run granted everything \
                 passes it"
            ),
            needed: None,
        }),
        PassEnv::Patterns(patterns) => patterns
            .iter()
            .find(|pattern| {
                !in_base_set(pattern) && !forward.iter().any(|held| held.holds(pattern))
            })
            .map_or(Ok(()), |pattern| {
                Err(Failure::NotGranted {
                    program: name.to_owned(),
                    cause: format!(
                        "{name} isn't granted {pattern} to be passed: --allow-run-env names \
                         what a program may be passed"
                    ),
                    needed: Some(SpawnNeed::Forward(pattern.to_string())),
                })
            }),
    }
}

/// Whether `pattern` names a variable of the base set exactly, which
/// every child is given whatever `pass_env` names.
fn in_base_set(pattern: &EnvPattern) -> bool {
    matches!(pattern, EnvPattern::Exact(name) if CHILD_BASE_ENV.contains(&name.as_str()))
}

/// The environment a program's children are given, made of `env`, the
/// variables the embedder read for children, by `options`: the base set,
/// then what `pass_env` names, then `env`'s values, each name once, in the
/// order first set, a later value in an earlier one's place. `PATH` keeps
/// only its absolute entries.
fn program_env(env: &EnvVars, options: &ProgramOptions) -> Vec<(String, String)> {
    let mut made: Vec<(String, String)> = Vec::new();
    let mut set = |name: &str, value: &str| {
        let value = if name == "PATH" {
            value
                .split(':')
                .filter(|entry| entry.starts_with('/'))
                .collect::<Vec<_>>()
                .join(":")
        } else {
            value.to_owned()
        };
        match made.iter_mut().find(|(held, _)| held == name) {
            Some((_, held)) => *held = value,
            None => made.push((name.to_owned(), value)),
        }
    };
    match &options.pass {
        PassEnv::All(omitted) => {
            for (name, value) in env.iter() {
                if !omitted.contains(name) {
                    set(name, value);
                }
            }
        }
        PassEnv::Patterns(patterns) => {
            for name in CHILD_BASE_ENV {
                if name == "NO_COLOR" {
                    set("LC_ALL", "C");
                }
                if let Some(value) = env.get(name) {
                    set(name, value);
                }
            }
            for pattern in patterns {
                for (name, value) in env.iter() {
                    if pattern.covers(name) {
                        set(name, value);
                    }
                }
            }
        }
    }
    for (name, value) in options.values.iter().flatten() {
        set(name, value);
    }
    made
}

/// A program handle for `name`, its pin at `at` in `run`'s table, its
/// children's environment made by `options`.
fn make_program<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    run: &SpawnRun,
    at: usize,
    name: &str,
    options: &ProgramOptions,
) -> Result<v8::Local<'s, v8::Value>, Failure> {
    let prototype = native::prototype(state, scope, &SPAWN_PROGRAM)
        .ok_or_else(|| Failure::Plain(NO_SPAWN_STATE.to_owned()))?;
    let real = run.pins.borrow().get(at).map(|pin| pin.real.clone());
    let real = real.ok_or_else(|| Failure::Plain(NO_SPAWN_STATE.to_owned()))?;
    let mut entries = Vec::new();
    for (key, value) in program_env(&run.env, options) {
        entries.push((key, string(scope, &value).ok_or(Failure::Thrown)?));
    }
    let env = v8::Object::with_prototype_and_properties(scope, v8::null(scope).into(), &[], &[]);
    for (key, value) in entries {
        let key = string(scope, &key).ok_or(Failure::Thrown)?;
        env.create_data_property(scope, key.into(), value.into())
            .ok_or(Failure::Thrown)?;
    }
    env.set_integrity_level(scope, v8::IntegrityLevel::Frozen)
        .ok_or(Failure::Thrown)?;
    let at = u32::try_from(at).map_err(|_| Failure::Plain(NO_SPAWN_STATE.to_owned()))?;
    let pin: v8::Local<v8::Value> = v8::Integer::new_from_unsigned(scope, at).into();
    let held = frozen_record(scope, &[(PIN, pin), (ENV, env.into())]).ok_or(Failure::Thrown)?;
    let own = [
        string(scope, name).ok_or(Failure::Thrown)?.into(),
        string(scope, &real).ok_or(Failure::Thrown)?.into(),
    ];
    handle::make(scope, &SPAWN_PROGRAM, prototype, held.into(), &own)
        .map(Into::into)
        .ok_or(Failure::Thrown)
}

/// A scope as a test makes it: the programs it grants, each with the
/// patterns it forwards, and its exec roots.
#[cfg(test)]
pub type TestScope<'a> = (&'a [(&'a str, &'a [&'a str])], Vec<ExecRoot>);

/// A run whose table holds a confined scope for each of `scopes`, in
/// order, the project's first: each grants its programs, each found
/// nowhere, with the patterns it lists to forward, and its exec roots. No
/// run's start makes a second scope yet, so this is how a test holds a
/// method to the scope its root names.
#[cfg(test)]
pub fn run_of_scopes(scopes: Vec<TestScope<'_>>) -> SpawnRun {
    let scopes = scopes
        .into_iter()
        .map(|(programs, exec_roots)| Scope {
            programs: Programs::Granted(
                programs
                    .iter()
                    .map(|(name, forward)| {
                        let granted = GrantedProgram {
                            pin: Err(Fault::NotFound),
                            forward: forward
                                .iter()
                                .map(|pattern| EnvPattern::parse(pattern).unwrap())
                                .collect(),
                        };
                        ((*name).to_owned(), granted)
                    })
                    .collect(),
            ),
            exec_roots,
        })
        .collect();
    SpawnRun {
        project_dir: Err("a test's run has no project".to_owned()),
        children: Arc::default(),
        guarded: Cell::new(false),
        env: EnvVars::default(),
        project: Some("/".to_owned()),
        scopes,
        pins: RefCell::new(Vec::new()),
        started: Rc::default(),
    }
}

/// A root of `root`'s prototype naming the scope at `place` in the run's
/// table, its names `null`, which no run makes yet but for the project's
/// scope: see [`run_of_scopes`].
#[cfg(test)]
pub fn root_of_scope<'s>(
    scope: &v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    place: usize,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = handle::prototype_of(scope, root)?;
    make_root(scope, prototype, place, v8::null(scope).into())
}

#[cfg(test)]
mod tests;
