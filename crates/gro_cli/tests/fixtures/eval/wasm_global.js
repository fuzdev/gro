// names WebAssembly as a global, which a jitless eval doesn't have
export default typeof WebAssembly.Module;
