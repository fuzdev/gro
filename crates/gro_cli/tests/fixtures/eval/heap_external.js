// memory outside V8's heap: an array buffer's bytes aren't counted against
// the heap's cap, so one twice the cap is made (its pages untouched, so not
// resident) and the eval succeeds
export default new ArrayBuffer(2 ** 30).byteLength;
