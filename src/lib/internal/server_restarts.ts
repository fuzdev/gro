import { relative } from 'node:path';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import type { FileChange, ModuleGraph } from '../module_graph.ts';
import type { SpawnChild, SpawnProgram, SpawnResult } from '../spawn.ts';
import { spawn_restartable, type SpawnRestartable } from '../spawn_restartable.ts';
import { to_spawn_result_message } from '../cli.ts';

/**
 * How long a restart waits after the last change that calls for it,
 * so a burst like a save-all or a branch switch restarts once.
 */
const SERVER_RESTART_DELAY = 100;

/**
 * Whether `change` reaches the server: a change to the entry itself,
 * or to a module the entry imports, directly or transitively, per `graph`.
 * @param graph - the module graph, whose edges are current when it delivers `change`
 * @param entry_id - the server's entry module, absolute
 * @param change - a change the graph delivered
 */
export const change_reaches_server = async (
	graph: Pick<ModuleGraph, 'dependents'>,
	entry_id: PathId,
	change: FileChange
): Promise<boolean> => change.id === entry_id || (await graph.dependents(change.id)).has(entry_id);

/**
 * Options for `start_watched_server`.
 */
export interface WatchedServerOptions {
	/**
	 * The program that runs the server, its environment fixed for every restart.
	 */
	program: SpawnProgram;
	/**
	 * The arguments after `argv[0]`, the same for every restart.
	 */
	args: ReadonlyArray<string>;
	/**
	 * The server's entry module, absolute, whose import closure in `graph` restarts it.
	 */
	entry_id: PathId;
	/**
	 * The module graph whose changes restart the server, `null` to start it once.
	 */
	graph: Pick<ModuleGraph, 'dependents' | 'watch'> | null;
	log: Logger;
	/**
	 * Prefixes each log line.
	 */
	label: string;
	/**
	 * The directory a logged path is relative to, usually the project root.
	 */
	cwd: string;
	/**
	 * How long to wait after the last change that calls for a restart.
	 * @default SERVER_RESTART_DELAY
	 */
	restart_delay?: number;
}

/**
 * A server started by `start_watched_server`.
 */
export interface WatchedServer {
	/**
	 * The restartable child, for its `child` and `closed`.
	 */
	readonly process: SpawnRestartable;
	/**
	 * Unsubscribes from the graph, cancels any restart not yet begun,
	 * and stops the server, waiting for it to exit. Later calls share the first.
	 */
	stop: () => Promise<void>;
}

/**
 * Starts a server and, given a graph, restarts it when its entry or anything the entry
 * imports changes. A burst of changes restarts it once, after `restart_delay` of quiet,
 * and restarts never overlap, since `spawn_restartable` runs them in order.
 * The watch is subscribed before the first start and its changes count once the server is
 * running, so the `add`s it delivers when it begins, and anything else the first start
 * already reads, restart nothing.
 * A server that exits on its own is logged and stays down until the next change restarts it,
 * so a crash or a broken edit never loops.
 * @returns the started server
 * @throws `SpawnError` - `launch_failed`, as `SpawnProgram.start` does
 */
export const start_watched_server = async (
	options: WatchedServerOptions
): Promise<WatchedServer> => {
	const {
		program,
		args,
		entry_id,
		graph,
		log,
		label,
		cwd,
		restart_delay = SERVER_RESTART_DELAY
	} = options;

	let stopping: Promise<void> | null = null;
	let timer: ReturnType<typeof setTimeout> | null = null;
	let unwatch: (() => Promise<void>) | null = null;
	// a change of the burst, the first whose check finished, for the restart's log line
	let restart_reason: string | null = null;

	// the child whose exit is already awaited, since a restart sharing one in flight sees the same
	let reported: SpawnChild | null = null;
	// the restart already announced: a call while one is pending shares its promise, so it's one
	// restart however many changes join it
	let begun: Promise<void> | null = null;
	// the restart whose failure is already logged, since a restart sharing one in flight sees the same
	let failed: Promise<void> | null = null;

	// logs an exit the plugin didn't ask for, which a restart or stop sees as a cleared `child`
	const report_exit = (server: SpawnRestartable): void => {
		const { child } = server;
		if (!child || child === reported) return;
		reported = child;
		void server.closed.then((result: SpawnResult) => {
			if (stopping || server.child !== child) return;
			const message = `${label} server exited with ${to_spawn_result_message(result)}`;
			if (graph) log.error(`${message}, restarting on the next change`);
			else log.error(message);
		});
	};

	const restart = async (): Promise<void> => {
		timer = null;
		if (stopping) return;
		const restarting = server.restart();
		// one line per restart, naming the change that began it, none for a change joining it
		if (restarting !== begun) {
			begun = restarting;
			log.info(`${label} restarting server for ${restart_reason}`);
		}
		restart_reason = null;
		try {
			await restarting;
		} catch (error) {
			if (!stopping && restarting !== failed) {
				failed = restarting;
				log.error(`${label} failed to restart server, retrying on the next change`, error);
			}
			return;
		}
		if (!stopping) report_exit(server);
	};

	const schedule_restart = (change: FileChange): void => {
		if (stopping) return;
		restart_reason ??= `${change.type} ${relative(cwd, change.id)}`;
		if (timer !== null) clearTimeout(timer);
		timer = setTimeout(() => void restart(), restart_delay);
	};

	// the changes before the first start are in what it reads, the initial `add`s among them
	let subscribed = false;
	if (graph) {
		unwatch = await graph.watch((change) => {
			if (!subscribed) return;
			void change_reaches_server(graph, entry_id, change).then(
				(reaches) => {
					if (reaches) schedule_restart(change);
				},
				(error: unknown) => {
					// a graph closing under a check in flight, like when a later plugin's setup fails
					if (!stopping) log.error(`${label} failed to check ${change.id}`, error);
				}
			);
		});
	}

	let server: SpawnRestartable;
	try {
		server = await spawn_restartable(program, args);
	} catch (error) {
		if (unwatch) await unwatch();
		throw error;
	}
	subscribed = true;
	report_exit(server);

	const stop = (): Promise<void> =>
		(stopping ??= (async () => {
			if (timer !== null) {
				clearTimeout(timer);
				timer = null;
			}
			if (unwatch) await unwatch();
			await server.stop();
		})());

	return { process: server, stop };
};
