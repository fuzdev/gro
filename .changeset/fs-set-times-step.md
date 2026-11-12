---
'@fuzdev/gro': patch
---

fix: `set_times` stores an integer millisecond exactly on Node 24

Node's libuv truncates the seconds `utimes` takes to a microsecond on libuv 1.51 (Node 24.14) and to
a nanosecond on 1.52.1 (Node 26.13). The Node implementation aimed half a nanosecond past the time, so on Node 24 a
time whose seconds double fell short of it, like `1700000000123`, was stored a whole microsecond
short and `stat` read it back as `1700000000122.999`. It now aims half the platform's step past the
time and splits off whole seconds first, so on Node 24 every integer millisecond through 2242 is
stored exactly and any other time within a microsecond. On Node 26 a time is stored within half the
seconds double's spacing, as before, and `stat` shows an integer millisecond exactly outside the
spans where that spacing is coarser than the milliseconds double's. Bun and Deno, which set times
without libuv, keep the nanosecond aim.
