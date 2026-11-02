// grows a map past the heap's cap: with the JIT, the heap first runs short on
// one large table the map grows into, before it's made
const map = new Map();
for (let i = 0; ; i++) map.set(i, { i });
