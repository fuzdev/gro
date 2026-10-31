import { basename, join } from 'node:path';
import { readFile } from 'node:fs/promises';
import { fs_exists, fs_search } from '@fuzdev/fuz_util/fs.ts';

import type { Gen } from '../lib/gen.ts';
import {
	create_gen_doc_context,
	create_root_link,
	create_breadcrumbs,
	create_gen_footer
} from './gro_docs_gen_helpers.ts';

// TODO look at `tasks.gen.md.ts` to refactor and generalize
// TODO show nested structure, not a flat list
// TODO work with file types beyond markdown

/**
 * The doc whose index list names the runtime's docs, which group together here,
 * so that list stays the one source of which docs are the runtime's.
 */
const RUNTIME_INDEX_FILE = 'runtime.md';

/**
 * Parses the sibling doc paths a markdown file lists as `- [name](name.md)` items, in order.
 */
const parse_listed_doc_paths = (content: string): Array<string> => {
	const paths: Array<string> = [];
	for (const match of content.matchAll(/^- \[[^\]]+\]\(([^)/#]+\.md)\)/gm)) {
		paths.push(match[1]!);
	}
	return paths;
};

/**
 * Renders a simple index of a possibly nested directory of files,
 * with the runtime's docs grouped after the rest.
 */
export const gen: Gen = {
	inputs: { files: ['src/docs/runtime.md'] }, // `RUNTIME_INDEX_FILE`, as a literal
	generate: async ({ origin_id }) => {
		const ctx = create_gen_doc_context(origin_id);
		const { origin_dir, origin_base, root_path, output_file_name, relative_dir } = ctx;

		// TODO this is GitHub-specific
		const root_link = create_root_link(root_path);
		const doc_files = await fs_search(origin_dir);
		const doc_paths: Array<string> = [];
		for (const { path } of doc_files) {
			if (path === output_file_name || !path.endsWith('.md')) {
				continue;
			}
			doc_paths.push(path);
		}

		const runtime_index_id = join(origin_dir, RUNTIME_INDEX_FILE);
		const runtime_paths = (await fs_exists(runtime_index_id))
			? parse_listed_doc_paths(await readFile(runtime_index_id, 'utf8')).filter((path) =>
					doc_paths.includes(path)
				)
			: [];
		const other_paths = doc_paths.filter((path) => !runtime_paths.includes(path));

		// TODO do we want to use absolute paths instead of relative paths,
		// because GitHub works with them and it simplifies the code?
		const is_index_file = output_file_name === 'README.md';
		const breadcrumbs = create_breadcrumbs(
			root_link,
			relative_dir,
			origin_dir,
			output_file_name,
			is_index_file
		);

		return `# docs


${breadcrumbs}

${render_doc_list(other_paths, '')}${runtime_paths.length ? `- the experimental runtime\n${render_doc_list(runtime_paths, '  ')}` : ''}
${breadcrumbs}

${create_gen_footer(origin_base)}
`;
	}
};

const render_doc_list = (paths: Array<string>, indent: string): string =>
	paths.reduce((result, path) => result + `${indent}- [${basename(path, '.md')}](${path})\n`, '');
