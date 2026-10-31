// data of every kind the serializer writes, with members it leaves out
export default {
	b: 1,
	a: [1, 'two', null, true, false],
	2: 'integer keys come first',
	zero: -0,
	nested: { x: { y: [] } },
	skipped: undefined,
	text: 'quote " backslash \\ newline \n tab \t é 😀'
};

export const named = ['the', 'named', 'export'];

export const missing_value = undefined;
