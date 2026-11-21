//! The verb's pre-pass, `fgro`'s part of it: the command line's grant flags
//! ([`grant_flags`]) and why a command runs on the Node Gro
//! ([`fallback`]), beside the closure and the grants table it reads
//! (`gro_runtime::verb`).

pub mod fallback;
pub mod grant_flags;

#[cfg(test)]
pub mod tests;
