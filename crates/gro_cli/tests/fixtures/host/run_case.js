// Runs one case: the scenario in `cases/` named for its host module, given the module's
// namespace as `load` imports it on this runtime, printing what it returns as one line of JSON.
// A scenario that ends the process, or prints from a later job, prints for itself, with the
// `print` it's given, and returns nothing: every line a case prints is one a run must print,
// so a line printed after a call that ends the process fails the case.

/**
 * @param {string} module - the host module's name, like `cwd`
 * @param {Array<string>} rest - the arguments after it
 * @param {(module: string) => Promise<object>} load - imports a host module's namespace, which a
 * scenario is handed too, for a second module it needs
 */
export const run_case = async (module, rest, load) => {
	const { default: scenario } = await import(`./cases/${module}.js`);
	const print = (/** @type {unknown} */ value) => console.log(JSON.stringify(value));
	const result = await scenario(await load(module), rest, print, load);
	if (result !== undefined) print(result);
};
