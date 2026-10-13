import { readdirSync, readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { place, STEPS } from "./tour";

test("every tour step points at an element that exists", () => {
  const dir = new URL(".", import.meta.url);
  const source = readdirSync(dir).filter((f) => f.endsWith(".tsx")).map((f) => readFileSync(new URL(f, dir), "utf8")).join("\n");
  for (const s of STEPS) if (s.target) expect(source, s.title).toContain(`data-tour="${s.target}"`);
});

const rect = (left: number, top: number, width: number, height: number) => ({ left, top, right: left + width, bottom: top + height });

test("place puts the card below a target with room under it", () => {
  expect(place(rect(100, 20, 200, 30), 300, 150, 1200, 800)).toEqual({ left: 100, top: 62 });
});

test("place goes beside a target as tall as the window", () => {
  expect(place(rect(0, 0, 300, 800), 300, 150, 1200, 800)).toEqual({ left: 312, top: 8 });
});

test("place keeps the card inside the window", () => {
  expect(place(rect(1100, 20, 90, 30), 300, 150, 1200, 800)).toEqual({ left: 892, top: 62 });
});

test("place centres the card without a target, or over one that leaves no room", () => {
  expect(place(null, 300, 150, 1200, 800)).toEqual({ left: 450, top: 325 });
  expect(place(rect(10, 10, 1180, 780), 300, 150, 1200, 800)).toEqual({ left: 450, top: 325 });
});
