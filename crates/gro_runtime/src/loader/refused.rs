//! The `import()`s a loader refused by one of gro's rules
//! ([`FailureReason::guards_a_boundary`](super::graph::FailureReason::guards_a_boundary)),
//! which fail the run whether or not the program caught them. What a
//! program can make it keep outside V8's heap is bounded: a few messages,
//! each cut at a fixed length, and a count.

use std::borrow::Cow;

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

/// The `import()`s one of gro's rules refused during a run, a refused import
/// in a module an `import()` loaded among them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefusedImports {
    /// The message of each of the first [`MAX_LISTED_REFUSALS`] distinct
    /// refusals, in the order each first happened, as the `TypeError` JS
    /// saw carries it, cut past [`MAX_LISTED_MESSAGE`] bytes and marked
    /// there with its whole length: `cannot resolve 'data:… (4096 bytes)`.
    /// Two refusals are one when their messages, cut, are the same text,
    /// as a retry's is.
    pub listed: Vec<String>,
    /// How many more times an `import()` was refused than `listed` lists:
    /// each refusal that isn't listed, a retry of one included.
    pub unlisted: u64,
}

impl RefusedImports {
    /// Whether no `import()` was refused.
    pub const fn is_empty(&self) -> bool {
        self.listed.is_empty() && self.unlisted == 0
    }

    /// Records a refusal whose `TypeError` carries `message`: listed,
    /// unless it's listed already, while there's room, and counted after.
    pub(crate) fn record(&mut self, message: &str) {
        let message = cut(message);
        if self.listed.iter().any(|listed| *listed == message) {
            return;
        }
        if self.listed.len() == MAX_LISTED_REFUSALS {
            self.unlisted += 1;
        } else {
            self.listed.push(message.into_owned());
        }
    }

    /// Drops the listing of the refusal whose `TypeError` carries
    /// `message`, if it's listed.
    pub(crate) fn unlist(&mut self, message: &str) {
        let message = cut(message);
        self.listed.retain(|listed| *listed != message);
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
    use super::{MAX_LISTED_MESSAGE, MAX_LISTED_REFUSALS, RefusedImports};

    /// The first refusals are listed in order, a retry of a listed one
    /// changes nothing, and each one past the listed, a retry included, is
    /// counted.
    #[test]
    fn the_first_distinct_refusals_are_listed_and_the_rest_counted() {
        let mut refused = RefusedImports::default();
        assert!(refused.is_empty());
        let messages: Vec<String> = (0..MAX_LISTED_REFUSALS + 2)
            .map(|i| format!("cannot resolve 'ext:{i}'"))
            .collect();
        for message in &messages[..MAX_LISTED_REFUSALS] {
            refused.record(message);
            refused.record(message);
        }
        assert!(!refused.is_empty());
        assert_eq!(refused.listed, messages[..MAX_LISTED_REFUSALS]);
        assert_eq!(refused.unlisted, 0);
        for message in &messages[MAX_LISTED_REFUSALS..] {
            refused.record(message);
            refused.record(message);
        }
        // a listed one retried once the list is full still isn't counted
        refused.record(&messages[0]);
        assert_eq!(refused.listed, messages[..MAX_LISTED_REFUSALS]);
        assert_eq!(refused.unlisted, 4);
    }

    /// A message at the bound is listed whole, and a longer one cut at a
    /// character boundary at or below it, marked with its whole length;
    /// two with one head and one length are one listing, and a longer one
    /// with that head is another.
    #[test]
    fn a_long_message_is_cut_and_marked_with_its_length() {
        let mut refused = RefusedImports::default();
        let whole = "a".repeat(MAX_LISTED_MESSAGE);
        refused.record(&whole);
        // 'é' is two bytes, so a cut at the bound falls inside the last one
        refused.record(&format!("b{}", "é".repeat(MAX_LISTED_MESSAGE / 2)));
        let head = "c".repeat(MAX_LISTED_MESSAGE);
        refused.record(&format!("{head}x"));
        refused.record(&format!("{head}y"));
        refused.record(&format!("{head}xy"));
        assert_eq!(
            refused.listed,
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
        let mut refused = RefusedImports::default();
        let long = "c".repeat(MAX_LISTED_MESSAGE + 1);
        for i in 0..MAX_LISTED_REFUSALS {
            refused.record(&format!("refused {i}"));
        }
        refused.record(&long);
        refused.unlist("refused 1");
        refused.unlist("refused");
        assert_eq!(refused.listed.len(), MAX_LISTED_REFUSALS - 1);
        assert!(!refused.listed.iter().any(|listed| listed == "refused 1"));
        assert_eq!(refused.unlisted, 1);

        let mut refused = RefusedImports::default();
        refused.record(&long);
        refused.unlist(&long);
        assert!(refused.is_empty());

        // with none listed, the counted ones are still refusals
        let mut refused = RefusedImports::default();
        for i in 0..=MAX_LISTED_REFUSALS {
            refused.record(&format!("refused {i}"));
        }
        for i in 0..MAX_LISTED_REFUSALS {
            refused.unlist(&format!("refused {i}"));
        }
        assert!(refused.listed.is_empty());
        assert!(!refused.is_empty());
    }
}
