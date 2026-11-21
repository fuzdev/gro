import { output_case } from './stdio_output.js';

/**
 * `stderr` ([`output_case`]), `stdout`'s handle the other stream's.
 * @param {typeof import('../../../../../../src/lib/stderr.ts')} ns
 * @param {Array<string>} _rest
 * @param {(value: unknown) => void} _print
 * @param {(module: string) => Promise<any>} load
 */
export default async (ns, _rest, _print, load) =>
	output_case(ns, 'stderr', (await load('stdout')).stdout);
