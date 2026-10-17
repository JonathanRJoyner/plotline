import { expect, test } from "vitest";
import { trace } from "./preview";

test("a one-vertex path is a filled disc and a longer one a line", () => {
  // A desktop browser's canvas drew no round cap on a near-zero stroke under a pixel wide, so a
  // dot style's page went blank at overview zooms; a disc is filled at any size.
  const calls: string[] = [];
  const record = (name: string) =>
    new Proxy({}, { get: (_, m) => (...a: number[]) => calls.push(`${name}.${String(m)}(${a.join(", ")})`) }) as unknown as Path2D;
  const chunk = { lines: record("lines"), dots: record("dots"), r: 0.15 };
  const c = new Float64Array([1, 2, 3, 4, 5, 6]);
  trace(chunk, c, 0, 1);
  trace(chunk, c, 1, 3);
  expect(calls).toEqual([
    "dots.moveTo(1.15, 2)",
    `dots.arc(1, 2, 0.15, 0, ${2 * Math.PI})`,
    "lines.moveTo(3, 4)",
    "lines.lineTo(5, 6)",
  ]);
});
