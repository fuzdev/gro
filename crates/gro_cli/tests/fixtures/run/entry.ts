// type-only TypeScript, which runs with its types blanked
import type { Missing } from './missing.ts';
import type { Shape } from './shapes.mts';
import { area } from './shapes.mts';

interface Named {
	name: string;
}

const identity = <T>(value: T): T => value;
const square = { kind: 'square', size: 3 } satisfies Shape;
const named = { name: 'typed' } as Named;
let missing: Missing | undefined;
console.log(identity<number>(area(square)), named.name, missing);
