// the driver evaluates first, and already sees `determinism: deny`: what
// it reads at its top level, before any of the module's code runs
const refused = (call) => {
	try {
		call();
		return 'ran';
	} catch (error) {
		return error.message.slice(0, error.message.indexOf(':'));
	}
};

const seen = {
	random: refused(() => Math.random()),
	now: refused(() => Date.now()),
	date: refused(() => new Date()),
	performance: refused(() => Object.getPrototypeOf(performance).now.call(performance)),
	timers: [typeof setTimeout, typeof setInterval, typeof clearTimeout, typeof clearInterval]
};

export default () => seen;
