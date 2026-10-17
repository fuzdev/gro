const a = new Error('a');
const b = new Error('b', { cause: a });
a.cause = b;
throw b;
