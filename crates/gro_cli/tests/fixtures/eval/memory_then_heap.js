// array buffers holding most of the process's memory cap (`MEMORY_LIMIT`),
// then a heap grown a small array at a time: the heap's pages run the
// process out of memory long before the heap nears its cap, so the stop is
// `memory` (the buffers' pages untouched, so not resident when the cap is off)
const held = [];
for (let i = 0; i < 26; i++) held.push(new ArrayBuffer(64 << 20));
const kept = [];
for (let i = 0; ; i++) kept.push(new Array(1000).fill(i));
