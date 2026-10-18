// every global name user code sees, sorted: the global object's own string and symbol keys
const names = [
	...Object.getOwnPropertyNames(globalThis),
	...Object.getOwnPropertySymbols(globalThis).map(String)
];
console.log(names.sort().join('\n'));
