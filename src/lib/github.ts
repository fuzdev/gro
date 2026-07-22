// TODO if this grows at all, use `@octokit/request`,
// for now it's just calling a single endpoint so we do it manually
// and we specify just the types we need

import { FetchValueCache, fetch_value } from '@fuzdev/fuz_util/fetch.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { z } from 'zod';

export const GITHUB_REPO_MATCHER = /.+github.com\/(.+)\/(.+)/;

export const GithubPullRequest = z.looseObject({
	url: z.string(),
	id: z.number(),
	html_url: z.string(),
	number: z.number(),
	user: z.looseObject({
		login: z.string()
	})
});
export type GithubPullRequest = z.infer<typeof GithubPullRequest>;

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
		parse: (v: Array<any>) => v.map((p) => GithubPullRequest.parse(p)),
		token,
		cache,
		return_early_from_cache: true,
		log
	});
	if (!fetched.ok) return null;
	return fetched.value;
};
