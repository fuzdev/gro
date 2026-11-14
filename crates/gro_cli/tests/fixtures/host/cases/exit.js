import { describe, thrown } from '../describe.js';

/** What `call` throws, its class and message, or `null` when it returns. */
const refusal = (/** @type {() => unknown} */ call) => {
	try {
		call();
		return null;
	} catch (error) {
		const { constructor, message } = /** @type {any} */ (error);
		return [constructor?.name ?? typeof error, message];
	}
};

/**
 * The scenarios of the exit module, by name. One that ends the process prints first what it's
 * about to do, and everything it prints after that is a line the process must never write.
 * @type {Record<string, (ns: typeof import('../../../../../../src/lib/exit.ts'), rest: Array<string>, print: (value: unknown) => void) => unknown>}
 */
const SCENARIOS = {
	// the namespace, both functions, and every value that is no code, each refused untouched;
	// the run then ends on its own, with no code set
	shape: (ns) => {
		const { exit, set_exit_code } = ns;
		let touched = 0;
		const touch = () => {
			touched++;
			return 1;
		};
		/** @type {Array<[string, unknown]>} */
		const not_codes = [
			['a string', '1'],
			['an empty string', ''],
			['a fraction', 1.5],
			['a negative', -1],
			['past a byte', 256],
			['NaN', NaN],
			['Infinity', Infinity],
			['null', null],
			['a boolean', true],
			['a bigint', 1n],
			['a symbol', Symbol.iterator],
			['a boxed number', new Number(1)],
			['valueOf', { valueOf: touch }],
			['toPrimitive', { [Symbol.toPrimitive]: touch }],
			[
				'a thenable',
				{
					get then() {
						return touch();
					}
				}
			],
			['a proxy', new Proxy({}, { get: touch, has: touch, getPrototypeOf: touch })],
			['an array', [1]],
			['a function', () => 1]
		];
		return {
			namespace: describe(ns),
			exit: { shape: describe(exit), name: exit.name, length: exit.length },
			set_exit_code: {
				shape: describe(set_exit_code),
				name: set_exit_code.name,
				length: set_exit_code.length
			},
			new_throws: [
				thrown(() => new /** @type {any} */ (exit)(1)),
				thrown(() => new /** @type {any} */ (set_exit_code)(1))
			],
			property_write_throws: thrown(() => {
				/** @type {any} */ (exit).extra = 1;
			}),
			namespace_write_throws: thrown(() => {
				/** @type {any} */ (ns).exit = null;
			}),
			exit_refuses: not_codes.map(([what, value]) => [
				what,
				refusal(() => exit(/** @type {any} */ (value)))
			]),
			set_exit_code_refuses: [
				...not_codes,
				/** @type {[string, unknown]} */ (['undefined', undefined])
			].map(([what, value]) => [what, refusal(() => set_exit_code(/** @type {any} */ (value)))]),
			set_exit_code_refuses_nothing_given: refusal(() => /** @type {any} */ (set_exit_code)()),
			touched,
			set_exit_code_returns: typeof set_exit_code.call({}, 0)
		};
	},
	// `exit` with the code given, as a number
	exit: (ns, [code], print) => {
		print({ before: `exit(${code})` });
		ns.exit(Number(code));
		print('after exit');
	},
	// `exit` with `-0`, an integer that is no less than 0
	exit_minus_zero: (ns, _rest, print) => {
		print({ before: 'exit(-0)' });
		ns.exit(-0);
		print('after exit');
	},
	// `exit` with nothing, in a run that set no code, borrowed by a receiver it never reads
	exit_nothing: (ns, _rest, print) => {
		print({ before: 'exit()' });
		ns.exit.call({});
		print('after exit');
	},
	// `exit` with nothing, and with `undefined`, uses the code set
	exit_after_set: (ns, [given], print) => {
		ns.set_exit_code(3);
		ns.set_exit_code(9);
		print({ before: `set_exit_code(9), exit(${given})` });
		if (given === 'undefined') ns.exit(undefined);
		ns.exit();
		print('after exit');
	},
	// a code given to `exit` wins over the code set, 0 included
	exit_over_set: (ns, [code], print) => {
		ns.set_exit_code(9);
		print({ before: `set_exit_code(9), exit(${code})` });
		ns.exit(Number(code));
		print('after exit');
	},
	// a run that ends on its own exits with the code set, the last one
	set_then_end: (ns, codes) => {
		for (const code of codes) ns.set_exit_code(Number(code));
		return { set: codes };
	},
	// set from a timer, after the entry evaluated
	set_late: (ns, _rest, print) => {
		setTimeout(() => {
			ns.set_exit_code(21);
			print({ set: 'in a timer' });
		}, 1);
	},
	// a value refused leaves the code set as it was
	set_refused_keeps: (ns) => {
		ns.set_exit_code(17);
		return { refused: refusal(() => ns.set_exit_code(/** @type {any} */ ('18'))) };
	},
	// no `finally` runs, and nothing after the call
	finally: (ns, _rest, print) => {
		print({ before: 'exit(5) in try/finally' });
		try {
			ns.exit(5);
			print('after exit');
		} catch (error) {
			print('caught');
		} finally {
			print('finally ran');
		}
	},
	// in a pending chain: no job queued before the call runs after it, no timer, no `finally`,
	// and no reaction of the function's promise
	pending_chain: async (ns, _rest, print) => {
		print({ before: 'exit(8) in a pending chain' });
		const chain = (async () => {
			try {
				await null;
				Promise.resolve().then(() => print('a queued job ran'));
				setTimeout(() => print('a timer ran'), 0);
				ns.exit(8);
				print('after exit');
			} finally {
				print('finally ran');
			}
		})();
		chain.then(
			() => print('the chain fulfilled'),
			() => print('the chain rejected')
		);
		await chain;
		print('the scenario went on');
	},
	// while an uncaught error is unwinding: the exit wins, and the error is never reported
	unwinding: (ns, _rest, print) => {
		print({ before: 'exit(4) while an error unwinds' });
		try {
			throw new Error('unwinding');
		} finally {
			ns.exit(4);
		}
	},
	// from a timer, with a later one pending and a code set: the code given wins, and the later
	// timer never fires
	timer: (ns, _rest, print) => {
		ns.set_exit_code(9);
		setTimeout(() => {
			print({ before: 'exit(3) in a timer' });
			ns.exit(3);
			print('after exit');
		}, 1);
		setTimeout(() => print('a later timer ran'), 5);
	}
};

/**
 * @param {typeof import('../../../../../../src/lib/exit.ts')} ns
 * @param {Array<string>} rest - the scenario's name, then its arguments
 * @param {(value: unknown) => void} print - prints a line now
 */
export default (ns, [name, ...rest], print) => {
	const scenario = SCENARIOS[/** @type {string} */ (name)];
	if (scenario === undefined) throw new Error(`no scenario named ${name}`);
	return scenario(ns, rest, print);
};
