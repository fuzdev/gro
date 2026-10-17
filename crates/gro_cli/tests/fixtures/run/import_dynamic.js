const { value } = await import('./sibling.js');
const again = await import('./sibling.js');
console.log('dynamic', value, again.value === value);
