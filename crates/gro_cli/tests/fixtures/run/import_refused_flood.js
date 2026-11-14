// past the refusals listed, each is counted, a retry of one included; a retry of a listed one
// is neither
const attempt = async (f) => {
	try {
		await f();
	} catch {}
};
for (let i = 0; i < 10; i++) {
	await attempt(() => import(`ext:${i}`));
}
await attempt(() => import('ext:0'));
await attempt(() => import('ext:9'));
await attempt(() => import('@fuzdev/gro/args.js'));
console.log('done');
