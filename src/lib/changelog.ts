import {readFile, writeFile} from 'node:fs/promises';
import {z} from 'zod';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import type {FetchValueCache} from '@fuzdev/fuz_util/fetch.js';

import {github_fetch_commit_prs} from './github.ts';

/**
 * Updates a changelog produced by `@changesets/changelog-git` with better links and formatting.
 * It's similar to `@changesets/changelog-github` but doesn't require a token for light usage.
 * This may be better implemented as a standalone dependency
 * as an alternative to `@changesets/changelog-git`.
 * @returns boolean indicating if the changelog changed
 */
export const update_changelog = async (
	owner: string,
	repo: string,
	path = 'CHANGELOG.md',
	token?: string,
	log?: Logger,
	cache: FetchValueCache = new Map(), // include a default cache to efficiently handle multiple changesets per commit
): Promise<boolean> => {
	const contents = await readFile(path, 'utf8');
	const parsed = parse_changelog(contents);
	const mapped = await map_changelog(parsed, owner, repo, token, log, cache);
	const updated = serialize_changelog(mapped);
	if (contents === updated) {
		return false;
	}
	await writeFile(path, updated, 'utf8');
	return true;
};

// keeping this really simple for now, no need to parse further for our current usecases
const ParsedChangelog = z.array(z.string());
type ParsedChangelog = z.infer<typeof ParsedChangelog>;
const parse_changelog = (contents: string): ParsedChangelog => contents.split('\n');
const serialize_changelog = (parsed: ParsedChangelog): string => parsed.join('\n');

const LINE_WITH_SHA_MATCHER = /^- ([a-z0-9]{7,8}): /;

const map_changelog = async (
	parsed: ParsedChangelog,
	owner: string,
	repo: string,
	token?: string,
	log?: Logger,
	cache?: FetchValueCache,
): Promise<ParsedChangelog> => {
	const mapped: ParsedChangelog = [];
	for (const line of parsed) {
		const matches = LINE_WITH_SHA_MATCHER.exec(line);
		if (matches) {
			const commit_sha = matches[1]!;
			const l = '- ' + line.substring(commit_sha.length + 4);
			const prs = await github_fetch_commit_prs(owner, repo, commit_sha, token, log, cache);
			if (prs?.length) {
				mapped.push(`${l} (${prs.map((p) => `[#${p.number}](${p.html_url})`).join(', ')})`);
			} else {
				mapped.push(
					`${l} ([${commit_sha}](https://github.com/${owner}/${repo}/commit/${commit_sha}))`,
				);
			}
		} else {
			mapped.push(line);
		}
	}
	return mapped;
};
