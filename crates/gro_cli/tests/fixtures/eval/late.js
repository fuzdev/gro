// eval has no timers, so work left after the evaluation runs as promise jobs,
// which all run before the value is taken: an error in one fails the
// evaluation, not the drain
let jobs = Promise.resolve();
for (let i = 0; i < 100; i++) jobs = jobs.then(() => {});
jobs.then(() => {
	throw new Error('late');
});

export default 1;
