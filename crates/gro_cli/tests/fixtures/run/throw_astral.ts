// the astral character is two UTF-16 units, so `new` is at column 38
const pair: [string, Error] = ['😀', new Error('astral')];
throw pair[1];
