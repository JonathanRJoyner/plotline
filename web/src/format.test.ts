import { expect, test } from "vitest";
import { lengthText, pathsText } from "./format";

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
