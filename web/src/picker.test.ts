import fc from "fast-check";
import { readFileSync } from "node:fs";
import { beforeAll, expect, test } from "vitest";
import init, { listStyles } from "../pkg/plotline_wasm.js";
import { FAMILIES, groupStyles, MORE } from "./picker";

beforeAll(async () => { await init({ module_or_path: readFileSync(new URL("../pkg/plotline_wasm_bg.wasm", import.meta.url)) }); });

const known = FAMILIES.flatMap(([, names]) => names);
const labels = [...FAMILIES.map(([label]) => label), MORE];

test("grouping lists every entry once, families first in table order, unknown ones last in input order", () => {
  const name = fc.oneof(fc.constantFrom(...known), fc.string({ minLength: 1 }));
  fc.assert(fc.property(fc.uniqueArray(name), (names) => {
    const groups = groupStyles(names.map((n) => ({ name: n })));
    const out = groups.flatMap(([, es]) => es.map((e) => e.name));
    expect([...out].sort()).toEqual([...names].sort());
    expect(groups.every(([, es]) => es.length > 0)).toBe(true);
    const order = groups.map(([label]) => labels.indexOf(label));
    expect(order.every((i, j) => i >= 0 && (j === 0 || i > order[j - 1]))).toBe(true);
    for (const [label, es] of groups) {
      const got = es.map((e) => e.name);
      const family = FAMILIES.find(([l]) => l === label);
      if (family) expect(got).toEqual(family[1].filter((n) => names.includes(n)));
      else expect(got).toEqual(names.filter((n) => !known.includes(n)));
    }
  }));
});

test("the families match the style guide's contents list", () => {
  const guide = readFileSync(new URL("../../docs/styles.md", import.meta.url), "utf8");
  const contents = [...guide.matchAll(/^- \[([^\]]+)\]\(#[^)]+\):(.*)$/gm)]
    .map(([, label, rest]) => [label, [...rest.matchAll(/`([^`]+)`/g)].map(([, n]) => n)]);
  expect(contents).toEqual(FAMILIES.map(([label, names]) => [label, [...names]]));
});

test("every style the engine lists has a family", () => {
  const names = (JSON.parse(listStyles()) as Array<{ name: string }>).map((s) => s.name);
  expect(names.filter((n) => !known.includes(n))).toEqual([]);
});
