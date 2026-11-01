// grows the heap a short string at a time, each kept, past any cap: a heap
// that, read once the run has stopped, can be all but empty
const kept = [];
for (let i = 0; ; i++) kept.push('x'.repeat(50) + i);
