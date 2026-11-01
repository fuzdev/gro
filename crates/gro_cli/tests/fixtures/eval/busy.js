// a little work, ended: a timeout of a millisecond stops it, and no timeout doesn't
let sum = 0;
for (let i = 0; i < 2e6; i++) sum += i % 7;
export default sum;
