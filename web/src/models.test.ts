import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { accept, accepted, download, forget, keep, keptModels, restoreModels } from "./models";
import type { CatalogueEntry } from "./protocol";

const hex = async (bytes: Uint8Array) =>
  [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes as BufferSource))].map((b) => b.toString(16).padStart(2, "0")).join("");

const DATA = new Uint8Array(1000).map((_, i) => i % 251);
const entryOf = async (over: Partial<CatalogueEntry> = {}): Promise<CatalogueEntry> => ({
  id: "m-1", label: "Model", description: "", url: "https://models.test/m-1.onnx", sha256: await hex(DATA),
  licence: "MIT", attribution: "By someone", source: "", paper: "", training: "", bytes: DATA.length, overlap: 64, ...over,
});

// A Cache Storage that keeps bodies in a map; `put` is what a test can watch.
function stubStorage() {
  const files = new Map<string, ArrayBuffer>();
  const put = vi.fn(async (key: string, res: Response) => { files.set(key, await res.arrayBuffer()); });
  const cache = {
    put,
    match: async (request: { url: string }) => {
      const body = files.get(new URL(request.url).pathname);
      return body && new Response(body);
    },
    keys: async () => [...files.keys()].map((url) => ({ url: `https://page.test${url}` })),
    delete: async (key: string) => files.delete(key),
  };
  vi.stubGlobal("caches", { open: async () => cache });
  const local = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (k: string) => local.get(k) ?? null, setItem: (k: string, v: string) => void local.set(k, v) });
  return { files, put };
}

const serve = (bytes: Uint8Array) => vi.stubGlobal("fetch", vi.fn(async () => new Response(bytes as BufferSource)));
const copy = () => DATA.buffer.slice(0) as ArrayBuffer;

beforeEach(() => { stubStorage(); });
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

test("download returns the bytes of a file that matches, and reports progress", async () => {
  serve(DATA);
  const seen: number[][] = [];
  const got = await download(await entryOf(), (received, total) => seen.push([received, total]));
  expect(new Uint8Array(got)).toEqual(DATA);
  expect(seen.at(-1)).toEqual([DATA.length, DATA.length]);
});

test("download refuses a wrong hash", async () => {
  const { put } = stubStorage();
  serve(DATA.map((b) => b ^ 1));
  await expect(download(await entryOf())).rejects.toThrow(/checksum/);
  expect(put).not.toHaveBeenCalled();
});

test("download refuses a short file", async () => {
  serve(DATA.slice(0, DATA.length / 2));
  await expect(download(await entryOf())).rejects.toThrow(/size/);
});

test("download refuses a long file", async () => {
  serve(new Uint8Array(DATA.length * 3));
  await expect(download(await entryOf())).rejects.toThrow(/size/);
});

test("download refuses a failed request", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => new Response("no", { status: 404 })));
  await expect(download(await entryOf())).rejects.toThrow(/404/);
});

test("keep reports false when Cache Storage throws", async () => {
  vi.stubGlobal("caches", { open: async () => { throw new Error("blocked"); } });
  await expect(keep("m-1", copy())).resolves.toBe(false);
  await expect(keptModels()).resolves.toEqual([]);
  await expect(forget("m-1")).resolves.toBeUndefined();
});

test("keep reports false without Cache Storage at all", async () => {
  vi.stubGlobal("caches", undefined);
  await expect(keep("m-1", copy())).resolves.toBe(false);
});

test("a kept model comes back after a reload, and forget drops it", async () => {
  await expect(keep("m-1", copy())).resolves.toBe(true);
  await keep("file-0123456789ab", new Uint8Array([1, 2, 3]).buffer);
  const kept = await keptModels();
  expect(kept.map((k) => k.id)).toEqual(["m-1", "file-0123456789ab"]);
  expect(new Uint8Array(kept[0].bytes)).toEqual(DATA);
  await forget("m-1");
  expect((await keptModels()).map((k) => k.id)).toEqual(["file-0123456789ab"]);
});

test("accepted survives a throwing localStorage", async () => {
  vi.stubGlobal("localStorage", { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); } });
  const entry = await entryOf();
  await expect(accepted(entry)).resolves.toBe(false);
  await expect(accept(entry)).resolves.toBeUndefined();
});

test("acceptance is per licence text", async () => {
  const entry = await entryOf();
  await expect(accepted(entry)).resolves.toBe(false);
  await accept(entry);
  await expect(accepted(entry)).resolves.toBe(true);
  await expect(accepted({ ...entry, licence: "MIT, changed" })).resolves.toBe(false);
  await expect(accepted({ ...entry, attribution: "By someone else" })).resolves.toBe(false);
  await expect(accepted({ ...entry, id: "m-2" })).resolves.toBe(false);
  await expect(accepted({ ...entry, label: "Renamed", bytes: 1 })).resolves.toBe(true);
});

const reply = (value: unknown) => ({ value });

test("restore re-adds a withdrawn id as a file", async () => {
  const { files } = stubStorage();
  await keep("gone-1", copy());
  const call = vi.fn(async (method: string) => reply(method === "catalogue" ? [] : { id: "file-aaaaaaaaaaaa" }));
  await restoreModels(call);
  expect(call).toHaveBeenCalledWith("addModel", [null, expect.any(Uint8Array)]);
  // The kept copy is now under the id the model has, so removing it in the GUI really forgets it.
  expect([...files.keys()]).toEqual(["/kept/file-aaaaaaaaaaaa"]);
});

test("restore adds a listed model under its catalogue id", async () => {
  await keep("m-1", copy());
  const call = vi.fn(async (method: string) => reply(method === "catalogue" ? [await entryOf()] : { id: "m-1" }));
  await restoreModels(call);
  expect(call).toHaveBeenCalledWith("addModel", ["m-1", expect.any(Uint8Array)]);
  expect((await keptModels()).map((k) => k.id)).toEqual(["m-1"]);
});

test("restore goes on past a model that fails, and warns instead of throwing", async () => {
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  await keep("a", new Uint8Array([1]).buffer);
  await keep("b", new Uint8Array([2]).buffer);
  const call = vi.fn(async (method: string, args?: unknown[]) => {
    if (method === "catalogue") return reply([{ id: "a" }, { id: "b" }]);
    if (args![0] === "a") throw new Error("bad file");
    return reply({ id: "b" });
  });
  await expect(restoreModels(call)).resolves.toBeUndefined();
  expect(call.mock.calls.filter(([m]) => m === "addModel").map(([, a]) => a![0])).toEqual(["a", "b"]);
  expect(warn).toHaveBeenCalledTimes(1);
});

test("restore with nothing kept asks the engine for nothing", async () => {
  const call = vi.fn();
  await restoreModels(call);
  expect(call).not.toHaveBeenCalled();
});
