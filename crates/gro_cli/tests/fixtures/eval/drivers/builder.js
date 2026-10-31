// a builder sealed once the config returns: a later call throws, and the
// host's after_drain call fails the eval even when the config caught it
const items = [];
const late = [];
let sealed = false;

const builder = {
	add(item) {
		if (sealed) {
			late.push(item);
			throw new Error(`add('${item}') was called after the config returned`);
		}
		items.push(item);
	}
};

export default async (namespace) => {
	await namespace.default(builder);
	sealed = true;
	return { items: [...items] };
};

export const after_drain = () => {
	if (late.length > 0) throw new Error(`late builder calls: ${late.join(', ')}`);
};
