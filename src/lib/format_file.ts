import prettier from 'prettier';
import {extname} from 'node:path';

import {package_json_load} from './package_json.ts';

let cached_base_options: prettier.Options | undefined;

/**
 * Formats a file with Prettier.
 * @param content
 * @param options
 * @param base_options - defaults to the cwd's `package.json` `prettier` value
 */
export const format_file = async (
	content: string,
	options: prettier.Options,
	base_options: prettier.Options | null | undefined = cached_base_options,
): Promise<string> => {
	const final_base_options =
		base_options !== undefined
			? base_options
			: (cached_base_options = (await package_json_load()).prettier as any);
	let final_options = options;
	if (options.filepath && !options.parser) {
		const {filepath, ...rest} = options;
		const parser = infer_parser(filepath);
		if (parser) final_options = {...rest, parser};
	}
	try {
		return await prettier.format(content, {...final_base_options, ...final_options});
	} catch (_err) {
		return content;
	}
};

// This is just a simple convenience for callers so they can pass a file path.
// They can provide the Prettier `options.parser` for custom extensions.
const infer_parser = (path: string): string | null => {
	const extension = extname(path).substring(1);
	switch (extension) {
		case 'svelte':
		case 'xml': {
			return extension;
		}
		default: {
			return null;
		}
	}
};
