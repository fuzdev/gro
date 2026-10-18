// the off-build-tree check's entry: a stripped module over the web globals
import './web_globals.js';

const stripped: string = 'stripped';
console.log(stripped satisfies string);
