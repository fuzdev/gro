---
'@fuzdev/gro': minor
---

feat: add `fs.real_path`, an absolute path's real path with every symlink followed, on the fs root beside `open_dir`

`FsPaths` resolves symlinks through the `Fs` it's built from, `fs.real_path` and a handle's `read_link`,
never `node:fs` directly, so a hand-written `Fs` passed to `create_fs_paths` needs a `real_path`.

**breaking** an `FsPaths` path holding a NUL byte or a lone surrogate is a `TypeError`, as for every fs op,
where a NUL byte was an `io_error` and a lone surrogate could resolve as the U+FFFD name it encodes to.
