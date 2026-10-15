import { describe, test, expect } from 'vitest';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { BuildCacheMetadata, BuildOutputEntry } from '#lib/build_cache.ts';

import {
	create_mock_build_cache_metadata,
	create_mock_output_entry
} from './build_cache_test_helpers.ts';

describe('BuildOutputEntry schema', () => {
	test('validates correct output entry', () => {
		expect(() => s.parse_or_throw(BuildOutputEntry, create_mock_output_entry())).not.toThrow();
	});

	test('rejects entry with missing hash', () => {
		expect(() =>
			s.parse_or_throw(BuildOutputEntry, {
				path: 'build/file.js',
				size: 1024,
				mtime: 1729512000000,
				ctime: 1729512000000,
				mode: 33188
			})
		).toThrow();
	});

	test('rejects entry with wrong size type', () => {
		expect(() =>
			s.parse_or_throw(BuildOutputEntry, {
				path: 'build/file.js',
				hash: 'abc',
				size: '1024', // should be number
				mtime: 1729512000000,
				ctime: 1729512000000,
				mode: 33188
			})
		).toThrow();
	});

	test('rejects entry with extra fields', () => {
		expect(() =>
			s.parse_or_throw(BuildOutputEntry, {
				...create_mock_output_entry(),
				extra: 'bad'
			})
		).toThrow();
	});
});

describe('BuildCacheMetadata schema', () => {
	test('validates correct metadata structure', () => {
		const metadata = create_mock_build_cache_metadata({
			outputs: [create_mock_output_entry('file.js')]
		});
		expect(() => s.parse_or_throw(BuildCacheMetadata, metadata)).not.toThrow();
	});

	test('rejects metadata with missing fields', () => {
		expect(() =>
			s.parse_or_throw(BuildCacheMetadata, {
				version: '1',
				git_commit: 'abc123',
				// missing build_cache_config_hash
				timestamp: '2025-10-23T12:00:00Z',
				outputs: []
			})
		).toThrow();
	});

	test('rejects metadata with wrong types', () => {
		expect(() =>
			s.parse_or_throw(BuildCacheMetadata, {
				version: 1, // should be string
				git_commit: 'abc123',
				build_cache_config_hash: 'hash',
				timestamp: '2025-10-23T12:00:00Z',
				outputs: []
			})
		).toThrow();
	});

	test('accepts a null git_commit and rejects a missing one', () => {
		const metadata = create_mock_build_cache_metadata({ git_commit: null });
		expect(() => s.parse_or_throw(BuildCacheMetadata, metadata)).not.toThrow();
		const { git_commit: _, ...without_commit } = metadata;
		expect(() => s.parse_or_throw(BuildCacheMetadata, without_commit)).toThrow(
			'git_commit: expected string, received undefined'
		);
	});

	test('rejects metadata with unexpected extra fields', () => {
		expect(() =>
			s.parse_or_throw(BuildCacheMetadata, {
				version: '1',
				git_commit: 'abc',
				build_cache_config_hash: 'hash',
				timestamp: '2025-10-23T12:00:00Z',
				outputs: [],
				unexpected_field: 'bad'
			})
		).toThrow();
	});
});
