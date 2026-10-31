// grows the heap a small array at a time, each kept, past any cap
const kept = [];
for (let i = 0; ; i++) kept.push(new Array(1000).fill(i));
