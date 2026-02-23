import type {ArgSchema} from '@fuzdev/fuz_util/args.js';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import {print_value} from '@fuzdev/fuz_util/print.js';
import {plural} from '@fuzdev/fuz_util/string.js';
import {
	zod_to_subschema,
	zod_to_schema_description,
	zod_to_schema_default,
} from '@fuzdev/fuz_util/zod.js';
import {styleText as st} from 'node:util';
import {z} from 'zod';

import type {LoadedTasks, TaskModuleMeta} from './task.ts';
import {print_path} from './paths.ts';

export const log_tasks = (log: Logger, loaded_tasks: LoadedTasks, log_intro = true): void => {
	const {modules, found_tasks} = loaded_tasks;
	const {resolved_input_files_by_root_dir} = found_tasks;

	const logged: Array<string> = [];
	if (log_intro) {
		logged.unshift(
			`\n\n${st('gray', 'Run a task:')} gro [name]`,
			`\n${st('gray', 'View help:')}  gro [name] --help`,
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
				resolved_input_files.length,
			)} in ${dir_label}:\n`,
		);
		const longest_task_name = to_max_length(modules, (m) => m.name);
		for (const resolved_input_file of resolved_input_files) {
			const meta = modules.find((m) => m.id === resolved_input_file.id)!;
			logged.push(
				'\n' + st('cyan', meta.name.padEnd(longest_task_name)),
				'  ',
				meta.mod.task.summary ?? '',
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
		mod: {task},
	} = meta;
	const logged: Array<string> = [];
	logged.push(
		st('cyan', name),
		'help',
		st('cyan', `\n\ngro ${name}`) + `: ${task.summary ?? '(no summary available)'}\n`,
	);
	if (task.Args) {
		const properties = to_arg_properties(task.Args, meta, log);
		// TODO hacky padding for some quick and dirty tables
		const longest_task_name = Math.max(
			ARGS_PROPERTY_NAME.length,
			to_max_length(properties, (p) => p.name),
		);
		const longest_type = to_max_length(properties, (p) => p.schema.type);
		const longest_default = to_max_length(properties, (p) => print_value(p.schema.default));
		for (const property of properties) {
			const name = property.name === '_' ? ARGS_PROPERTY_NAME : property.name;
			logged.push(
				`\n${st('green', name.padEnd(longest_task_name))} `,
				st('gray', property.schema.type.padEnd(longest_type)) + ' ',
				print_value(property.schema.default).padEnd(longest_default) + ' ',
				property.schema.description || '(no description available)',
			);
		}
		if (!properties.length) {
			logged.push('\n' + st('gray', 'this task has no args'));
		}
	}
	log.info(...logged, '\n');
};

// TODO rework all of this
// The following Zod helpers only need to support single-depth schemas for CLI args,
// but there's generic recursion to handle things like `ZodOptional` and `ZodDefault`.

interface ArgSchemaProperty {
	name: string;
	schema: ArgSchema;
}

// TODO this blocks many usecases like unions, and it's only implemented for CLI arg types, need better support for arbitrary schemas
const to_arg_properties = (
	schema: z.ZodType,
	meta: TaskModuleMeta,
	log: Logger,
): Array<ArgSchemaProperty> => {
	const {def} = schema;

	// TODO overly restrictive, support optional objects and/or unions?
	if (!('shape' in def)) {
		log.error(`Expected Args for task "${meta.name}" to be an object schema but got ${def.type}`);
		return [];
	}
	const shape = (def as z.core.$ZodObjectDef).shape;

	const properties: Array<ArgSchemaProperty> = [];
	for (const name in shape) {
		if ('no-' + name in shape) continue;
		const s = shape[name] as z.ZodType;
		const schema: ArgSchema = {
			type: to_args_schema_type(s),
			description: zod_to_schema_description(s) || '',
			default: zod_to_schema_default(s) as ArgSchema['default'],
		};
		properties.push({name, schema});
	}
	return properties;
};

const to_max_length = <T>(items: Array<T>, toString: (item: T) => string) =>
	items.reduce((max, m) => Math.max(toString(m).length, max), 0);

const to_args_schema_type = (schema: z.ZodType): ArgSchema['type'] => {
	const {def} = schema._zod;
	switch (def.type) {
		case 'string':
			return 'string';
		case 'number':
			return 'number';
		case 'int':
			return 'int';
		case 'boolean':
			return 'boolean';
		case 'bigint':
			return 'bigint';
		case 'symbol':
			return 'symbol';
		case 'null':
			return 'null';
		case 'undefined':
			return 'undefined';
		case 'void':
			return 'void';
		case 'never':
			return 'never';
		case 'any':
			return 'any';
		case 'unknown':
			return 'unknown';
		case 'date':
			return 'date';
		case 'object':
			return 'object';
		case 'record':
			return 'record';
		case 'file':
			return 'file';
		case 'array':
			// TODO other types, only handling a subset of CLI arg cases
			return 'Array<string>';
		case 'tuple':
			return 'tuple';
		case 'union':
			// TODO fix, this is a hacky way to handle unions for CLI args
			return 'string | Array<string>';
		case 'intersection':
			return 'intersection';
		case 'map':
			return 'map';
		case 'set':
			return 'set';
		case 'enum':
			return (schema as unknown as {options: Array<string>}).options
				.map((v) => `'${v}'`)
				.join(' | ');
		case 'literal':
			return (def as unknown as {values: Array<any>}).values.map((v) => print_value(v)).join(' | ');
		case 'nullable': {
			const subschema = zod_to_subschema(def);
			return subschema ? to_args_schema_type(subschema) + ' | null' : 'nullable';
		}
		case 'optional': {
			const subschema = zod_to_subschema(def);
			return subschema ? to_args_schema_type(subschema) + ' | undefined' : 'optional';
		}
		case 'success':
			return 'success';
		case 'catch':
			return 'catch';
		case 'nan':
			return 'NaN';
		case 'readonly':
			return 'readonly';
		case 'template_literal':
			return 'template_literal';
		case 'promise':
			return 'promise';
		case 'lazy':
			return 'lazy';
		case 'custom':
			return 'custom';
		// Unwrap these:
		// case 'nonoptional':
		// case 'transform':
		// case 'default':
		// case 'prefault':
		// case 'pipe':
		default: {
			const subschema = zod_to_subschema(def);
			return subschema ? to_args_schema_type(subschema) : def.type;
		}
	}
};
