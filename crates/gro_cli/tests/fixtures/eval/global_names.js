// every global name user code sees, sorted: the global object's own string and symbol keys
export default [
	...Object.getOwnPropertyNames(globalThis),
	...Object.getOwnPropertySymbols(globalThis).map(String)
].sort();
