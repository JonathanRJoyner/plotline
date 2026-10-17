import fc from "fast-check";
import { readFileSync } from "node:fs";
import { beforeAll, expect, test } from "vitest";
import init, { listStyles } from "../pkg/plotline_wasm.js";
import { FAMILIES, groupStyles, MORE, withModels } from "./picker";
import type { CatalogueEntry } from "./protocol";

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

const style = (name: string) => ({ name, label: name, description: `${name} draws.` });
const teed = { id: "teed-1", label: "Edges", description: "Finds edges.", licence: "MIT" } as CatalogueEntry;

test("without a model, learned is the built-in xdog entry, drawn from the trial and with nothing to download", () => {
  const entries = withModels([style("grid"), style("learned"), style("stipple")], []);
  expect(entries.map((e) => [e.name, e.model])).toEqual([["grid", undefined], ["learned", "xdog"], ["stipple", undefined]]);
  expect(entries[1]).toMatchObject({ label: "Lines (XDoG)", note: "built in" });
  expect(entries[1].description).toContain("needs no download");
  expect("picture" in entries[1]).toBe(false); // so the card shows the trial's thumbnail
  expect(entries[0]).toEqual(style("grid"));
});

test("each loaded model is a learned entry of its own after the built-in one, with the card's picture when the catalogue lists it", () => {
  const models = [
    { id: "file-0123456789ab", label: "file-0123456789ab", overlap: 64, entry: null },
    { id: "teed-1", label: "Edges", overlap: 32, entry: teed },
  ];
  const entries = withModels([style("grid"), style("learned"), style("stipple")], models);
  expect(entries.map((e) => [e.name, e.model])).toEqual([["grid", undefined], ["learned", "xdog"], ["learned", "file-0123456789ab"], ["learned", "teed-1"], ["stipple", undefined]]);
  expect(entries[1]).toMatchObject({ label: "Lines (XDoG)", note: "built in" });
  expect(entries[2]).toMatchObject({ label: "file-0123456789ab", note: "your own", picture: null });
  expect(entries[3]).toMatchObject({ label: "Edges", description: "Finds edges.", note: "MIT", picture: "models/teed-1.png" });
  // Grouped, every model stays, in the order the engine lists them.
  const lines = groupStyles(entries).find(([label]) => label === "Lines")![1];
  expect(lines.map((e) => e.model)).toEqual([undefined, "xdog", "file-0123456789ab", "teed-1"]);
});

test("withModels keeps every other style as it is, and gives learned the built-in entry first, then one per model", () => {
  const names = fc.uniqueArray(fc.oneof(fc.constantFrom(...known), fc.string({ minLength: 1 })));
  const ids = fc.uniqueArray(fc.stringMatching(/^[a-z0-9-]{1,12}$/), { maxLength: 4 });
  fc.assert(fc.property(names, ids, (names, ids) => {
    const styles = names.map(style);
    const models = ids.map((id) => ({ id, label: id, overlap: 64, entry: null }));
    const entries = withModels(styles, models);
    expect(entries.filter((e) => e.name !== "learned")).toEqual(styles.filter((s) => s.name !== "learned"));
    const learned = entries.filter((e) => e.name === "learned");
    if (!names.includes("learned")) expect(learned).toEqual([]);
    else expect(learned.map((e) => e.model)).toEqual(["xdog", ...ids]);
    // Only the built-in entry is drawn from the trial; a model's card has a picture of its own, or plain paper.
    expect(learned.slice(1).every((e) => e.picture !== undefined)).toBe(true);
  }));
});
