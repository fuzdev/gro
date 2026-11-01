// an array buffer the size of the process's memory cap (`MEMORY_LIMIT`),
// which the process, holding anything else, can't make: the failed
// allocation stops the run as `memory` (its pages untouched, so not resident
// when the cap is off)
export default new ArrayBuffer(2 ** 31).byteLength;
