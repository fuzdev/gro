---
'@fuzdev/gro': minor
---

feat: hold the host modules to the runtime's contract: every error's `message` is exactly `kind: target`, the detail in its `cause`, which `format_error_detail` prints, the error classes take no message, `FsErrorKind` has `not_granted`, `list` and `fs_search` sort by the names' UTF-8 bytes, and the handles' methods are frozen

**breaking** `FsError`, `SpawnError`, and `NetError` take no message: the constructors are
`new FsError(kind, path, options?)`, `new SpawnError(kind, program, options?)`, and `new NetError(kind, target, options?)`.
Drop the third argument where one was passed, putting any detail in `{cause}`; an options object passed fourth moves to third.

**breaking** an error's `message` is exactly `kind: target`, like `not_found: /p/a.txt` or `not_granted: example.com:443`,
where it carried the OS's text or the module's explanation after it.
That detail is the `cause`, by one rule: where the module has an explanation of its own (`../x climbs above /p`,
`tool at /p/tool can't be read`), the `cause` is a plain `Error` holding it, whose own `cause` is the error the failure came from,
if any; where the module has nothing to add, the `cause` is that error, unchanged (an fs op's Node error, a stream's error);
and an error with nothing more to say than its kind and target, like `env`'s `not_granted` or a walk's `not_found`, has no `cause`.
So spawn's `launch_failed` reason, a stat's error, and the `FsError` a `program_in` lookup met are now its cause's `cause`, where they were its `cause`.
Code that matched or printed `message` for the detail prints the chain instead, with `format_error_detail` below;
gro prints it when a task fails, a task or genfile module fails to import, a genfile fails, a deploy's build fails, an error goes uncaught, a program can't be started, a path can't be read, and in its warnings, after the stack where it prints one.
`watch`'s errors are `FsError`s, so the same holds for them.

`format_error_detail(error)` in the new grant-free module `@fuzdev/gro/error_detail.ts` prints an error's message
followed by its chain of causes, each in parentheses inside the one before, at most `MAX_ERROR_CAUSES`,
control and bidirectional characters escaped, and never throws, and `format_error_causes(error)` prints the chain alone,
for after a stack: what a program that shows a host module's failure to a person calls.

**breaking** `FsErrorKind` gains `not_granted`, a reach outside what the run granted, which only the runtime's builtin module throws;
the Node module checks no grant and never throws it. An exhaustive `switch` over the kinds needs a case for it.

**breaking** `list` yields a directory's entries sorted by name, comparing the names' UTF-8 bytes (code point order),
where the order was the filesystem's, and it reads the whole directory before yielding the first entry.
A listing holding a name that isn't UTF-8 fails with `io_error` before any entry, where entries before it could be yielded first.
Code that sorted a listing itself can drop the sort if byte order is the order it wants;
JS's default sort compares UTF-16 code units, which puts an astral name before U+E000 to U+FFFF, and `localeCompare` follows a locale.

**breaking** `fs_search` sorts its results by path segment by segment, each compared by code point, its UTF-8 byte order:
`list`'s order taken depth-first, where it sorted whole paths with `localeCompare`, whose order changed with the process's locale
(`B.ts` now comes before `a.ts`, and `a/z.ts` before `a-b.ts`).
Pass `sort: false` and sort the results yourself for another order.

**breaking** the methods of every `fs`, `spawn`, `net`, and `watch` handle, the functions of the `fs` and `watch` roots,
and those of a spawned child's streams and iterators are frozen, as their prototypes already were,
and so is a generator method's own `prototype` (`list`'s, which every listing inherits from),
so writing a property on one (`fs.project.read.cache = …`) is a `TypeError` in strict code, where it was stored on the function every holder shares.
Keep such a value beside the handle instead.
