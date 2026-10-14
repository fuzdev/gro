import type { ArgSchema } from '@fuzdev/fuz_util/args.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { print_value } from '@fuzdev/fuz_util/print.ts';
import { plural } from '@fuzdev/fuz_util/string.ts';
import type { AnySchema, SchemaNode } from '@fuzdev/fuz_schema/schema.ts';
import {
	format_schema_type,
	get_schema_entries,
	get_schema_fields,
	unwrap_schema,
	type SchemaField
} from '@fuzdev/fuz_schema/introspect.ts';
import { styleText as st } from 'node:util';

import type { LoadedTasks, TaskModuleMeta } from './task.ts';
import { print_path } from './paths.ts';

export const log_tasks = (log: Logger, loaded_tasks: LoadedTasks, log_intro = true): void => {
	const { modules, found_tasks } = loaded_tasks;
	const { resolved_input_files_by_root_dir } = found_tasks;

	const logged: Array<string> = [];
	if (log_intro) {
		logged.unshift(
			`\n\n${st('gray', 'Run a task:')} gro [name]`,
			`\n${st('gray', 'View help:')}  gro [name] --help`
		);
	}

	for (const [root_dir, resolved_input_files] of resolved_input_files_by_root_dir) {
		const dir_label = print_path(root_dir);
		if (!resolved_input_files.length) {
			log.info(`No tasks found in ${dir_label}.`);
			continue;
		}
		logged.push(
			`${log_intro ? '\n\n' : ''}${resolved_input_files.length} task${plural(
				resolved_input_files.length
			)} in ${dir_label}:\n`
		);
		const longest_task_name = to_max_length(modules, (m) => m.name);
		for (const resolved_input_file of resolved_input_files) {
			const meta = modules.find((m) => m.id === resolved_input_file.id)!;
			logged.push(
				'\n' + st('cyan', meta.name.padEnd(longest_task_name)),
				'  ',
				meta.mod.task.summary ?? ''
			);
		}
	}
	log[log_intro ? 'info' : 'raw'](logged.join('') + '\n');
};

export const log_error_reasons = (log: Logger, reasons: Array<string>): void => {
	for (const reason of reasons) {
		log.error(st('red', reason));
	}
};

const ARGS_PROPERTY_NAME = '[...args]';

export const log_task_help = (log: Logger, meta: TaskModuleMeta): void => {
	const {
		name,
		mod: { task }
	} = meta;
	const logged: Array<string> = [];
	logged.push(
		st('cyan', name),
		'help',
		st('cyan', `\n\ngro ${name}`) + `: ${task.summary ?? '(no summary available)'}\n`
	);
	if (task.Args) {
		const properties = to_arg_properties(task.Args, meta, log);
		// TODO hacky padding for some quick and dirty tables
		const longest_task_name = Math.max(
			ARGS_PROPERTY_NAME.length,
			to_max_length(properties, (p) => p.name)
		);
		const longest_type = to_max_length(properties, (p) => p.schema.type);
		const longest_default = to_max_length(properties, (p) => print_value(p.schema.default));
		for (const property of properties) {
			const name = property.name === '_' ? ARGS_PROPERTY_NAME : property.name;
			logged.push(
				`\n${st('green', name.padEnd(longest_task_name))} `,
				st('gray', property.schema.type.padEnd(longest_type)) + ' ',
				print_value(property.schema.default).padEnd(longest_default) + ' ',
				property.schema.description || '(no description available)'
			);
		}
		if (!properties.length) {
			logged.push('\n' + st('gray', 'this task has no args'));
		}
	}
	log.info(...logged, '\n');
};

interface ArgSchemaProperty {
	name: string;
	schema: ArgSchema;
}

// TODO overly restrictive, support optional objects and/or unions?
const to_arg_properties = (
	schema: AnySchema,
	meta: TaskModuleMeta,
	log: Logger
): Array<ArgSchemaProperty> => {
	if (!get_schema_entries(schema)) {
		log.error(
			`Expected Args for task "${meta.name}" to be an object schema but got ${unwrap_schema(schema).kind}`
		);
		return [];
	}
	const fields = get_schema_fields(schema);
	const names = new Set(fields.map((f) => f.name));
	const properties: Array<ArgSchemaProperty> = [];
	for (const field of fields) {
		if (names.has('no-' + field.name)) continue;
		properties.push({
			name: field.name,
			schema: {
				type: format_schema_type(field.schema),
				description: field.description ?? '',
				default: to_field_default(field) as ArgSchema['default']
			}
		});
	}
	return properties;
};

// the default a field's help shows: a `with_default_fn`'s is made by calling its `make`,
// as each parse that fills the key in does, found by walking the layers `get_schema_fields`
// reads, whose first `optional` or default it is when `default_computed`
const to_field_default = (field: SchemaField): unknown => {
	if (!field.default_computed) return field.default;
	let layer: SchemaNode = field.schema;
	for (;;) {
		switch (layer.kind) {
			case 'with_default_fn':
				return layer.def.make();
			case 'optional':
			case 'nullable':
			case 'with_default':
				layer = layer.def.schema;
				break;
			case 'pipe':
				layer = layer.def[0]!;
				break;
			case 'lazy':
				layer = layer.def();
				break;
			default:
				return undefined;
		}
	}
};

const to_max_length = <T>(items: Array<T>, toString: (item: T) => string) =>
	items.reduce((max, m) => Math.max(toString(m).length, max), 0);
