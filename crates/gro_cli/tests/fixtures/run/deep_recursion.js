// unbounded recursion meets V8's checked stack limit: caught, then uncaught
const recurse = (n) => recurse(n + 1) + 1;
try {
	recurse(0);
} catch (error) {
	console.log(error instanceof RangeError, error.message);
}
recurse(0);
