// the event loop drains: the run ends only after every live timer fires, and
// a cleared timer never does
const cleared = setTimeout(() => console.log('cleared timeout fired'), 0);
clearTimeout(cleared);
let ticks = 0;
const interval = setInterval(() => {
	ticks++;
	console.log('tick', ticks);
	if (ticks === 3) {
		clearInterval(interval);
		setTimeout(() => console.log('timeout'), 1);
	}
}, 1);
queueMicrotask(() => console.log('microtask'));
console.log('sync');
