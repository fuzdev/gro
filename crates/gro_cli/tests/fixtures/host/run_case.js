// Runs one case: the scenario in `cases/` named for its host module, given the module's
// namespace as `load` imports it on this runtime, printing what it returns as one line of JSON.

/**
 * @param {string} module - the host module's name, like `cwd`
 * @param {Array<string>} rest - the arguments after it
 * @param {(module: string) => Promise<object>} load - imports the host module's namespace
 */
export const run_case = async (module, rest, load) => {
	const { default: scenario } = await import(`./cases/${module}.js`);
	console.log(JSON.stringify(scenario(await load(module), rest)));
};
