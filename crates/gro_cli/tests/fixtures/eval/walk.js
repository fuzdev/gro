// a value built in a moment that the serializer walks for far longer than
// any test waits: each array holds the one below it twice, and a value
// reached twice by different paths is written twice, so the walk writes
// 2^40 empty objects
let value = {};
for (let i = 0; i < 40; i++) value = [value, value];

export default value;
