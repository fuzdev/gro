// past the refusals the report lists, the rest are counted, a retry included;
// a probe of a builtin after the flood is listed, and its retry is neither
const attempt = async (f) => {
	try {
		await f();
	} catch {}
};
for (let i = 0; i < 101; i++) {
	await attempt(() => import(`ext:${i}`));
}
// retries of the one past the bound, each counted
await attempt(() => import('ext:100'));
await attempt(() => import('ext:100'));
await attempt(() => import('@fuzdev/gro/process.js'));
await attempt(() => import('@fuzdev/gro/process.js'));
console.log('done');
