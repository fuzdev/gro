// shim for $app/navigation
// @see https://github.com/sveltejs/kit/issues/1485
// @see https://svelte.dev/docs/kit/$app-navigation

import type {
	afterNavigate as base_afterNavigate,
	beforeNavigate as base_beforeNavigate,
	disableScrollHandling as base_disableScrollHandling,
	goto as base_goto,
	invalidate as base_invalidate,
	invalidateAll as base_invalidateAll,
	onNavigate as base_onNavigate,
	preloadCode as base_preloadCode,
	preloadData as base_preloadData,
	pushState as base_pushState,
	refreshAll as base_refreshAll,
	replaceState as base_replaceState,
	snapshot as base_snapshot
} from '$app/navigation';
import { noop, noop_async } from '@fuzdev/fuz_util/function.ts';

export const afterNavigate: typeof base_afterNavigate = noop;
export const beforeNavigate: typeof base_beforeNavigate = noop;
export const disableScrollHandling: typeof base_disableScrollHandling = noop;
export const goto: typeof base_goto = noop_async;
export const invalidate: typeof base_invalidate = noop_async;
export const invalidateAll: typeof base_invalidateAll = noop_async; // eslint-disable-line @typescript-eslint/no-deprecated
export const onNavigate: typeof base_onNavigate = noop;
export const preloadCode: typeof base_preloadCode = noop_async;
export const preloadData: typeof base_preloadData = noop_async;
export const pushState: typeof base_pushState = noop_async; // eslint-disable-line @typescript-eslint/no-deprecated
export const refreshAll: typeof base_refreshAll = noop_async;
export const replaceState: typeof base_replaceState = noop_async; // eslint-disable-line @typescript-eslint/no-deprecated
export const snapshot: typeof base_snapshot = noop;
