// the value is taken, then the timer's error fails the eval: its delay
// outlasts the evaluation and the export's read by far, so it fires in the
// drain (a timer due sooner can fire before the value is taken, a
// `module_threw`)
setTimeout(() => {
	throw new Error('late');
}, 500);

export default 1;
