// Learned line models in the browser: downloading a catalogue model and checking it, keeping files
// in Cache Storage, remembering accepted licences in localStorage, and adding the kept models to
// the engine at start. Every storage call is wrapped: where storage is blocked the page works,
// without remembering (spec 2026-10-16-learned-line-models, "Downloads" and "In the GUI").
import type { CatalogueEntry, ModelInfo } from "./protocol";

/** An engine call that takes the reply's state, as the page makes it. */
export type Call = (method: string, args?: unknown[]) => Promise<{ value: unknown }>;

const CACHE = "plotline-models";
const MAX_BYTES = 209_715_200; // the core's limit, MAX_BYTES in crates/core/src/models/mod.rs
const KEPT = "/kept/"; // Cache Storage wants a URL for a key; the origin is the page's own and never fetched
const ACCEPTED = "plotline.licence.";

const hex = async (bytes: BufferSource) =>
  [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((b) => b.toString(16).padStart(2, "0")).join("");

/** Every kept model, in the order they were kept; none where storage is blocked. */
export async function keptModels(): Promise<{ id: string; bytes: ArrayBuffer }[]> {
  const kept: { id: string; bytes: ArrayBuffer }[] = [];
  try {
    const cache = await caches.open(CACHE);
    for (const request of await cache.keys()) {
      try {
        const res = await cache.match(request);
        if (res) kept.push({ id: new URL(request.url).pathname.split("/").pop()!, bytes: await res.arrayBuffer() });
      } catch { /* one unreadable file does not hide the others */ }
    }
  } catch { /* storage is blocked */ }
  return kept;
}

/** The ids of the kept models, without reading their files; none where storage is blocked. */
export async function keptIds(): Promise<string[]> {
  try {
    return (await (await caches.open(CACHE)).keys()).map((request) => new URL(request.url).pathname.split("/").pop()!);
  } catch {
    return [];
  }
}

/** Keep a model's file; false when the browser refuses (blocked, or no room). */
export async function keep(id: string, bytes: ArrayBuffer): Promise<boolean> {
  try {
    await (await caches.open(CACHE)).put(KEPT + id, new Response(bytes));
    return true;
  } catch {
    return false;
  }
}

/** Drop a model's kept file. */
export async function forget(id: string): Promise<void> {
  try {
    await (await caches.open(CACHE)).delete(KEPT + id);
  } catch { /* nothing kept, or storage blocked */ }
}

// The key holds the id and the hash of the licence text, as the CLI keeps them, so a changed
// licence, attribution or training note is a different key and asks again. `crypto.subtle` is
// async, so the two functions below are too.
const acceptKey = async (e: CatalogueEntry) => `${ACCEPTED}${e.id}.${await hex(new TextEncoder().encode(`${e.licence}\n${e.attribution}\n${e.training}`))}`;

/** Whether the user accepted this model's current licence, attribution and training note; false where storage is blocked. */
export async function accepted(entry: CatalogueEntry): Promise<boolean> {
  try {
    return localStorage.getItem(await acceptKey(entry)) === "1";
  } catch {
    return false;
  }
}

/** Remember that the user accepted this model's licence, attribution and training note; forgotten where storage is blocked. */
export async function accept(entry: CatalogueEntry): Promise<void> {
  try {
    localStorage.setItem(await acceptKey(entry), "1");
  } catch { /* asked again next time */ }
}

/**
 * Fetch a catalogue model and check its size, then its SHA-256, before returning it; a mismatch
 * rejects with a plain message and nothing is kept, and so does a connection that fails. At most
 * `entry.bytes` plus one chunk is read, so a wrong file at the URL cannot fill memory. The core
 * checks both again when the model is added.
 */
export async function download(entry: CatalogueEntry, onProgress?: (received: number, total: number) => void): Promise<ArrayBuffer> {
  // A browser that cannot reach the server only says "Failed to fetch", which names nothing.
  const offline = (): never => { throw new Error(`could not download ${entry.label}: check your connection and try again`); };
  const res = await fetch(entry.url).catch(offline);
  if (!res.ok || !res.body) throw new Error(`could not download ${entry.label}: the server answered ${res.status}`);
  const reader = res.body.getReader();
  const chunks: Uint8Array[] = [];
  let received = 0;
  for (;;) {
    const { done, value } = await reader.read().catch(offline);
    if (done) break;
    chunks.push(value);
    received += value.length;
    onProgress?.(received, entry.bytes);
    if (received > entry.bytes) {
      await reader.cancel();
      break;
    }
  }
  if (received !== entry.bytes) throw new Error(`${entry.label} has the wrong size (${received} bytes, expected ${entry.bytes}); it was discarded`);
  const bytes = new Uint8Array(received);
  let at = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, at);
    at += chunk.length;
  }
  if ((await hex(bytes)) !== entry.sha256.toLowerCase()) throw new Error(`${entry.label} does not match its checksum; it was discarded`);
  return bytes.buffer;
}

/** Download a catalogue model, add it to the engine, and keep it once the engine has taken it. */
export async function fetchModel(entry: CatalogueEntry, call: Call, onProgress?: (received: number, total: number) => void): Promise<void> {
  const bytes = await download(entry, onProgress);
  await call("addModel", [entry.id, new Uint8Array(bytes)]);
  await keep(entry.id, bytes);
}

/**
 * Add a model file of the user's own to the engine, and keep it under the id the engine gives it.
 * A file over the core's limit is refused, with the core's words, before it is read into memory.
 */
export async function loadModel(file: Blob, call: Call): Promise<ModelInfo> {
  if (file.size > MAX_BYTES) throw new Error(`the model file is ${file.size} bytes, over the 200 MB limit`);
  const bytes = await file.arrayBuffer();
  const info = (await call("addModel", [null, new Uint8Array(bytes)])).value as ModelInfo;
  await keep(info.id, bytes);
  return info;
}

/**
 * Add every kept model to the engine. A model the catalogue no longer lists is added as a file of
 * your own and kept again under the id that gives it, so removing it later really forgets it. A
 * model that fails is warned about and the others go on; this never rejects.
 */
export async function restoreModels(call: Call): Promise<void> {
  try {
    const files = await keptModels();
    if (!files.length) return;
    const listed = new Set(((await call("catalogue")).value as { id: string }[]).map((e) => e.id));
    for (const { id, bytes } of files) {
      try {
        const info = (await call("addModel", [listed.has(id) ? id : null, new Uint8Array(bytes)])).value as { id: string };
        if (info.id !== id && (await keep(info.id, bytes))) await forget(id);
      } catch (e) {
        console.warn(`could not restore the model ${id}: ${e instanceof Error ? e.message : e}`);
      }
    }
  } catch (e) {
    console.warn(`could not restore the kept models: ${e instanceof Error ? e.message : e}`);
  }
}
