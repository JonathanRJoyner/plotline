import { expect, test } from "vitest";
import type { Optimum, Trial } from "./events";
import { lengthText, pathsText, unscoredText } from "./format";

test("a count of paths agrees in number and groups its thousands", () => {
  expect(pathsText(0)).toBe("0 paths");
  expect(pathsText(1)).toBe("1 path");
  expect(pathsText(2)).toBe("2 paths");
  expect(pathsText(999)).toBe("999 paths");
  expect(pathsText(1234)).toBe("1,234 paths");
  expect(pathsText(1234567)).toBe("1,234,567 paths");
});

test("a length reads in millimetres below a metre and in metres from there", () => {
  expect(lengthText(0)).toBe("0.0 mm");
  expect(lengthText(12.345)).toBe("12.3 mm");
  expect(lengthText(999.9)).toBe("999.9 mm");
  expect(lengthText(1000)).toBe("1.00 m");
  expect(lengthText(44190.4)).toBe("44.19 m");
});

test("a length that rounds up to a metre reads in metres", () => {
  expect(lengthText(999.96)).toBe("1.00 m");
});

const trial = (value: number, t: Partial<Trial> = {}): Trial => ({ value, error: null, minutes: 0, metric: null, over: false, slow: false, warnings: [], failed: null, ...t });
const search = (trials: Trial[]): Optimum => ({
  stage: "hatch", key: "spacing_mm", value: 1, error: null, minutes: 0, current: 1, current_error: null, current_minutes: 0,
  budget: null, limit_minutes: 60, beyond: null, trials, reason: "", patch: null,
});
const mm = (v: number) => `${v} mm`;

test("a search with no score says why, as the engine does", () => {
  const values = [1, 0.25, 4, 2];
  expect(unscoredText(search(values.map((v) => trial(v))), "Spacing", mm)).toBe(
    "No value of Spacing from 0.25 mm to 4 mm draws anything here, so there is nothing to compare. Try a lower threshold, or the tone's black, white and gamma settings.",
  );
  expect(unscoredText(search(values.map((v) => trial(v, { minutes: 3 }))), "Spacing", mm)).toBe(
    "No drawing of Spacing from 0.25 mm to 4 mm follows the image's tones enough for a tone error, so there is no value to suggest; judge this style by eye.",
  );
  const left = [trial(1, { error: 0.1, minutes: 3, over: true }), trial(2, { error: 0.1, minutes: 9, over: true, slow: true }), trial(4, { failed: "too many dots", minutes: null })];
  expect(unscoredText(search(left), "Spacing", mm)).toBe("No value of Spacing from 1 mm to 4 mm can be suggested: of 3 trials, 2 over the target, 1 too slow, 1 failed.");
});
