import { describe, test, assert, expect } from 'vitest';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { GithubPullRequest } from '#lib/github.ts';

const create_pull_request = () => ({
	url: 'https://api.github.com/repos/fuzdev/gro/pulls/1',
	id: 1,
	html_url: 'https://github.com/fuzdev/gro/pull/1',
	number: 1,
	user: { login: 'someone', id: 2 },
	state: 'open'
});

describe('GithubPullRequest schema', () => {
	test('keeps the keys it does not name', () => {
		const parsed = s.parse_or_throw(GithubPullRequest, create_pull_request());
		assert.deepEqual(parsed, create_pull_request());
	});

	test('rejects a pull request without the fields Gro reads', () => {
		const { html_url: _, ...without_html_url } = create_pull_request();
		expect(() => s.parse_or_throw(GithubPullRequest, without_html_url)).toThrow(
			'html_url: expected string, received undefined'
		);
		expect(() =>
			s.parse_or_throw(GithubPullRequest, { ...create_pull_request(), user: {} })
		).toThrow('user.login: expected string, received undefined');
	});
});
