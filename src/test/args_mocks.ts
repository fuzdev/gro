const mock_cli_args: { current: ReadonlyArray<string> } = { current: [] };

/**
 * Sets the args a mocked `args` reads, the CLI's args after its own path.
 */
export const set_mock_args = (args: ReadonlyArray<string>): void => {
	mock_cli_args.current = args;
};

/**
 * A mock of `#lib/args.ts` for tests of a root that reads `args`,
 * the snapshot the module takes at load being the one thing a test can't replace.
 */
export const create_mock_args_module = (): typeof import('#lib/args.ts') => ({
	get args() {
		return mock_cli_args.current;
	}
});
