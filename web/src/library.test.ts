import fc from "fast-check";
import { afterEach, expect, test, vi } from "vitest";
import type { Pen } from "./events";
import { addPen, libraryFile, loadLibrary, mergeLibraries, parseLibrary, removePen, samePen, saveLibrary, usable } from "./library";

const red: Pen = { name: "Fine red", width_mm: 0.25, color: "#ff0000" };
const blue: Pen = { name: "Blue", width_mm: 0.5, color: "#0000ff" };
const file = (pens: unknown[], extra: Record<string, unknown> = {}) => ({ format: "plotline-pens", version: 1, pens, ...extra });

test("a library file reads back", () => {
  expect(parseLibrary(JSON.parse(libraryFile([red, blue])))).toEqual([red, blue]);
  expect(parseLibrary(file([]))).toEqual([]);
  // Fields the file has beyond a pen's three are left out.
  expect(parseLibrary(file([{ ...red, note: "x" }]))).toEqual([red]);
});

test("a file that is not a pen library is refused in plain words", () => {
  const refused = (value: unknown) => {
    try {
      parseLibrary(value);
    } catch (e) {
      return (e as Error).message;
    }
    throw new Error("not refused");
  };
  const format = "not a pen library: its format must be 'plotline-pens'";
  expect(refused(null)).toBe(format);
  expect(refused([red])).toBe(format);
  expect(refused({ format: "other", version: 1, pens: [] })).toBe(format);
  expect(refused(file([], { version: 2 }))).toBe("unsupported pen library version 2");
  expect(refused(file([], { pens: "x" }))).toBe("not a pen library: it has no list of pens");
  expect(refused(file([red, { ...red, name: "  " }]))).toBe("pen 2 has no name");
  expect(refused(file([null]))).toBe("pen 1 has no name");
  for (const width_mm of [0, -1, "0.3", null, Infinity]) {
    expect(refused(file([red, blue, { ...red, width_mm }]))).toBe("pen 3's line width must be a number above 0");
  }
  for (const color of ["red", "#fff", "#ff00000", "ff0000", 5]) {
    expect(refused(file([{ ...red, color }]))).toBe("pen 1's colour must be written as #rrggbb");
  }
});

test("pens are the same when they differ only in the case of name or colour", () => {
  expect(samePen(red, { ...red, name: "FINE RED", color: "#FF0000" })).toBe(true);
  expect(samePen(red, { ...red, width_mm: 0.3 })).toBe(false);
  expect(samePen(red, blue)).toBe(false);
});

test("a pen is added once, removed by place, and merged in order", () => {
  expect(addPen([red], blue)).toEqual([red, blue]);
  expect(addPen([red], { ...red, color: "#FF0000" })).toEqual([red]);
  expect(removePen([red, blue], 0)).toEqual([blue]);
  expect(mergeLibraries([red], [blue, red, blue])).toEqual([red, blue]);
  expect(usable(red)).toBe(true);
  expect(usable({ ...red, name: "" })).toBe(false);
});

const hex = fc.array(fc.constantFrom(..."0123456789abcdefABCDEF"), { minLength: 6, maxLength: 6 }).map((d) => `#${d.join("")}`);
const pen = fc.record({
  name: fc.string({ minLength: 1 }).filter((s) => s.trim() !== ""),
  width_mm: fc.double({ min: 0, minExcluded: true, noNaN: true, noDefaultInfinity: true }),
  color: hex,
});
const library = fc.array(pen, { maxLength: 8 });

test("a library file always reads back as the library it was written from", () => {
  fc.assert(fc.property(library, (lib) => { expect(parseLibrary(JSON.parse(libraryFile(lib)))).toEqual(lib); }));
});

test("adding a pen twice is adding it once", () => {
  fc.assert(fc.property(library, pen, (lib, p) => { expect(addPen(addPen(lib, p), p)).toEqual(addPen(lib, p)); }));
});

test("merging a library with itself changes nothing", () => {
  fc.assert(fc.property(library, (lib) => { expect(mergeLibraries(lib, lib)).toEqual(lib); }));
});

test("merging drops no pen of either side", () => {
  fc.assert(fc.property(library, library, (a, b) => {
    const merged = mergeLibraries(a, b);
    for (const p of [...a, ...b]) expect(merged.some((q) => samePen(p, q))).toBe(true);
  }));
});

// Storage the test can break: the library is a convenience, so storage failing is not an error.
afterEach(() => vi.unstubAllGlobals());

test("the library is kept in storage, and unreadable storage is an empty library", () => {
  const data = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) });
  expect(loadLibrary()).toEqual([]);
  saveLibrary([red, blue]);
  expect(loadLibrary()).toEqual([red, blue]);
  data.set("plotline.pens", "not json");
  expect(loadLibrary()).toEqual([]);
  data.set("plotline.pens", JSON.stringify(file([{ name: "" }])));
  expect(loadLibrary()).toEqual([]);
  vi.stubGlobal("localStorage", { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("full"); } });
  expect(loadLibrary()).toEqual([]);
  expect(() => saveLibrary([red])).not.toThrow();
});
