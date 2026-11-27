//! What a run refused by one of gro's rules, which fails the run whether
//! or not the program caught it: an `import()` the loader refused
//! ([`FailureReason::guards_a_boundary`](super::graph::FailureReason::guards_a_boundary)),
//! and a host function's call refused for a grant its handle doesn't hold (an
//! environment variable outside an env handle's names, an `fs` path outside
//! every root the call may use, an `fs` op its handle's scope lacks, or a
//! program, or a variable to pass one, outside a spawn handle's grant). One
//! record holds both, so both are listed, cut, and counted alike. A refused
//! call that a grant of the run would have allowed also records that grant
//! ([`NeededGrant`]), as data, never parsed from a message, so a report can
//! say what to grant. What a program can make it keep outside V8's heap is
//! bounded: a few messages, each cut at a fixed length, a few grants, and a
//! count.

use std::borrow::Cow;

use crate::host::error_class::{ENV_ERROR, FS_ERROR, SPAWN_ERROR};
use crate::resolver::Capability;

/// The most refusals listed.
///
/// A program meets one by importing what a rule refuses, a handful of times
/// when a library tries each host module it could use, and its reader fixes
/// the first and runs again; past these it's a loop, which the count stands
/// for.
pub const MAX_LISTED_REFUSALS: usize = 8;

/// The bytes of a refusal's message a listing keeps.
///
/// A message names the specifier, the importer, and the reason with its
/// fix, in a few hundred bytes unless the program made the specifier long,
/// as a `data:` URL is, and then its head says what it is.
pub const MAX_LISTED_MESSAGE: usize = 2048;

/// What refused something by one of gro's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusedBy {
    /// The loader refused an `import()`, which rejected with a `TypeError`.
    Import,
    /// `env` refused a call for a variable outside the names of the handle
    /// it was called on, throwing `EnvError` `not_granted`. The grant that
    /// would have allowed it, when one would, is the variable, which the
    /// message names, so the listing keeps no need.
    Env,
    /// `fs` refused a call for a path outside every root the call may use,
    /// or an op its handle's scope lacks, throwing `FsError` `not_granted`,
    /// with the kind of fs grant that would have allowed it, `None` when
    /// none would.
    Fs(Option<FsNeed>),
    /// `spawn` refused a call for a program outside the set of the handle
    /// it was called on, a variable its program's grant doesn't forward,
    /// or what no grant of the run allows, throwing `SpawnError`
    /// `not_granted` naming the program. The grant that would have allowed
    /// it, when one would, is kept apart ([`NeededGrant`]).
    Spawn,
}

impl RefusedBy {
    /// The name of the class of the error JS saw the refusal as.
    pub const fn error(self) -> &'static str {
        match self {
            Self::Import => "TypeError",
            Self::Env => ENV_ERROR.name,
            Self::Fs(_) => FS_ERROR.name,
            Self::Spawn => SPAWN_ERROR.name,
        }
    }
}

/// The kind of the error a host module's call throws when it's refused for
/// a grant the handle it was called on doesn't hold: its message is
/// `not_granted: target`, as the family's class makes it.
pub const NOT_GRANTED: &str = "not_granted";

/// The kind of the error an `fs` call throws for a path that leaves the
/// handle it was called on, by its text or through a symlink.
const OUTSIDE_ROOT: &str = "outside_root";

/// A host module's call refused for a grant the handle it was called on
/// doesn't hold, as [`Refusals::record_not_granted`] takes it, with what
/// would have allowed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotGranted {
    /// `env` refused the variable the target names; `needed` when
    /// granting it would have let the call through, as for a refusal by
    /// the run's root handle.
    Env {
        /// Whether a grant of the variable would have allowed the call.
        needed: bool,
    },
    /// `fs` refused the path the target names, with the kind of fs grant
    /// that would have allowed it, `None` when none would.
    Fs(Option<FsNeed>),
    /// `spawn` refused a call about the program the target names, with
    /// what would have allowed it, `None` when nothing the run can be
    /// granted would.
    Spawn(Option<SpawnNeed>),
}

/// What would have let a refused `spawn` call through: a grant of the
/// run's, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnNeed {
    /// The program, by the name it was asked for ([`NeededGrant::Run`]).
    Program(String),
    /// A variable forwarded to a granted program, by its name, or the glob
    /// a program's `pass_env` gave ([`NeededGrant::RunEnv`]).
    Forward(String),
    /// An exec root holding the directory a program was asked for beneath
    /// ([`NeededGrant::RunIn`]), by kind alone, as an fs grant's directory.
    ExecRoot,
}

impl From<SpawnNeed> for NeededGrant {
    fn from(need: SpawnNeed) -> Self {
        match need {
            SpawnNeed::Program(name) => Self::Run(name),
            SpawnNeed::Forward(name) => Self::RunEnv(name),
            SpawnNeed::ExecRoot => Self::RunIn,
        }
    }
}

/// What would have allowed a refused call, so a report can say what to
/// grant.
///
/// An environment variable, by the name an embedder's flag lists; a kind
/// of fs grant, by kind alone, since a path a program chose, shown back,
/// can name another directory than the one the refusal needed; a handle
/// on a directory the run already grants; or a symlink kept within its
/// root, or its target opened by its own path.
///
/// Recorded only where the refusal is the run's own grant's, so granting
/// it lets the call through: a call refused by `env`'s root handle, which
/// holds what `--allow-env=` granted the project; a program refused by
/// `spawn`'s, which holds the programs `--allow-run=` granted; a variable
/// to pass that a granted program's grant doesn't forward, which
/// `--allow-run-env=` widens whichever root the call was made on; a
/// directory no exec root of the scope the root names holds, which
/// `--allow-run-in=` grants; an fs
/// reach no root holds; one that left every root holding it through a
/// symlink, which the link or the path the program opens lets through,
/// where a grant of the target's directory needn't; and an op a handle's
/// scope lacks, a grant of the kind it lacks, unless a root the op's
/// target is in already grants it, since a handle's scope never widens;
/// and a builtin module the project's own module imported that the run's
/// grants lack the row of, which an embedder's flag grants. Never a name
/// refused by a handle the program narrowed, which no grant widens, and
/// never a builtin refused to a package or to Gro's library, which no grant
/// of the project's lets through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeededGrant {
    /// A builtin module's row, by the capability it needs: the run's
    /// grants lack it, and the project's own module imported the builtin.
    Module(Capability),
    /// An environment variable to read, by its name.
    Env(String),
    /// An fs grant, by its kind.
    Fs(FsNeed),
    /// A program to run, by the name it was asked for.
    Run(String),
    /// A variable to forward to a granted program, by its name, or the
    /// glob a program's `pass_env` gave.
    RunEnv(String),
    /// An exec root holding the directory a program was asked for
    /// beneath, by kind alone, as an fs grant's directory is.
    RunIn,
}

impl NeededGrant {
    /// The kind of name it is, when it's one.
    const fn name_kind(&self) -> Option<NameKind> {
        match self {
            Self::Env(_) => Some(NameKind::Env),
            Self::Run(_) => Some(NameKind::Run),
            Self::RunEnv(_) => Some(NameKind::RunEnv),
            Self::Module(_) | Self::Fs(_) | Self::RunIn => None,
        }
    }
}

/// A kind of grant needed by name, each bounded on its own in a record
/// ([`Refusals::needed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    /// An environment variable to read ([`NeededGrant::Env`]).
    Env,
    /// A program to run ([`NeededGrant::Run`]).
    Run,
    /// A variable to forward to a program ([`NeededGrant::RunEnv`]).
    RunEnv,
}

/// The kind of fs grant a refused call needed, which a listed fs refusal
/// names, so a hint of a kind pairs with the listed refusals of that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsNeed {
    /// A directory to read.
    Read,
    /// A symlink kept within its root, or its target opened by its own
    /// path: a reach left every root holding it by its text through a
    /// symlink whose target leaves the root by its text, an absolute one or
    /// one climbing above the root by `..`, which a grant of the target's
    /// directory needn't let through, since a root holds a path by its
    /// text.
    LinkTarget,
    /// A directory to write, through a handle `fs.open_dir` makes on it
    /// once it's granted: a handle made from another root never gains the
    /// scope.
    Write,
    /// A directory a root already reads or writes as the refused op
    /// needed: a handle `fs.open_dir` makes on it does the op, where the
    /// handle that refused it, made from another root's, never gains the
    /// scope.
    OpenDir,
    /// A fresh temporary directory, which `fs.temp_dir` makes under the
    /// OS's temporary directory once it's granted.
    Temp,
    /// A read in the project's root where it's the entry's own directory,
    /// with no project's `package.json` at or above it, which a run reads
    /// only by a flag naming it: the embedder, which resolved that root,
    /// names it in its hint, never a path the program chose.
    EntryDirRead,
    /// A write there, as [`FsNeed::EntryDirRead`] is for a read.
    EntryDirWrite,
}

/// One refusal, as a run's record lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// What refused it, with the kind of fs grant an `fs` refusal needed.
    pub by: RefusedBy,
    /// The message of the error JS saw, cut past [`MAX_LISTED_MESSAGE`]
    /// bytes and marked there with its whole length: `cannot resolve
    /// 'data:… (4096 bytes)`. A refused op's names its target and never a
    /// value: `not_granted: HOME`.
    pub message: String,
}

/// What one of gro's rules refused during a run: the `import()`s, a
/// refused import in a module an `import()` loaded among them, and the host
/// ops' calls refused for a grant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refusals {
    /// The first [`MAX_LISTED_REFUSALS`] distinct refusals, in the order
    /// each first happened. Two refusals are one when what refused them is
    /// the same, an `fs` refusal's need included, and their messages, cut,
    /// are the same text, as a retry's are.
    pub listed: Vec<Refusal>,
    /// How many more refusals there were than `listed` lists: each that
    /// isn't listed, a retry of one included.
    pub unlisted: u64,
    /// The grants that would have allowed the refused calls that one
    /// would have, distinct, in the order each was first needed: every fs
    /// kind, and, of each kind of name ([`NameKind`]), the first
    /// [`MAX_LISTED_REFUSALS`], each at most [`MAX_LISTED_MESSAGE`] bytes
    /// long. Kept apart from `listed`, so the refusal that is the run's
    /// uncaught error, which isn't listed, still has its grant here.
    pub needed: Vec<NeededGrant>,
    /// Each kind of name a refused call needed one of that `needed`
    /// doesn't name, past its bound or longer than a name it keeps, once.
    pub more_names: Vec<NameKind>,
    /// The messages, cut, of the first [`MAX_LISTED_REFUSALS`] distinct
    /// `fs` errors `outside_root` about a path a root of the run already
    /// grants the call, which a handle `fs.open_dir` makes there does. Not
    /// refusals: such an error fails nothing caught, so it's only told
    /// when it's the run's uncaught error.
    pub outside_granted: Vec<String>,
}

impl Refusals {
    /// Whether nothing was refused.
    pub const fn is_empty(&self) -> bool {
        self.listed.is_empty() && self.unlisted == 0
    }

    /// The listed refusals' messages, in order.
    #[cfg(test)]
    pub fn messages(&self) -> Vec<&str> {
        self.listed
            .iter()
            .map(|refusal| refusal.message.as_str())
            .collect()
    }

    /// Records an `import()` the loader refused, its `TypeError` carrying
    /// `message` ([`Self::list`]).
    pub(crate) fn record_import(&mut self, message: &str) {
        self.list(RefusedBy::Import, message);
    }

    /// Records that the project's own module imported a builtin needing
    /// `capability`, which the run's grants lack, whether the import was
    /// static or an `import()`; the static one isn't listed, since it's the
    /// run's failure.
    pub(crate) fn need_module(&mut self, capability: Capability) {
        self.need(NeededGrant::Module(capability));
    }

    /// Records an `fs` call's `outside_root` about `target`, a path a
    /// root of the run already grants the call, its error's message
    /// `outside_root: target`, while there's room.
    pub(crate) fn record_outside_granted(&mut self, target: &str) {
        let message = cut(&format!("{OUTSIDE_ROOT}: {target}")).into_owned();
        if self.outside_granted.len() < MAX_LISTED_REFUSALS
            && !self.outside_granted.contains(&message)
        {
            self.outside_granted.push(message);
        }
    }

    /// Keeps the grant the run's uncaught error needed when the error isn't
    /// a refusal: an `FsError` `outside_root` about a path a root already
    /// grants the call ([`Self::record_outside_granted`]), a handle opened
    /// there with `fs.open_dir` ([`FsNeed::OpenDir`]), the error being of
    /// the class named `error` and carrying `message`.
    pub(crate) fn need_for_uncaught(&mut self, error: &str, message: &str) {
        if error == FS_ERROR.name
            && self
                .outside_granted
                .iter()
                .any(|kept| *kept == cut(message))
        {
            self.need(NeededGrant::Fs(FsNeed::OpenDir));
        }
    }

    /// Records a host module's call refused about `target`, its error's
    /// message `not_granted: target`, the text the family's class gives the
    /// error it throws, which [`Self::unlist`] finds the run's uncaught
    /// error by; and the grant that would have allowed it, if one would
    /// have, kept unless it's kept already, while there's room.
    pub(crate) fn record_not_granted(&mut self, refused: NotGranted, target: &str) {
        let by = match refused {
            NotGranted::Spawn(needed) => {
                if let Some(needed) = needed {
                    self.need(needed.into());
                }
                RefusedBy::Spawn
            }
            NotGranted::Env { needed } => {
                if needed {
                    self.need(NeededGrant::Env(target.to_owned()));
                }
                RefusedBy::Env
            }
            NotGranted::Fs(need) => {
                if let Some(need) = need {
                    self.need(NeededGrant::Fs(need));
                }
                RefusedBy::Fs(need)
            }
        };
        self.list(by, &format!("{NOT_GRANTED}: {target}"));
    }

    /// Lists a refusal by `by` whose error carries `message`, unless it's
    /// listed already, while there's room, and counts it after.
    fn list(&mut self, by: RefusedBy, message: &str) {
        let message = cut(message);
        if self
            .listed
            .iter()
            .any(|listed| listed.by == by && *listed.message == message)
        {
            return;
        }
        if self.listed.len() == MAX_LISTED_REFUSALS {
            self.unlisted += 1;
        } else {
            self.listed.push(Refusal {
                by,
                message: message.into_owned(),
            });
        }
    }

    /// Keeps `needed` among the grants the refused calls needed, unless
    /// it's kept already: an fs kind always, there being few, and a name
    /// while there's room for its kind's and it's short enough to keep,
    /// else marks that there are more of its kind.
    fn need(&mut self, needed: NeededGrant) {
        if self.needed.contains(&needed) {
            return;
        }
        if let (
            Some(kind),
            NeededGrant::Env(name) | NeededGrant::Run(name) | NeededGrant::RunEnv(name),
        ) = (needed.name_kind(), &needed)
        {
            let names = self
                .needed
                .iter()
                .filter(|kept| kept.name_kind() == Some(kind))
                .count();
            if names == MAX_LISTED_REFUSALS || name.len() > MAX_LISTED_MESSAGE {
                if !self.more_names.contains(&kind) {
                    self.more_names.push(kind);
                }
                return;
            }
        }
        self.needed.push(needed);
    }

    /// Drops the listing of the refusal that is the run's uncaught error,
    /// which its report shows: the one listed refusal whose error is of the
    /// class named `error` and carries `message`. The error carries no
    /// need, so `fs`'s refusals of one path that needed different grants
    /// carry the same class and message; when more than one is listed,
    /// each stays, the uncaught one beside the others, each naming its
    /// need, since which one the report shows is out of reach. When any
    /// refusal was counted, not listed, the uncaught one may be among
    /// those, so every listed one stays.
    pub(crate) fn unlist(&mut self, error: &str, message: &str) {
        if self.unlisted > 0 {
            return;
        }
        let message = cut(message);
        let mut matching = self
            .listed
            .iter()
            .enumerate()
            .filter(|(_, listed)| listed.by.error() == error && *listed.message == message);
        if let (Some((index, _)), None) = (matching.next(), matching.next()) {
            self.listed.remove(index);
        }
    }
}

/// `message`, when it's longer than [`MAX_LISTED_MESSAGE`] bytes, cut to at
/// most that many at a character boundary and marked with its whole length.
fn cut(message: &str) -> Cow<'_, str> {
    if message.len() <= MAX_LISTED_MESSAGE {
        return Cow::Borrowed(message);
    }
    let head = &message[..message.floor_char_boundary(MAX_LISTED_MESSAGE)];
    Cow::Owned(format!("{head}… ({} bytes)", message.len()))
}

#[cfg(test)]
mod tests {
    use super::{
        FsNeed, MAX_LISTED_MESSAGE, MAX_LISTED_REFUSALS, NameKind, NeededGrant, NotGranted,
        Refusals, RefusedBy, SpawnNeed,
    };

    /// A name a narrowed env handle refused, which no grant would allow.
    const NARROWED: NotGranted = NotGranted::Env { needed: false };

    /// A name env's root handle refused, which a grant of it would allow.
    const ROOT: NotGranted = NotGranted::Env { needed: true };

    /// An fs refusal that needed `need`.
    const fn fs(need: Option<FsNeed>) -> NotGranted {
        NotGranted::Fs(need)
    }

    /// The first refusals are listed in order, a retry of a listed one
    /// changes nothing, and each one past the listed, a retry included, is
    /// counted.
    #[test]
    fn the_first_distinct_refusals_are_listed_and_the_rest_counted() {
        let mut refused = Refusals::default();
        assert!(refused.is_empty());
        let messages: Vec<String> = (0..MAX_LISTED_REFUSALS + 2)
            .map(|i| format!("cannot resolve 'ext:{i}'"))
            .collect();
        for message in &messages[..MAX_LISTED_REFUSALS] {
            refused.record_import(message);
            refused.record_import(message);
        }
        assert!(!refused.is_empty());
        assert_eq!(refused.messages(), messages[..MAX_LISTED_REFUSALS]);
        assert_eq!(refused.unlisted, 0);
        for message in &messages[MAX_LISTED_REFUSALS..] {
            refused.record_import(message);
            refused.record_import(message);
        }
        // a listed one retried once the list is full still isn't counted
        refused.record_import(&messages[0]);
        assert_eq!(refused.messages(), messages[..MAX_LISTED_REFUSALS]);
        assert_eq!(refused.unlisted, 4);
    }

    /// A message at the bound is listed whole, and a longer one cut at a
    /// character boundary at or below it, marked with its whole length;
    /// two with one head and one length are one listing, and a longer one
    /// with that head is another.
    #[test]
    fn a_long_message_is_cut_and_marked_with_its_length() {
        let mut refused = Refusals::default();
        let whole = "a".repeat(MAX_LISTED_MESSAGE);
        refused.record_import(&whole);
        // 'é' is two bytes, so a cut at the bound falls inside the last one
        refused.record_import(&format!("b{}", "é".repeat(MAX_LISTED_MESSAGE / 2)));
        let head = "c".repeat(MAX_LISTED_MESSAGE);
        refused.record_import(&format!("{head}x"));
        refused.record_import(&format!("{head}y"));
        refused.record_import(&format!("{head}xy"));
        assert_eq!(
            refused.messages(),
            [
                whole,
                format!(
                    "b{}… ({} bytes)",
                    "é".repeat(MAX_LISTED_MESSAGE / 2 - 1),
                    MAX_LISTED_MESSAGE + 1
                ),
                format!("{head}… ({} bytes)", MAX_LISTED_MESSAGE + 1),
                format!("{head}… ({} bytes)", MAX_LISTED_MESSAGE + 2),
            ]
        );
        assert_eq!(refused.unlisted, 0);
    }

    /// Unlisting drops the one listing with the message, a long one's by
    /// its cut text, and only by its whole text.
    #[test]
    fn a_refusal_is_unlisted_by_its_message() {
        let mut refused = Refusals::default();
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record_import(&format!("refused {i}"));
        }
        refused.unlist("TypeError", "refused 1");
        refused.unlist("TypeError", "refused");
        assert_eq!(refused.messages().len(), MAX_LISTED_REFUSALS - 1);
        assert!(!refused.messages().contains(&"refused 1"));

        let mut refused = Refusals::default();
        let long = "c".repeat(MAX_LISTED_MESSAGE + 1);
        refused.record_import(&long);
        refused.unlist("TypeError", &long);
        assert!(refused.is_empty());

        // with none listed, the counted ones are still refusals
        let refused = Refusals {
            unlisted: 1,
            ..Refusals::default()
        };
        assert!(!refused.is_empty());
    }

    /// Once a refusal was counted, not listed, unlisting drops nothing,
    /// since the uncaught one may be the counted one, and a listed refusal
    /// of its class and message another the program caught.
    #[test]
    fn nothing_is_unlisted_once_a_refusal_was_counted() {
        let mut refused = Refusals::default();
        for i in 0..MAX_LISTED_REFUSALS - 1 {
            refused.record_import(&format!("refused {i}"));
        }
        refused.record_not_granted(fs(Some(FsNeed::OpenDir)), "/a");
        refused.record_not_granted(fs(None), "/a");
        assert_eq!(refused.unlisted, 1);
        refused.unlist("FsError", "not_granted: /a");
        refused.unlist("TypeError", "refused 0");
        assert_eq!(refused.listed.len(), MAX_LISTED_REFUSALS);
        assert_eq!(
            refused.listed[MAX_LISTED_REFUSALS - 1].by,
            RefusedBy::Fs(Some(FsNeed::OpenDir))
        );
    }

    /// An import's refusal and an op's are two listings even when their
    /// messages are the same text, each counted in the one bound, and each
    /// unlisted only as its own class's error.
    #[test]
    fn an_import_s_refusal_and_an_op_s_share_the_record_apart() {
        let mut refused = Refusals::default();
        refused.record_not_granted(NARROWED, "HOME");
        refused.record_import("not_granted: HOME");
        refused.record_not_granted(NARROWED, "HOME");
        assert_eq!(
            refused
                .listed
                .iter()
                .map(|listed| listed.by)
                .collect::<Vec<_>>(),
            [RefusedBy::Env, RefusedBy::Import]
        );
        refused.unlist("TypeError", "not_granted: HOME");
        assert_eq!(
            refused
                .listed
                .iter()
                .map(|listed| listed.by)
                .collect::<Vec<_>>(),
            [RefusedBy::Env]
        );
        refused.unlist("EnvError", "not_granted: HOME");
        assert!(refused.is_empty());
        refused.record_not_granted(NARROWED, "HOME");
        refused.record_import("not_granted: HOME");
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record_not_granted(NARROWED, &format!("N{i}"));
        }
        assert_eq!(refused.listed.len(), MAX_LISTED_REFUSALS);
        assert_eq!(refused.unlisted, 2);
    }

    /// Each refusal's error is of its family's class, an fs one's whatever
    /// it needed, and an import's a `TypeError`.
    #[test]
    fn a_refusal_s_error_is_its_family_s_class() {
        assert_eq!(RefusedBy::Import.error(), "TypeError");
        assert_eq!(RefusedBy::Env.error(), "EnvError");
        for need in [
            None,
            Some(FsNeed::Read),
            Some(FsNeed::LinkTarget),
            Some(FsNeed::Write),
            Some(FsNeed::OpenDir),
            Some(FsNeed::Temp),
        ] {
            assert_eq!(RefusedBy::Fs(need).error(), "FsError");
        }
    }

    /// A host module's refusal is listed with the message its error
    /// carries, `not_granted: target`, by which the run's uncaught one is
    /// unlisted.
    #[test]
    fn a_refused_call_s_message_is_not_granted_and_its_target() {
        let mut refused = Refusals::default();
        refused.record_not_granted(ROOT, "HOME");
        refused.record_not_granted(fs(Some(FsNeed::Read)), "/a b");
        assert_eq!(
            refused.messages(),
            ["not_granted: HOME", "not_granted: /a b"]
        );
        refused.unlist("EnvError", "not_granted: HOME");
        refused.unlist("FsError", "not_granted: /a b");
        assert!(refused.is_empty());
    }

    /// An fs refusal is listed with the kind of grant it needed, and two of
    /// one path are one listing only when they needed the same kind, or
    /// both none; an env refusal keeps no kind, whatever it needed.
    #[test]
    fn an_fs_refusal_is_listed_with_the_kind_it_needed() {
        let mut refused = Refusals::default();
        let (read, write) = (fs(Some(FsNeed::Read)), fs(Some(FsNeed::Write)));
        refused.record_not_granted(read.clone(), "/a");
        refused.record_not_granted(write.clone(), "/a");
        refused.record_not_granted(read, "/a");
        refused.record_not_granted(fs(None), "/a");
        refused.record_not_granted(fs(Some(FsNeed::OpenDir)), "/a");
        refused.record_not_granted(write, "/a");
        refused.record_not_granted(fs(None), "/a");
        refused.record_not_granted(ROOT, "A");
        refused.record_not_granted(NARROWED, "A");
        assert_eq!(
            refused
                .listed
                .iter()
                .map(|listed| listed.by)
                .collect::<Vec<_>>(),
            [
                RefusedBy::Fs(Some(FsNeed::Read)),
                RefusedBy::Fs(Some(FsNeed::Write)),
                RefusedBy::Fs(None),
                RefusedBy::Fs(Some(FsNeed::OpenDir)),
                RefusedBy::Env,
            ]
        );
        assert_eq!(refused.unlisted, 0);
    }

    /// An uncaught `FsError` whose message more than one listed refusal
    /// carries, of one path with different needs, unlists none of them,
    /// and one whose message only one carries unlists that one.
    #[test]
    fn an_fs_refusal_shared_by_needs_stays_listed() {
        let mut refused = Refusals::default();
        refused.record_not_granted(fs(Some(FsNeed::OpenDir)), "/a");
        refused.record_not_granted(fs(None), "/a");
        refused.record_not_granted(fs(Some(FsNeed::Read)), "/b");
        refused.unlist("FsError", "not_granted: /a");
        assert_eq!(
            refused
                .listed
                .iter()
                .map(|listed| (listed.message.as_str(), listed.by))
                .collect::<Vec<_>>(),
            [
                ("not_granted: /a", RefusedBy::Fs(Some(FsNeed::OpenDir))),
                ("not_granted: /a", RefusedBy::Fs(None)),
                ("not_granted: /b", RefusedBy::Fs(Some(FsNeed::Read))),
            ]
        );
        refused.unlist("FsError", "not_granted: /b");
        assert_eq!(refused.messages(), ["not_granted: /a", "not_granted: /a"]);
    }

    /// The grant a refused call needed, by its name for `env`.
    fn needs(value: &str) -> NeededGrant {
        NeededGrant::Env(value.to_owned())
    }

    /// The grants refused calls needed are kept distinct, in the order each
    /// was first needed, a retry's and an unlisted refusal's included, the
    /// variables up to the bound, each name at most the message bound long,
    /// and one past either marks that there are more; the fs kinds always,
    /// past the variables' bound too; a refusal needing none adds none,
    /// and unlisting a refusal keeps its grant.
    #[test]
    fn the_grants_refused_calls_needed_are_kept_apart_and_bounded() {
        let mut refused = Refusals::default();
        refused.record_import("cannot import");
        refused.record_not_granted(NARROWED, "NARROWED");
        refused.record_not_granted(fs(None), "/n");
        assert!(refused.needed.is_empty());
        assert!(refused.more_names.is_empty());
        refused.record_not_granted(ROOT, "B");
        refused.record_not_granted(ROOT, "A");
        refused.record_not_granted(ROOT, "B");
        refused.unlist("EnvError", "not_granted: A");
        assert_eq!(refused.needed, [needs("B"), needs("A")]);
        assert!(refused.more_names.is_empty());
        // one exactly the bound long is kept, one longer marks more
        let longest = "L".repeat(MAX_LISTED_MESSAGE);
        refused.record_not_granted(ROOT, &longest);
        assert!(refused.more_names.is_empty());
        refused.record_not_granted(ROOT, &format!("{longest}M"));
        assert_eq!(refused.more_names, [NameKind::Env]);
        assert_eq!(refused.needed.len(), 3);
        let mut refused = Refusals::default();
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record_not_granted(ROOT, &format!("N{i}"));
        }
        assert!(refused.more_names.is_empty());
        // a kept one again past the bound is no more
        refused.record_not_granted(ROOT, "N0");
        assert!(refused.more_names.is_empty());
        refused.record_not_granted(ROOT, "X");
        assert_eq!(refused.more_names, [NameKind::Env]);
        assert_eq!(refused.needed.len(), MAX_LISTED_REFUSALS);
        assert_eq!(refused.needed[0], needs("N0"));
        // the fs kinds past the variables' bound, each once
        for (path, need) in [
            ("/a", FsNeed::Write),
            ("/b", FsNeed::Read),
            ("/c", FsNeed::Write),
            ("/d", FsNeed::OpenDir),
            ("/e", FsNeed::Read),
        ] {
            refused.record_not_granted(fs(Some(need)), path);
        }
        assert_eq!(
            refused.needed[MAX_LISTED_REFUSALS..],
            [
                NeededGrant::Fs(FsNeed::Write),
                NeededGrant::Fs(FsNeed::Read),
                NeededGrant::Fs(FsNeed::OpenDir)
            ]
        );
        // and none takes a variable's room
        let mut refused = Refusals::default();
        refused.record_not_granted(fs(Some(FsNeed::Read)), "/a");
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record_not_granted(ROOT, &format!("N{i}"));
        }
        assert!(refused.more_names.is_empty());
        assert_eq!(refused.needed.len(), MAX_LISTED_REFUSALS + 1);
    }
    /// A spawn refusal is listed by the program it names, the grant it
    /// needed kept apart, a program's or a variable's, each kind of name
    /// bounded on its own, so neither takes the other's room or an env
    /// name's, and one past its bound marks that kind alone.
    #[test]
    fn a_spawn_refusal_keeps_its_need_apart_and_bounded_by_kind() {
        let mut refused = Refusals::default();
        let run = |name: &str| NotGranted::Spawn(Some(SpawnNeed::Program(name.to_owned())));
        let run_env = |name: &str| NotGranted::Spawn(Some(SpawnNeed::Forward(name.to_owned())));
        refused.record_not_granted(run("git"), "git");
        refused.record_not_granted(run_env("CARGO_HOME"), "cargo");
        refused.record_not_granted(NotGranted::Spawn(None), "tool");
        assert_eq!(
            refused.messages(),
            [
                "not_granted: git",
                "not_granted: cargo",
                "not_granted: tool"
            ]
        );
        assert!(
            refused
                .listed
                .iter()
                .all(|listed| listed.by == RefusedBy::Spawn)
        );
        assert_eq!(RefusedBy::Spawn.error(), "SpawnError");
        assert_eq!(
            refused.needed,
            [
                NeededGrant::Run("git".to_owned()),
                NeededGrant::RunEnv("CARGO_HOME".to_owned())
            ]
        );
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record_not_granted(run(&format!("p{i}")), "p");
            refused.record_not_granted(ROOT, &format!("V{i}"));
        }
        assert_eq!(refused.more_names, [NameKind::Run]);
        refused.record_not_granted(run_env("X"), "x");
        assert_eq!(refused.more_names, [NameKind::Run]);
        refused.record_not_granted(ROOT, "W");
        assert_eq!(refused.more_names, [NameKind::Run, NameKind::Env]);
        let kept =
            |kind: fn(&NeededGrant) -> bool| refused.needed.iter().filter(|n| kind(n)).count();
        assert_eq!(
            kept(|n| matches!(n, NeededGrant::Run(_))),
            MAX_LISTED_REFUSALS
        );
        assert_eq!(kept(|n| matches!(n, NeededGrant::RunEnv(_))), 2);
        assert_eq!(
            kept(|n| matches!(n, NeededGrant::Env(_))),
            MAX_LISTED_REFUSALS
        );
    }

    /// An `outside_root` a root grants needs a handle opened there only
    /// when it's the run's uncaught `FsError`, by its message; it's never
    /// listed, and the record keeps a few.
    #[test]
    fn an_outside_root_a_root_grants_is_told_only_when_uncaught() {
        let mut refused = Refusals::default();
        refused.record_outside_granted("../a");
        refused.record_outside_granted("../a");
        assert!(refused.is_empty());
        assert_eq!(refused.outside_granted, ["outside_root: ../a"]);
        refused.need_for_uncaught("TypeError", "outside_root: ../a");
        refused.need_for_uncaught("FsError", "outside_root: ../b");
        refused.need_for_uncaught("FsError", "not_granted: ../a");
        assert_eq!(refused.needed, []);
        refused.need_for_uncaught("FsError", "outside_root: ../a");
        assert_eq!(refused.needed, [NeededGrant::Fs(FsNeed::OpenDir)]);
        for i in 0..MAX_LISTED_REFUSALS * 2 {
            refused.record_outside_granted(&format!("../{i}"));
        }
        assert_eq!(refused.outside_granted.len(), MAX_LISTED_REFUSALS);
    }
}
