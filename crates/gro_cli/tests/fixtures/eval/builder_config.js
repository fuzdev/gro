// declares through the builder, then once more after returning, from a chain
// of promise jobs longer than the driver's await of this function: the driver
// seals the builder first, so the late call throws, and the config catches it
export default async (builder) => {
	builder.add('early');
	let jobs = Promise.resolve();
	for (let i = 0; i < 10; i++) jobs = jobs.then(() => {});
	jobs.then(() => {
		try {
			builder.add('late');
		} catch {
			// swallowed, so only the driver's after_drain can fail the eval
		}
	});
};
