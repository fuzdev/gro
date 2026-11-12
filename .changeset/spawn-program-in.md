---
'@fuzdev/gro': minor
---

feat: add `program_in` and `find_in` to the spawn host module, running a program beneath an fs handle

`spawn.program_in(dir, name, options?)` resolves a relative `name` beneath the directory of `dir`, an fs
handle, by the handle's own rules, and pins it as `program` pins a name containing `/`: an absolute name,
a climb above `dir`, an absolute symlink, or a relative one that climbs out is `not_granted`, and a link
that stays inside is followed. `find_in` returns `null` where `program_in` throws `not_found`, and a
permission error resolving the name is `not_executable`, never a miss. `dir` must be a handle from this
copy of `fs`, else a `TypeError`, and a narrowed root refuses both.

**breaking** the `Spawn` interface grows these methods, so a hand-written implementation or fake of it
no longer typechecks until it has them.

**breaking** the spawn module's `PATH` walk skips every `node_modules/.bin` entry, like those `npm run`
and `deno task` put ahead of the rest, so a dependency's bin can no longer shadow a bare name like
`spawn.program('git')`. A project's tools are reached by a name containing `/`, as `find_cli` does, so a
tool found before only through an ancestor's `node_modules/.bin` on `PATH`, as a workspace hoists one,
is now `not_found` unless it's also installed elsewhere on `PATH`.
