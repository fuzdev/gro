setTimeout(() => {
	throw new TypeError('late');
}, 1);
