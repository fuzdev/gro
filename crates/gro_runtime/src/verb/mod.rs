//! The verb's pre-pass: what a command of Gro's may reach, and whether it
//! runs on the binary.
//!
//! Both are decided for a command like `gro check` or `gro gen --check` from
//! the command line and the grants table before any JS runs, so a run can be
//! held to it from its first module.
//!
//! - [`table`]: the grants table, `src/builtin_grants.json`, carried in the
//!   binary.
//!
//! A project's `package.json` and installed packages are read by the
//! resolver's reads ([`crate::resolver::read_project_package`],
//! [`crate::resolver::read_installed_package`]). Nothing here touches the
//! filesystem, the environment, or V8: each part decides from what it's
//! handed.

pub mod table;
