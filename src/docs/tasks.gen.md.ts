import { relative } from 'node:path';

import type { Gen } from '../lib/gen.ts';
import { paths } from '../lib/paths.ts';
import { task_registry_find_builtins } from '../lib/task_registry.ts';
import {
	create_gen_doc_context,
	create_root_link,
	create_breadcrumbs,
	create_gen_footer
} from './gro_docs_gen_helpers.ts';

// This is the first simple implementation of Gro's automated docs.
// It combines Gro's gen and task systems
// to generate a markdown file with a summary of all of Gro's tasks.
// Other projects that use Gro should be able to import this module
// or other otherwise get frictionless access to this specific use case,
// and they should be able to extend or customize it to any degree.

// TODO display more info about each task, including a summary and params
// TODO needs some cleanup and better APIs - paths are confusing and verbose!
// TODO add backlinks to every document that links to this one

export const gen: Gen = async ({ origin_id }) => {
	// read from the task modules, not the generated summaries, so one `gro gen` pass is enough
	const tasks = await task_registry_find_builtins(paths.lib);

	const ctx = create_gen_doc_context(origin_id);
	const { origin_dir, origin_base, root_path, output_file_name, relative_dir } = ctx;

	// TODO this is GitHub-specific
	const root_link = create_root_link(root_path);

	// TODO do we want to use absolute paths instead of relative paths,
	// because GitHub works with them and it simplifies the code?
	const breadcrumbs = create_breadcrumbs(root_link, relative_dir, origin_dir, output_file_name);

	// TODO render the footer with the origin_id
	return `# tasks

${breadcrumbs}

What is a \`Task\`? See [\`task.md\`](./task.md).

## all tasks

${tasks.reduce(
	(result, task) =>
		result +
		`- [${task.name}](${relative(origin_dir, task.id)})${
			task.summary ? ` - ${task.summary}` : ''
		}\n`,
	''
)}
## usage

\`\`\`bash
$ gro # lists the builtins
$ gro check # runs a builtin by name
$ gro task # lists the project's tasks under src/
$ gro task src/lib/some/file.task.ts # runs a project task by path
\`\`\`

${breadcrumbs}

${create_gen_footer(origin_base)}
`;
};
