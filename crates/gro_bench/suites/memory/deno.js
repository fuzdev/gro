// the memory suite's Deno driver, read on stdin: imports the module its
// argument names and writes its default export as JSON
const namespace = await import(`file://${Deno.args[0]}`);
console.log(JSON.stringify(namespace.default));
