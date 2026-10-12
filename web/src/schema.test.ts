import { expect, test } from "vitest";
import { controlsFor, splitStages } from "./schema";

const properties = {
  invert: { type: "boolean", default: false, description: "Swap light and dark." },
  spacing_mm: { type: "number", minimum: 0.2, maximum: 20, default: 2, "x-step": 0.1, "x-unit": "mm", "x-label": "Spacing" },
  resolution: { type: "integer", minimum: 64, maximum: 4000, default: 1000 },
  seed: { type: "integer", default: 0, "x-advanced": true },
  shape: { $ref: "#/$defs/Shape", default: "point" },
  note: { type: "string" },
};
const $defs = {
  Shape: { oneOf: [{ type: "string", const: "point", description: "One vertex." }, { type: "string", const: "circle" }] },
};
const schema = { $defs, properties };

const shapeOptions = [
  { value: "point", label: "point", title: "One vertex." },
  { value: "circle", label: "circle", title: null },
];

test("maps each property to a control, in order", () => {
  const controls = controlsFor(schema);
  expect(controls.map((c) => c.key)).toEqual(["invert", "spacing_mm", "resolution", "seed", "shape", "note"]);
  expect(controls).toHaveLength(6);

  expect(controls[0]).toMatchObject({ kind: "checkbox", default: false, help: "Swap light and dark.", advanced: false });
  expect(controls[1]).toMatchObject({ kind: "slider", label: "Spacing", step: 0.1, unit: "mm", integer: false, min: 0.2, max: 20, default: 2 });
  expect(controls[2]).toMatchObject({ kind: "slider", step: 1, integer: true, label: "resolution", help: "", unit: null });
  expect(controls[3]).toMatchObject({ kind: "number", advanced: true, integer: true, default: 0 });
  expect(controls[4]).toMatchObject({ kind: "select", options: shapeOptions, default: "point" });
  expect(controls[5]).toMatchObject({ kind: "text", default: null });
});

test("a $ref wrapped in a one-element allOf is the same select", () => {
  const wrapped = { $defs, properties: { shape: { allOf: [{ $ref: "#/$defs/Shape" }], default: "point" } } };
  expect(controlsFor(wrapped)[0]).toMatchObject({ kind: "select", options: shapeOptions, default: "point" });
});

test("definitions, enum and a default step", () => {
  const s = {
    definitions: { Mode: { enum: ["a", "b"] } },
    properties: {
      mode: { $ref: "#/definitions/Mode" },
      kind: { enum: ["x"], description: "Pick." },
      ratio: { type: "number", minimum: 0, maximum: 1 },
      gain: { type: "number" },
      missing: { $ref: "#/$defs/Nope" },
    },
  };
  const c = controlsFor(s);
  expect(c[0]).toMatchObject({ kind: "select", default: null, options: [{ value: "a", label: "a", title: null }, { value: "b", label: "b", title: null }] });
  expect(c[1]).toMatchObject({ kind: "select", options: [{ value: "x", label: "x", title: null }] });
  expect(c[2]).toMatchObject({ kind: "slider", step: 0.01 });
  expect(c[3]).toMatchObject({ kind: "number", integer: false, default: null });
  expect(c[4]).toMatchObject({ kind: "text" });
  expect(controlsFor({})).toEqual([]);
});

test("splitStages puts the style's own stages first, then order, then tone as Image", () => {
  const st = (name: string) => ({ name, label: name.toUpperCase(), schema: {} });
  const { own, shared } = splitStages([st("tone"), st("points"), st("render"), st("order")]);
  expect(own.map((s) => s.name)).toEqual(["points", "render"]);
  expect(shared.map((s) => [s.name, s.label])).toEqual([["order", "ORDER"], ["tone", "Image"]]);
  expect(splitStages([st("a")])).toEqual({ own: [st("a")], shared: [] });
});
