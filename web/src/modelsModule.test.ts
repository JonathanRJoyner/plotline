import { beforeEach, expect, test, vi } from "vitest";

// Stands in for the model module's glue, counting how often it is imported and initialised.
const glue = vi.hoisted(() => ({ imports: 0, init: vi.fn(async () => {}) }));
vi.mock("../pkg-models/plotline_models.js", () => {
  glue.imports++;
  return { default: glue.init, load: vi.fn(), infer: vi.fn(), unload: vi.fn() };
});

beforeEach(() => {
  vi.resetModules();
  vi.restoreAllMocks();
  glue.imports = 0;
  glue.init.mockClear();
  delete globalThis.plotlineModels;
});

test("loadModelsModule refuses without SIMD", async () => {
  vi.spyOn(WebAssembly, "validate").mockReturnValue(false);
  const { loadModelsModule } = await import("./modelsModule");
  await expect(loadModelsModule()).rejects.toThrow("this browser lacks WebAssembly SIMD, which models need");
  expect(glue.imports).toBe(0);
  expect(glue.init).not.toHaveBeenCalled();
  expect(globalThis.plotlineModels).toBeUndefined();
});

test("loadModelsModule loads once", async () => {
  const { loadModelsModule, simdSupported } = await import("./modelsModule");
  expect(simdSupported()).toBe(true); // Node has SIMD, so this checks the probe is a valid module
  await Promise.all([loadModelsModule(), loadModelsModule()]);
  await loadModelsModule();
  expect(glue.imports).toBe(1);
  expect(glue.init).toHaveBeenCalledTimes(1);
  expect(globalThis.plotlineModels?.load).toBeTypeOf("function");
});
