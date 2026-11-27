//! The verb's pre-pass, `fgro`'s part of it: the command line's grant flags
//! ([`grant_flags`]), why a command runs on the Node Gro ([`fallback`]),
//! the entry a run starts from ([`entry`]), and the plan they make together
//! ([`prepass`]), which the run's start in its kernel box takes
//! (`crate::start`), beside the closure and the grants table it reads
//! (`gro_runtime::verb`).

pub mod entry;
pub mod fallback;
pub mod grant_flags;
pub mod prepass;

#[cfg(test)]
pub mod tests;
