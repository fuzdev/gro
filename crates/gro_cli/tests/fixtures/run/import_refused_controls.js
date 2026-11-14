// a listed specifier writes no control or bidirectional formatting character to stderr
const attempt = async (f) => {
	try {
		await f();
	} catch {}
};
await attempt(() => import('https://x/\r\x1b[2K'));
await attempt(() => import('ext:a\x1b[8m'));
await attempt(() => new Function('s', 'return import(s)')('z\nerror: x\u202e'));
console.log('done');
