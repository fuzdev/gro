---
'@fuzdev/gro': minor
---

feat: `fs`'s `write` checks its data, a string or a `Uint8Array` (a `Buffer` among them), and `mkdir` reads `recursive` by truthiness, as the runtime's builtin does

**breaking** `write` with any other data throws `TypeError` `fs write data is a string or a Uint8Array`, before anything is written, where a number, `null`, `undefined`, an `ArrayBuffer`, an array of numbers, or another object failed as `io_error`, and a `Uint16Array`, a `DataView`, another view, a boxed string, or an array of strings was written. Pass a string, or the bytes as a `Uint8Array` (`new Uint8Array(view.buffer, view.byteOffset, view.byteLength)` for another view).

`mkdir(path, {recursive: 1})` makes the directory and its parents, where any `recursive` but a boolean failed as `io_error`; a falsy one is no `recursive`, as `exists_ok`, `remove`'s `recursive` and `missing_ok`, and `write`'s `create_new` already were.
