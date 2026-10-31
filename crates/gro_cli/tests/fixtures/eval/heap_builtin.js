// one builtin call that fills past the heap's cap, which no terminate stops
export default new Array(5e7).fill({}).length;
