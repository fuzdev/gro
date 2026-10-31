// a value inside the depth cap whose deepest level allocates enough to start
// a garbage collection while the walk is near its deepest
const leaves = [];
for (let i = 0; i < 1e5; i++) leaves.push({});
let value = leaves;
for (let i = 0; i < 995; i++) value = [value];

export default value;
