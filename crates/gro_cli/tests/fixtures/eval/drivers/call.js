// calls the config's default export, and lists the namespace's exports
export default async (namespace) => ({
	exports: Object.keys(namespace).sort(),
	value: await namespace.default(2)
});
