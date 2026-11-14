// the web globals whose own code calls an op eval refuses: what each call did, `ran` or
// the message it threw, which `determinism: deny` stands in front of unless it's off
const call = (/** @type {() => unknown} */ f) => {
	try {
		f();
		return 'ran';
	} catch (error) {
		return /** @type {Error} */ (error).message;
	}
};

export default {
	now: call(() => performance.now()),
	blob: call(() => new Blob(['a'])),
	timer: call(() => clearTimeout(setTimeout(() => {}, 1)))
};
