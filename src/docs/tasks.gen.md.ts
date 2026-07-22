import { relative } from 'node:path';

import type { Gen } from '../lib/gen.ts';
import { paths } from '../lib/paths.ts';
import { log_error_reasons } from '../lib/task_logging.ts';
import { find_tasks, load_tasks, TaskError } from '../lib/task.ts';
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

export const gen: Gen = async ({ origin_id, log, config }) => {
	const found = await find_tasks(['.'], [paths.lib], config);
	if (!found.ok) {
		log_error_reasons(log, found.reasons);
		throw new TaskError(`Failed to generate task docs: ${found.type}`);
	}
	const found_tasks = found.value;

	const loaded = await load_tasks(found_tasks);
	if (!loaded.ok) {
		log_error_reasons(log, loaded.reasons);
		throw new TaskError(`Failed to generate task docs: ${loaded.type}`);
	}
	const loaded_tasks = loaded.value;
	const tasks = loaded_tasks.modules;

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
			task.mod.task.summary ? ` - ${task.mod.task.summary}` : ''
		}\n`,
	''
)}
## usage

\`\`\`bash
$ gro some/name
\`\`\`

${breadcrumbs}

${create_gen_footer(origin_base)}
`;
};
