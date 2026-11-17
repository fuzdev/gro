const inner = new Error('unnamed');
inner.name = '';
throw new Error('outer', { cause: inner });
