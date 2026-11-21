//! The verb's pre-pass: what a command of Gro's may reach, and whether it
//! runs on the binary.
//!
//! Both are decided for a command like `gro check` or `gro gen --check` from
//! the command line and the grants table before any JS runs, so a run can be
//! held to it from its first module.
//!
//! - [`table`]: the grants table, `src/builtin_grants.json`, carried in the
//!   binary.
//! - [`closure`]: a command line's closure, the builtins it runs and the
//!   union of their grants, or why it runs on the Node Gro.
//!
//! A closure is read from the command line with its grant flags split off,
//! the embedder's to split, and a project's `package.json` and installed
//! packages by the resolver's reads ([`crate::resolver::read_project_package`],
//! [`crate::resolver::read_installed_package`]). Nothing here touches the
//! filesystem, the environment, or V8: each part decides from what it's
//! handed.

mod argv;
pub mod closure;
pub mod table;
