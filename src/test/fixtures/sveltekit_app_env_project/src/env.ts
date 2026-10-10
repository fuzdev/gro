import { defineEnvVars } from '@sveltejs/kit/env';

export const variables = defineEnvVars({
	SOME_PUBLIC_VAR: { public: true },
	SOME_PRIVATE_VAR: {},
	SOME_PORT: { schema: (value) => (value === undefined ? 3000 : Number(value)) }
});
