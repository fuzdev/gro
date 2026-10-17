const value = await new Promise((resolve) => setTimeout(() => resolve(7), 5));
console.log('awaited', value);
