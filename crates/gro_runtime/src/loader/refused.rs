//! What a run refused by one of gro's rules, which fails the run whether
//! or not the program caught it: an `import()` the loader refused
//! ([`FailureReason::guards_a_boundary`](super::graph::FailureReason::guards_a_boundary)),
//! and a host function's call refused for a grant its handle doesn't hold (an
//! environment variable outside an env handle's names, or an `fs` path no
//! root holds). One record holds both, so both are listed, cut, and counted
//! alike. A refused call that a
//! grant of the run would have allowed also records that grant
//! ([`NeededGrant`]), as data, never parsed from a message, so a report can
//! say what to grant. What a program can make it keep outside V8's heap is
//! bounded: a few messages, each cut at a fixed length, a few grants, and a
//! count.

use std::borrow::Cow;

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
    /// A host function of the module `grant` names refused a call for a target
    /// outside the handle it was called on, throwing `not_granted` of the
    /// error class named `error` (`EnvError`).
    Grant {
        /// The host module's grant, which names it: `env`.
        grant: Capability,
        /// The name of the class of the error the call threw.
        error: &'static str,
    },
}

impl RefusedBy {
    /// The name of the class of the error JS saw the refusal as.
    pub const fn error(self) -> &'static str {
        match self {
            Self::Import => "TypeError",
            Self::Grant { error, .. } => error,
        }
    }
}

/// What would have allowed a refused call, so a report can say what to
/// grant.
///
/// An environment variable, by the name an embedder's flag lists; a kind
/// of fs grant, by kind alone, since a path a program chose, shown back,
/// can name another directory than the one the refusal needed; or a handle
/// on a directory the run already grants.
///
/// Recorded only where the refusal is the run's own grant's, so granting
/// it lets the call through: a call refused by `env`'s root handle, which
/// holds what `--allow-env=` granted the project, and never one refused by
/// a handle the program narrowed, which no grant widens; an fs reach no
/// root holds, a step out of its root through a symlink among them; and
/// an op a handle's scope lacks, a grant of the kind it lacks, unless a
/// root the op's target is in already grants it, since a handle's scope
/// never widens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeededGrant {
    /// An environment variable to read, by its name.
    Env(String),
    /// A directory to read.
    Read,
    /// A directory to write, through a handle `fs.open_dir` makes on it
    /// once it's granted: a handle made from another root never gains the
    /// scope.
    Write,
    /// A directory a root already reads or writes as the refused op
    /// needed: a handle `fs.open_dir` makes on it does the op, where the
    /// handle that refused it, made from another root's, never gains the
    /// scope.
    OpenDir,
}

/// One refusal, as a run's record lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// What refused it.
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
    /// the same and their messages, cut, are the same text, as a retry's
    /// are.
    pub listed: Vec<Refusal>,
    /// How many more refusals there were than `listed` lists: each that
    /// isn't listed, a retry of one included.
    pub unlisted: u64,
    /// The grants that would have allowed the refused calls that one
    /// would have, distinct, in the order each was first needed: every fs
    /// kind, and the first [`MAX_LISTED_REFUSALS`] environment variables,
    /// each of a name at most [`MAX_LISTED_MESSAGE`] bytes long. Kept apart
    /// from `listed`, so the refusal that is the run's uncaught error,
    /// which isn't listed, still has its grant here.
    pub needed: Vec<NeededGrant>,
    /// Whether a refused call needed a variable `needed` doesn't name,
    /// past its bound or longer than a name it keeps.
    pub more_names: bool,
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

    /// Records a refusal by `by` whose error carries `message`: listed,
    /// unless it's listed already, while there's room, and counted after;
    /// and `needed`, the grant that would have allowed it, if one would
    /// have, kept unless it's kept already, while there's room.
    pub(crate) fn record(&mut self, by: RefusedBy, message: &str, needed: Option<NeededGrant>) {
        if let Some(needed) = needed {
            self.need(needed);
        }
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
    /// it's kept already: an fs kind always, there being few, and a
    /// variable while there's room for its name and it's short enough to
    /// keep, else marks that there are more.
    fn need(&mut self, needed: NeededGrant) {
        if self.needed.contains(&needed) {
            return;
        }
        if let NeededGrant::Env(name) = &needed {
            let names = self
                .needed
                .iter()
                .filter(|kept| matches!(kept, NeededGrant::Env(_)))
                .count();
            if names == MAX_LISTED_REFUSALS || name.len() > MAX_LISTED_MESSAGE {
                self.more_names = true;
                return;
            }
        }
        self.needed.push(needed);
    }

    /// Drops the listing of each refusal whose error is of the class named
    /// `error` and carries `message`, if one is listed: the run's uncaught
    /// error, which its report shows.
    pub(crate) fn unlist(&mut self, error: &str, message: &str) {
        let message = cut(message);
        self.listed
            .retain(|listed| listed.by.error() != error || *listed.message != message);
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
    use super::{MAX_LISTED_MESSAGE, MAX_LISTED_REFUSALS, NeededGrant, Refusals, RefusedBy};
    use crate::resolver::Capability;

    const ENV: RefusedBy = RefusedBy::Grant {
        grant: Capability::Env,
        error: "EnvError",
    };

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
            refused.record(RefusedBy::Import, message, None);
            refused.record(RefusedBy::Import, message, None);
        }
        assert!(!refused.is_empty());
        assert_eq!(refused.messages(), messages[..MAX_LISTED_REFUSALS]);
        assert_eq!(refused.unlisted, 0);
        for message in &messages[MAX_LISTED_REFUSALS..] {
            refused.record(RefusedBy::Import, message, None);
            refused.record(RefusedBy::Import, message, None);
        }
        // a listed one retried once the list is full still isn't counted
        refused.record(RefusedBy::Import, &messages[0], None);
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
        refused.record(RefusedBy::Import, &whole, None);
        // 'é' is two bytes, so a cut at the bound falls inside the last one
        refused.record(
            RefusedBy::Import,
            &format!("b{}", "é".repeat(MAX_LISTED_MESSAGE / 2)),
            None,
        );
        let head = "c".repeat(MAX_LISTED_MESSAGE);
        refused.record(RefusedBy::Import, &format!("{head}x"), None);
        refused.record(RefusedBy::Import, &format!("{head}y"), None);
        refused.record(RefusedBy::Import, &format!("{head}xy"), None);
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
    /// its cut text, and leaves the count.
    #[test]
    fn a_refusal_is_unlisted_by_its_message() {
        let mut refused = Refusals::default();
        let long = "c".repeat(MAX_LISTED_MESSAGE + 1);
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record(RefusedBy::Import, &format!("refused {i}"), None);
        }
        refused.record(RefusedBy::Import, &long, None);
        refused.unlist("TypeError", "refused 1");
        refused.unlist("TypeError", "refused");
        assert_eq!(refused.messages().len(), MAX_LISTED_REFUSALS - 1);
        assert!(!refused.messages().contains(&"refused 1"));
        assert_eq!(refused.unlisted, 1);

        let mut refused = Refusals::default();
        refused.record(RefusedBy::Import, &long, None);
        refused.unlist("TypeError", &long);
        assert!(refused.is_empty());

        // with none listed, the counted ones are still refusals
        let mut refused = Refusals::default();
        for i in 0..=MAX_LISTED_REFUSALS {
            refused.record(RefusedBy::Import, &format!("refused {i}"), None);
        }
        for i in 0..MAX_LISTED_REFUSALS {
            refused.unlist("TypeError", &format!("refused {i}"));
        }
        assert!(refused.listed.is_empty());
        assert!(!refused.is_empty());
    }

    /// An import's refusal and an op's are two listings even when their
    /// messages are the same text, each counted in the one bound, and each
    /// unlisted only as its own class's error.
    #[test]
    fn an_import_s_refusal_and_an_op_s_share_the_record_apart() {
        let mut refused = Refusals::default();
        refused.record(ENV, "not_granted: HOME", None);
        refused.record(RefusedBy::Import, "not_granted: HOME", None);
        refused.record(ENV, "not_granted: HOME", None);
        assert_eq!(
            refused
                .listed
                .iter()
                .map(|listed| listed.by)
                .collect::<Vec<_>>(),
            [ENV, RefusedBy::Import]
        );
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record(ENV, &format!("not_granted: N{i}"), None);
        }
        assert_eq!(refused.listed.len(), MAX_LISTED_REFUSALS);
        assert_eq!(refused.unlisted, 2);
        refused.unlist("TypeError", "not_granted: HOME");
        assert_eq!(refused.listed[0].by, ENV);
        assert_eq!(refused.listed.len(), MAX_LISTED_REFUSALS - 1);
        refused.unlist("EnvError", "not_granted: HOME");
        assert_eq!(refused.messages()[0], "not_granted: N0");
        assert_eq!(ENV.error(), "EnvError");
        assert_eq!(RefusedBy::Import.error(), "TypeError");
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
        refused.record(RefusedBy::Import, "cannot import", None);
        refused.record(ENV, "not_granted: NARROWED", None);
        assert!(refused.needed.is_empty());
        assert!(!refused.more_names);
        refused.record(ENV, "not_granted: B", Some(needs("B")));
        refused.record(ENV, "not_granted: A", Some(needs("A")));
        refused.record(ENV, "not_granted: B", Some(needs("B")));
        refused.unlist("EnvError", "not_granted: A");
        assert_eq!(refused.needed, [needs("B"), needs("A")]);
        assert!(!refused.more_names);
        // one exactly the bound long is kept, one longer marks more
        let longest = "L".repeat(MAX_LISTED_MESSAGE);
        refused.record(ENV, "not_granted: L", Some(needs(&longest)));
        assert!(!refused.more_names);
        refused.record(ENV, "not_granted: M", Some(needs(&format!("{longest}M"))));
        assert!(refused.more_names);
        assert_eq!(refused.needed.len(), 3);
        let mut refused = Refusals::default();
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record(
                ENV,
                &format!("not_granted: N{i}"),
                Some(needs(&format!("N{i}"))),
            );
        }
        assert!(!refused.more_names);
        // a kept one again past the bound is no more
        refused.record(ENV, "not_granted: N0", Some(needs("N0")));
        assert!(!refused.more_names);
        refused.record(ENV, "not_granted: X", Some(needs("X")));
        assert!(refused.more_names);
        assert_eq!(refused.needed.len(), MAX_LISTED_REFUSALS);
        assert_eq!(refused.needed[0], needs("N0"));
        // the fs kinds past the variables' bound, each once
        let fs = RefusedBy::Grant {
            grant: Capability::Fs,
            error: "FsError",
        };
        for (path, needed) in [
            ("/a", NeededGrant::Write),
            ("/b", NeededGrant::Read),
            ("/c", NeededGrant::Write),
            ("/d", NeededGrant::OpenDir),
            ("/e", NeededGrant::Read),
        ] {
            refused.record(fs, &format!("not_granted: {path}"), Some(needed));
        }
        assert_eq!(
            refused.needed[MAX_LISTED_REFUSALS..],
            [NeededGrant::Write, NeededGrant::Read, NeededGrant::OpenDir]
        );
        // and none takes a variable's room
        let mut refused = Refusals::default();
        refused.record(fs, "not_granted: /a", Some(NeededGrant::Read));
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record(
                ENV,
                &format!("not_granted: N{i}"),
                Some(needs(&format!("N{i}"))),
            );
        }
        assert!(!refused.more_names);
        assert_eq!(refused.needed.len(), MAX_LISTED_REFUSALS + 1);
    }
}
