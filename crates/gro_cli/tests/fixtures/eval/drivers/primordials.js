// captured when the driver evaluates, before the config can patch it
const stringify = JSON.stringify;

export default () => ({
	patched_since: JSON.stringify !== stringify,
	captured: stringify([1])
});
