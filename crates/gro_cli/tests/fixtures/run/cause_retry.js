let error;
for (let i = 0; i < 3; i++) error = new Error('retry failed', { cause: error });
throw error;
