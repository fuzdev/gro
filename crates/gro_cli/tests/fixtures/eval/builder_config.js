// declares through the builder, then once more after returning, from a
// timer due well after the driver seals the builder: the late call throws,
// and the config catches it
export default async (builder) => {
	builder.add('early');
	setTimeout(() => {
		try {
			builder.add('late');
		} catch {
			// swallowed, so only the driver's after_drain can fail the eval
		}
	}, 500);
};
