// TODO if this grows at all, use `@octokit/request`,
// for now it's just calling a single endpoint so we do it manually
// and we specify just the types we need

import { fetch_value, type FetchValueCache } from '@fuzdev/fuz_util/fetch.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';

export const GITHUB_REPO_MATCHER = /.+github.com\/(.+)\/(.+)/;

// loose: GitHub's pull request objects carry many more fields, which pass through as they are
export const GithubPullRequest = s.loose_object({
	url: s.string,
	id: s.number,
	html_url: s.string,
	number: s.number,
	user: s.loose_object({
		login: s.string
	})
});
export type GithubPullRequest = s.Infer<typeof GithubPullRequest>;

/**
 * @see https://docs.github.com/en/rest/commits/commits?apiVersion=2022-11-28#list-pull-requests-associated-with-a-commit
 */
export const github_fetch_commit_prs = async (
	owner: string,
	repo: string,
	commit_sha: string,
	token?: string,
	log?: Logger,
	cache?: FetchValueCache,
	api_version?: string
): Promise<Array<GithubPullRequest> | null> => {
	const headers = api_version ? new Headers({ 'x-github-api-version': api_version }) : undefined;
	const url = `https://api.github.com/repos/${owner}/${repo}/commits/${commit_sha}/pulls`;
	const fetched = await fetch_value(url, {
		request: { headers },
		parse: (v: Array<any>) => v.map((p) => s.parse_or_throw(GithubPullRequest, p)),
		token,
		cache,
		return_early_from_cache: true,
		log
	});
	if (!fetched.ok) return null;
	return fetched.value;
};
