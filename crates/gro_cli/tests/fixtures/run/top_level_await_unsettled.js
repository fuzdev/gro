// a promise nothing settles: the event loop drains with the module's
// evaluation still waiting on it
await new Promise(() => {});
console.log('settled');
