// The model module (crates/models built with its `web` feature): tract, which runs learned line
// models, in a WebAssembly module of its own so the engine module stays small. The worker loads
// it before the first model is added, and the engine's bridge (crates/wasm/js/models_bridge.js)
// finds it on globalThis.
type ModelsModule = typeof import("../pkg-models/plotline_models.js");

declare global {
  var plotlineModels: ModelsModule | undefined;
}

// A module of one function, () -> v128, whose body is `i32.const 0; i8x16.splat; end`, encoded
// by the WebAssembly binary format's rules. It validates only where SIMD is supported.
const SIMD_PROBE = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic and version
  0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7b, // type section: one func type () -> v128
  0x03, 0x02, 0x01, 0x00, // function section: one function of type 0
  0x0a, 0x08, 0x01, 0x06, 0x00, 0x41, 0x00, 0xfd, 0x0f, 0x0b, // code section: the body
]);

/** Whether this browser runs WebAssembly SIMD, which the model module is built with. */
export function simdSupported(): boolean {
  return WebAssembly.validate(SIMD_PROBE);
}

let loading: Promise<void> | undefined;

/** Fetch and start the model module once; a failed load is tried again on the next call. */
export function loadModelsModule(): Promise<void> {
  loading ??= (async () => {
    if (!simdSupported()) throw new Error("this browser lacks WebAssembly SIMD, which models need");
    const module = await import("../pkg-models/plotline_models.js");
    await module.default();
    globalThis.plotlineModels = module;
  })().catch((e) => {
    loading = undefined;
    throw e;
  });
  return loading;
}
