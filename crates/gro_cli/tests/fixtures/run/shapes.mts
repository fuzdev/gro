export interface Shape {
	kind: 'square';
	size: number;
}

export const area = (shape: Shape): number => shape.size ** 2;
