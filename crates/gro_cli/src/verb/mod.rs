//! The verb's pre-pass, `fgro`'s part of it: the command line's grant flags
//! ([`grant_flags`]), why a command runs on the Node Gro ([`fallback`]),
//! the entry a run starts from ([`entry`]), and the plan they make together
//! ([`prepass`]), beside the closure and the grants table it reads
//! (`gro_runtime::verb`); then the run's start in its kernel box
//! ([`start`]).

pub mod entry;
pub mod fallback;
pub mod grant_flags;
pub mod prepass;
pub mod start;

#[cfg(test)]
pub mod tests;
