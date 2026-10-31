// no timer is reachable: the globals are gone, and `AbortSignal.timeout`, the
// one other way to the event loop's timers, refuses
const refused = (call) => {
	try {
		call();
		return 'ran';
	} catch (error) {
		return `${error.constructor.name}: ${error.message}`;
	}
};

export default {
	globals: [typeof setTimeout, typeof setInterval, typeof clearTimeout, typeof clearInterval],
	abort_signal_timeout: refused(() => AbortSignal.timeout(1)),
	atomics_wait: refused(() => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1))
};
