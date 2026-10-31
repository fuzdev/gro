// code generation from strings throws an EvalError, whatever the JIT
const thrown = (generate) => {
	try {
		generate();
		return 'generated';
	} catch (error) {
		return error instanceof EvalError ? 'EvalError' : String(error);
	}
};

const AsyncFunction = (async () => {}).constructor;

export default {
	eval: thrown(() => eval('1')),
	indirect_eval: thrown(() => (0, eval)('1')),
	function: thrown(() => new Function('return 1')),
	async_function: thrown(() => new AsyncFunction('return 1'))
};
