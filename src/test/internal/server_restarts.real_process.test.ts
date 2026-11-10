import { describe, test, expect, onTestFinished } from 'vitest';
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { start_watched_server } from '#lib/internal/server_restarts.ts';
import { create_node_module_graph } from '#lib/module_graph_node.ts';
import { spawn } from '#lib/spawn.ts';

import { create_mock_logger } from '../test_helpers.ts';

/*

A real `node` running a tiny server over a real module graph. The server listens on port 0 and
writes `<pid>.json` with its port and the message it imports into the status dir once listening,
so the test reads each start by pid, and polls with a deadline rather than sleeping.

*/

const DEADLINE = 10_000;

const SERVER = `import { createServer } from 'node:http';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { message } from './dep.ts';
const server = createServer((_req, res) => res.end(message));
server.listen(0, '127.0.0.1', () => {
	const address = server.address();
	const port = typeof address === 'object' && address ? address.port : 0;
	writeFileSync(join(process.env.STATUS_DIR, process.pid + '.json'), JSON.stringify({ port, message }));
});
process.on('SIGTERM', () => server.close(() => process.exit(0)));
`;

const to_dep = (message: string): string => `export const message: string = '${message}';\n`;

interface Start {
	pid: number;
	port: number;
	message: string;
}

const create_project = async () => {
	const root = await mkdtemp(join(tmpdir(), 'gro_server_restarts_'));
	onTestFinished(() => rm(root, { recursive: true, force: true }));
	const src = join(root, 'src');
	const status = join(root, 'status');
	await mkdir(src);
	await mkdir(status);
	await writeFile(join(src, 'server.ts'), SERVER);
	await writeFile(join(src, 'dep.ts'), to_dep('one'));
	await writeFile(join(src, 'other.ts'), 'export const other = 1;\n');
	return { root, src, status, entry: join(src, 'server.ts') };
};

const read_starts = async (status: string): Promise<Array<Start>> => {
	const starts: Array<Start> = [];
	for (const name of await readdir(status)) {
		const text = await readFile(join(status, name), 'utf8').catch(() => '');
		if (!text) continue;
		starts.push({ pid: Number(name.slice(0, -'.json'.length)), ...JSON.parse(text) });
	}
	return starts;
};

const wait_for_starts = async (status: string, count: number): Promise<Array<Start>> => {
	const start = Date.now();
	for (;;) {
		const starts = await read_starts(status);
		if (starts.length >= count) return starts;
		if (Date.now() - start > DEADLINE) throw Error(`timed out waiting for ${count} starts`);
		await new Promise((r) => setTimeout(r, 20));
	}
};

const is_pid_running = (pid: number): boolean => {
	try {
		process.kill(pid, 0);
		return true;
	} catch {
		return false;
	}
};

const fetch_text = async (port: number): Promise<string> =>
	(await fetch(`http://127.0.0.1:${port}/`)).text();

const start = async (project: Awaited<ReturnType<typeof create_project>>) => {
	const graph = create_node_module_graph({ watch: { dir: project.src, filter: null } });
	onTestFinished(() => graph.close());
	const log = create_mock_logger();
	const node = spawn.program('node', { env: { STATUS_DIR: project.status } });
	const server = await start_watched_server({
		program: node,
		// stdio left inherited, as the plugin does, the fixture printing nothing
		args: [project.entry],
		entry_id: project.entry,
		graph,
		log,
		label: '[test]',
		cwd: project.root,
		restart_delay: 50
	});
	const pids: Array<number> = [];
	onTestFinished(async () => {
		await server.stop();
		for (const pid of pids) if (is_pid_running(pid)) process.kill(pid, 'SIGKILL');
	});
	return { server, log, pids, graph };
};

describe('start_watched_server over a real node and module graph', () => {
	test('starts, restarts on an imported change, ignores an unrelated one, stops cleanly', async () => {
		const project = await create_project();
		const { server, pids, graph } = await start(project);

		const [first] = await wait_for_starts(project.status, 1);
		pids.push(first!.pid);
		expect(await fetch_text(first!.port)).toBe('one');

		await writeFile(join(project.src, 'dep.ts'), to_dep('two'));
		const starts = await wait_for_starts(project.status, 2);
		const second = starts.find((s) => s.pid !== first!.pid)!;
		pids.push(second.pid);
		expect(second.message).toBe('two');
		expect(await fetch_text(second.port)).toBe('two');
		// the restart waited for the old server to exit
		expect(is_pid_running(first!.pid)).toBe(false);

		// waits for the graph to deliver the unrelated change, then past the restart delay,
		// the timer a restart would have been scheduled on
		const other_id = join(project.src, 'other.ts');
		let seen_other!: () => void;
		const other_seen = new Promise<void>((r) => (seen_other = r));
		const unwatch = await graph.watch((change) => {
			if (change.type === 'update' && change.id === other_id) seen_other();
		});
		await writeFile(other_id, 'export const other = 2;\n');
		await other_seen;
		await unwatch();
		await new Promise((r) => setTimeout(r, 200));
		expect(await read_starts(project.status)).toHaveLength(2);
		expect(server.process.child?.pid).toBe(second.pid);

		await server.stop();
		expect(is_pid_running(second.pid)).toBe(false);
		await expect(fetch_text(second.port)).rejects.toThrow();
	});

	test('a broken edit exits the server, and fixing it brings the server back', async () => {
		const project = await create_project();
		const { server, log, pids } = await start(project);

		const [first] = await wait_for_starts(project.status, 1);
		pids.push(first!.pid);

		await writeFile(join(project.src, 'dep.ts'), "export const message: string = 'unclosed;\n");
		const deadline = Date.now() + DEADLINE;
		while ((log.error as any).mock.calls.length === 0) {
			if (Date.now() > deadline) throw Error('timed out waiting for the broken server to exit');
			await new Promise((r) => setTimeout(r, 20));
		}
		expect((log.error as any).mock.calls[0][0]).toMatch(
			/^\[test\] server exited with code \d+, restarting on the next change$/
		);
		expect(is_pid_running(first!.pid)).toBe(false);
		expect(await read_starts(project.status)).toHaveLength(1);

		await writeFile(join(project.src, 'dep.ts'), to_dep('fixed'));
		const starts = await wait_for_starts(project.status, 2);
		const fixed = starts.find((s) => s.pid !== first!.pid)!;
		pids.push(fixed.pid);
		expect(await fetch_text(fixed.port)).toBe('fixed');

		await server.stop();
		expect(is_pid_running(fixed.pid)).toBe(false);
	});
});

describe('start_watched_server over a real node that ignores SIGTERM', () => {
	test("a restart's grace kill ends the server itself, freeing its port", async () => {
		const project = await create_project();
		await writeFile(
			join(project.src, 'server.ts'),
			SERVER.replace(
				"process.on('SIGTERM', () => server.close(() => process.exit(0)));",
				"process.on('SIGTERM', () => {});"
			)
		);
		const { pids } = await start(project);

		const [first] = await wait_for_starts(project.status, 1);
		pids.push(first!.pid);
		await writeFile(join(project.src, 'dep.ts'), to_dep('two'));
		const starts = await wait_for_starts(project.status, 2);
		const second = starts.find((s) => s.pid !== first!.pid)!;
		pids.push(second.pid);
		// the next start waited out the grace and its `SIGKILL`, which reached the server
		expect(is_pid_running(first!.pid)).toBe(false);
		await expect(fetch_text(first!.port)).rejects.toThrow();
		expect(await fetch_text(second.port)).toBe('two');
	}, 15_000);
});
