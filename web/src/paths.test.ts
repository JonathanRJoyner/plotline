import { expect, test } from "vitest";
import { eachPath, pathCount } from "./paths";

test("a dot and a two-vertex path", () => {
  const coords = new Float64Array([1, 2, 3, 4, 5, 6]);
  const offsets = new Uint32Array([0, 1, 3]);
  const seen: [number, number][] = [];
  eachPath(coords, offsets, (s, e) => seen.push([s, e]));
  expect(seen).toEqual([[0, 1], [1, 3]]);
  expect(pathCount(offsets)).toBe(2);
});

test("an empty buffer has no paths", () => {
  for (const offsets of [new Uint32Array([]), new Uint32Array([0])]) {
    const seen: number[] = [];
    eachPath(new Float64Array([]), offsets, (s) => seen.push(s));
    expect(seen).toEqual([]);
    expect(pathCount(offsets)).toBe(0);
  }
});
