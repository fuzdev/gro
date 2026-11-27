import { fs } from '@fuzdev/gro/fs.ts';
import { env } from '@fuzdev/gro/env.ts';
import { spawn } from '@fuzdev/gro/spawn.ts';
import { stdout } from '@fuzdev/gro/stdout.ts';
import { stderr } from '@fuzdev/gro/stderr.ts';
import { args } from '@fuzdev/gro/args.ts';
import { cwd } from '@fuzdev/gro/cwd.ts';
import { set_exit_code } from '@fuzdev/gro/exit.ts';
import { binary_main } from '@fuzdev/gro/binary_main.ts';

await binary_main({
	args,
	fs,
	env,
	spawn,
	stdout,
	stderr,
	cwd,
	set_exit_code,
	import_url: (url) => import(url),
	home: '/home/fixture',
	version: '0.0.0-fixture',
	cli_name: 'fgro'
});
