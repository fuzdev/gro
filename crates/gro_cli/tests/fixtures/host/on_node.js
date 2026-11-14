// A case under Node, on the Node implementation in `src/lib`, run by Node's own type
// stripping with no loader: `node on_node.js <module> [args...]`.

import { run_case } from './run_case.js';

const [module, ...rest] = process.argv.slice(2);

await run_case(
	module,
	rest,
	(name) => import(new URL(`../../../../../src/lib/${name}.ts`, import.meta.url).href)
);
