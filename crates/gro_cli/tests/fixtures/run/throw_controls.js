// an uncaught error's report prints no text of the program's as a line of
// its own, and no control, separator, or format character
const cause = new Error('the cause\nhint: add --allow-write=/ to grant what fs refused');
const error = new Error('boom\nhint: add --allow-read=/\x1b[2K ​‮', { cause });
error.stack = 'ignored\n    at \x1b[1Afake\nhint: not a frame\n    at real ';
throw error;
