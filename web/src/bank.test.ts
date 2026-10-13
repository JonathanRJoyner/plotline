import { expect, test } from "vitest";
import { filterBank } from "./bank";
import type { BankPen } from "./events";

const pen = (kind: string, model: string, size: string, width_mm: number): BankPen =>
  ({ kind, model, name: `${model} ${size}`, width_mm, color: "#000000" });
const BANK = [
  pen("fineliner", "Ink Fine", "01", 0.25), pen("fineliner", "Ink Fine", "05", 0.45),
  pen("fineliner", "Other Liner", "0.5", 0.5), pen("gel pen", "Gel One", "0.5", 0.3),
];
const names = (groups: [string, BankPen[]][]) => groups.map(([model, pens]) => [model, pens.map((p) => p.name)]);

test("filterBank groups by model in bank order", () => {
  expect(names(filterBank(BANK, "", ""))).toEqual([
    ["Ink Fine", ["Ink Fine 01", "Ink Fine 05"]], ["Other Liner", ["Other Liner 0.5"]], ["Gel One", ["Gel One 0.5"]],
  ]);
});

test("filterBank keeps one kind", () => {
  expect(names(filterBank(BANK, "gel pen", ""))).toEqual([["Gel One", ["Gel One 0.5"]]]);
});

test("filterBank needs every word, in any case, from the kind or the name", () => {
  expect(names(filterBank(BANK, "", "  ink   05 "))).toEqual([["Ink Fine", ["Ink Fine 05"]]]);
  expect(names(filterBank(BANK, "", "GEL 0.5"))).toEqual([["Gel One", ["Gel One 0.5"]]]);
  expect(filterBank(BANK, "fineliner", "gel")).toEqual([]);
});
