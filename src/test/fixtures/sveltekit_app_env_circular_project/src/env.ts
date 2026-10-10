import { defineEnvVars } from '@sveltejs/kit/env';
// @ts-ignore
import '$app/env/public'; // circular - the declarations can't import the module they declare

export const variables = defineEnvVars({ SOME_PUBLIC_VAR: { public: true } });
