---
'@fuzdev/gro': minor
---

feat: hold the host modules' errors to the runtime's contract: every `message` is exactly `kind: target`, the detail in its `cause`, which `format_error_detail` prints, and the error classes take no message

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
