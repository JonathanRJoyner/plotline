// The engine module's way to the model module (crates/models, its `web` feature). Both run in
// the one worker, and web/src/modelsModule.ts puts the model module on globalThis once it has
// loaded, so each call is synchronous. wasm-bindgen copies a tile out of the engine's memory and
// the model module's glue copies it into its own, and the same for the answer on the way back.
function models() {
  const m = globalThis.plotlineModels;
  if (!m) throw new Error("the model module is not loaded");
  return m;
}

export function models_load(id, bytes, side) {
  models().load(id, bytes, side);
}

export function models_infer(id, rgb) {
  return models().infer(id, rgb);
}

export function models_unload(id) {
  models().unload(id);
}
