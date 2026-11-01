// says it's running, then never returns, so a test can read the process while it runs
export default () => {
	console.error('ready');
	for (;;) {}
};
