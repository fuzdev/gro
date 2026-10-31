// patches a primordial the driver captured before this module ran
JSON.stringify = () => 'patched';

export default 1;
