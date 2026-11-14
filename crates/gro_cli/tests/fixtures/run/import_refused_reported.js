// a refusal by rule is reported on stderr even when the program catches it, once a site;
// probing for what isn't there, or for an attribute that isn't supported, stays quiet
const attempt = async (f) => {
	try {
		await f();
	} catch {}
};
for (let i = 0; i < 3; i++) {
	await attempt(() => import('@fuzdev/gro/process.js'));
	await attempt(() => import('./sibling.js', { with: { x: 'y' } }));
	await attempt(() => (0, eval)("import('./sibling.js')"));
	await attempt(() => import('optional-dependency'));
	await attempt(() => import('./missing.js'));
}
console.log('done');
