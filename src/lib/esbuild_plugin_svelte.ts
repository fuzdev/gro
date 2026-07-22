import * as esbuild from 'esbuild';
import {
	compile,
	compileModule,
	preprocess,
	type CompileOptions,
	type ModuleCompileOptions,
	type PreprocessorGroup
} from 'svelte/compiler';
import { readFile } from 'node:fs/promises';
import { relative } from 'node:path';

import { to_define_import_meta_env, default_ts_transform_options } from './esbuild_helpers.ts';
import {
	default_svelte_config,
	to_default_compile_module_options,
	type ParsedSvelteConfig
} from './svelte_config.ts';
import { TS_MATCHER, SVELTE_MATCHER, SVELTE_RUNES_MATCHER } from './constants.ts';

export interface EsbuildPluginSvelteOptions {
	dev: boolean;
	base_url: ParsedSvelteConfig['base_url'];
	dir?: string;
	svelte_compile_options?: CompileOptions;
	svelte_compile_module_options?: ModuleCompileOptions;
	svelte_preprocessors?: PreprocessorGroup | Array<PreprocessorGroup>;
	ts_transform_options?: esbuild.TransformOptions;
	is_ts?: (filename: string) => boolean;
}

export const esbuild_plugin_svelte = (options: EsbuildPluginSvelteOptions): esbuild.Plugin => {
	const {
		dev,
		base_url,
		dir = process.cwd(),
		svelte_compile_options = default_svelte_config.svelte_compile_options,
		svelte_compile_module_options = to_default_compile_module_options(svelte_compile_options),
		svelte_preprocessors,
		ts_transform_options = default_ts_transform_options,
		is_ts = (f) => TS_MATCHER.test(f)
	} = options;

	const final_ts_transform_options: esbuild.TransformOptions = {
		...ts_transform_options,
		define: to_define_import_meta_env(dev, base_url),
		sourcemap: 'inline'
	};

	return {
		name: 'svelte',
		setup: (build) => {
			build.onLoad({ filter: SVELTE_RUNES_MATCHER }, async ({ path }) => {
				const source = await readFile(path, 'utf8');
				try {
					const filename = relative(dir, path);
					const js_source = is_ts(filename)
						? (
								await esbuild.transform(source, {
									...final_ts_transform_options,
									sourcefile: filename
								})
							).code // TODO @many use warnings? handle not-inline sourcemaps?
						: source;
					const { js, warnings } = compileModule(js_source, {
						...svelte_compile_module_options,
						filename
					});
					const contents = js.code + '//# sourceMappingURL=' + js.map.toUrl();
					return {
						contents,
						warnings: warnings.map((w) => convert_svelte_message_to_esbuild(filename, source, w))
					};
				} catch (error) {
					return { errors: [convert_svelte_message_to_esbuild(path, source, error)] };
				}
			});

			build.onLoad({ filter: SVELTE_MATCHER }, async ({ path }) => {
				let source = await readFile(path, 'utf8');
				try {
					const filename = relative(dir, path);
					const preprocessed = svelte_preprocessors
						? await preprocess(source, svelte_preprocessors, { filename })
						: null;
					if (preprocessed?.code) source = preprocessed.code;
					const { js, warnings } = compile(source, { ...svelte_compile_options, filename });
					const contents = js.code + '//# sourceMappingURL=' + js.map.toUrl();
					return {
						contents,
						warnings: warnings.map((w) => convert_svelte_message_to_esbuild(filename, source, w))
					};
				} catch (error) {
					return { errors: [convert_svelte_message_to_esbuild(path, source, error)] };
				}
			});
		}
	};
};

/**
 * Following the example in the esbuild docs:
 * https://esbuild.github.io/plugins/#svelte-plugin
 */
const convert_svelte_message_to_esbuild = (
	path: string,
	source: string,
	{ message, start, end }: SvelteError
): esbuild.PartialMessage => {
	let location: esbuild.PartialMessage['location'] = null;
	if (start && end) {
		const lineText = source.split(/\r\n|\r|\n/g)[start.line - 1] ?? '';
		const lineEnd = start.line === end.line ? end.column : lineText.length;
		location = {
			file: path,
			line: start.line,
			lineText,
			column: start.column,
			length: lineEnd - start.column
		};
	}
	return { text: message, location };
};

// these are not exported by Svelte
interface SvelteError {
	message: string;
	start?: LineInfo;
	end?: LineInfo;
}
interface LineInfo {
	line: number;
	column: number;
}
