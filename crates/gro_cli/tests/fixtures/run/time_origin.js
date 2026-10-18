// the time origin is this process's start, not a snapshot's build
console.log(
	JSON.stringify({
		origin: performance.timeOrigin,
		drift: performance.timeOrigin + performance.now() - Date.now()
	})
);
