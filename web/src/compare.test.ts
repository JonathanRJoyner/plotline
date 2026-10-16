import { expect, test } from "vitest";
import { atFromX, choice, dividerX, offers } from "./compare";
import type { ResultEvent } from "./events";

const result = (tone: boolean) => ({ type: "result", id: 1, layers: [], tone: tone ? {} : null }) as unknown as ResultEvent;

test("dividerX is a percentage of the width, kept on the canvas", () => {
  expect(dividerX(50, 800)).toBe(400);
  expect(dividerX(0, 800)).toBe(0);
  expect(dividerX(100, 800)).toBe(800);
  expect(dividerX(-5, 800)).toBe(0);
  expect(dividerX(120, 800)).toBe(800);
});

test("atFromX inverts dividerX in half-percent steps and stays between 0 and 100", () => {
  expect(atFromX(400, 800)).toBe(50);
  expect(atFromX(301, 800)).toBe(37.5);
  expect(atFromX(-30, 800)).toBe(0);
  expect(atFromX(900, 800)).toBe(100);
  expect(atFromX(10, 0)).toBe(50);
});

test("the previous drawing is on offer once a run has replaced one, the source once the result has a tone map", () => {
  expect(offers(null, null)).toEqual({ previous: false, source: false });
  expect(offers(null, result(true))).toEqual({ previous: false, source: true });
  expect(offers(result(true), result(false))).toEqual({ previous: true, source: false });
  expect(offers(result(true), result(true))).toEqual({ previous: true, source: true });
});

test("choice keeps the wanted comparison, else takes the other, else none", () => {
  expect(choice("previous", { previous: true, source: true })).toBe("previous");
  expect(choice("source", { previous: true, source: true })).toBe("source");
  expect(choice("previous", { previous: false, source: true })).toBe("source");
  expect(choice("source", { previous: true, source: false })).toBe("previous");
  expect(choice("previous", { previous: false, source: false })).toBeNull();
});
