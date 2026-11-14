// the project's respellings of a builtin and other refusals, each past its bound
const attempt = async (f) => {
	try {
		await f();
	} catch {}
};
// the first is the project's probe, then 100 more, then 5 counted
for (let i = 0; i < 106; i++) await attempt(() => import(`@fuzdev/gro/process.ts?${i}`));
for (let i = 0; i < 107; i++) await attempt(() => import(`ext:${i}`));
