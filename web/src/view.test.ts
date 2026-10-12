import { expect, test } from "vitest";
import { fitView, toMm, zoomAt } from "./view";

test("fitView centres the page at the largest scale that fits", () => {
  const v = fitView(100, 50, 440, 240);
  expect(v.scale).toBe(4);
  expect(v.x).toBe(20);
  expect(v.y).toBe(20);
});

test("fitView never returns a scale below 1e-6", () => {
  expect(fitView(100, 50, 0, 0).scale).toBeGreaterThanOrEqual(1e-6);
});

test("zoomAt keeps the point under the cursor fixed", () => {
  const v = fitView(100, 50, 440, 240);
  const z = zoomAt(v, 2, 100, 60);
  expect(z.scale).toBe(8);
  const [ax, ay] = toMm(v, 100, 60);
  const [bx, by] = toMm(z, 100, 60);
  expect(Math.abs(ax - bx)).toBeLessThan(1e-9);
  expect(Math.abs(ay - by)).toBeLessThan(1e-9);
});
