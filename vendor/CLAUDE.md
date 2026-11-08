# vendor/

Published crates vendored with gro's patches; they aren't gro's code.
[README.md](README.md) is the index: what each crate is, every patch, and how
to upgrade.

- Edit a vendored crate only as a gro patch: mark the site with a one-line
  `// gro patch:` comment saying why, add the entry to README.md, and
  regenerate the crate's patch (`cargo xtask vendor-patch <name>`).
  `cargo xtask check-rust` and the xtask's vendor tests fail until the three
  agree.
- Never reformat, lint-fix, or tidy vendored code; keep each patch minimal.
- Upgrades re-vendor from the checksum-verified `.crate` per README.md, never
  by editing the old copy forward.
