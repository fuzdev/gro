import { basename } from 'node:path';
import { fs_search } from '@fuzdev/fuz_util/fs.ts';

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
 * Renders a simple index of a possibly nested directory of files.
 */
export const gen: Gen = async ({ origin_id }) => {
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

	// TODO render the footer with the origin_id
	return `# docs


${breadcrumbs}

${doc_paths.reduce((result, path) => result + `- [${basename(path, '.md')}](${path})\n`, '')}
${breadcrumbs}

${create_gen_footer(origin_base)}
`;
};
