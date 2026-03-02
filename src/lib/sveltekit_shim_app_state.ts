// shim for $app/state
// @see https://github.com/sveltejs/kit/issues/1485
// @see https://svelte.dev/docs/kit/$app-state

import type {
	navigating as base_navigating,
	page as base_page,
	updated as base_updated,
} from '$app/state';

export const navigating: typeof base_navigating = {
	from: null,
	to: null,
	type: null,
	willUnload: null,
	delta: null,
	complete: null,
};

export const page: typeof base_page = {
	data: {},
	form: null,
	error: null,
	params: {},
	route: {id: null},
	state: {},
	status: -1,
	url: new URL('https://github.com/fuzdev/gro') as any, // TODO maybe use package.json?
};

export const updated: typeof base_updated = {
	current: false,
	check: () => {
		throw Error('Can only call updated.check() in the browser');
	},
};
