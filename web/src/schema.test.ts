import { expect, test } from "vitest";
import { controlsFor, splitStages, withModelChoice } from "./schema";

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
  expect(controls[2]).toMatchObject({ kind: "slider", step: 1, integer: true, label: "Resolution", help: "", unit: null });
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

const st = (name: string) => ({ name, label: name.toUpperCase(), schema: {} });

test("splitStages puts the style's own stages first, then order, then tone as Image", () => {
  const { own, shared } = splitStages([st("tone"), st("points"), st("render"), st("order")]);
  expect(own.map((s) => s.name)).toEqual(["points", "render"]);
  expect(shared.map((s) => [s.name, s.label])).toEqual([["order", "ORDER"], ["tone", "Image"]]);
  expect(splitStages([st("a")])).toEqual({ own: [st("a")], shared: [], pens: [] });
});

test("a property marked x-hidden has no control", () => {
  const schema = { properties: { blur: { type: "number" }, focus: { type: "array", default: [], "x-hidden": true }, gamma: { type: "number" } } };
  expect(controlsFor(schema).map((c) => c.key)).toEqual(["blur", "gamma"]);
});

test("x-order puts the listed keys first, in its order, and the rest after in schema order", () => {
  const props = { a: { type: "number" }, b: { type: "number" }, c: { type: "number" }, d: { type: "number" }, h: { type: "number", "x-hidden": true } };
  const keys = (schema: Record<string, unknown>) => controlsFor(schema).map((c) => c.key);
  expect(keys({ properties: props, "x-order": ["d", "gone", "b"] })).toEqual(["d", "b", "a", "c"]);
  expect(keys({ properties: props, "x-order": ["h", "c"] })).toEqual(["c", "a", "b", "d"]);
  expect(keys({ properties: props })).toEqual(["a", "b", "c", "d"]);
});

test("underscores in enum values read as spaces", () => {
  const [c] = controlsFor({ properties: { mode: { enum: ["one_pen", "by_colour"], default: "one_pen" } } });
  expect(c.kind === "select" && c.options.map((o) => o.label)).toEqual(["one pen", "by colour"]);
});

test("a key's label drops a trailing unit word and starts with a capital", () => {
  const labels = controlsFor({
    properties: {
      blur_mm: { type: "number", "x-unit": "mm" },
      min_area_mm2: { type: "number", "x-unit": "mm²" },
      clip_percent: { type: "number" },
      count: { type: "integer" },
      angle_deg: { type: "number", "x-unit": "deg", "x-label": "Tilt_deg" },
    },
  }).map((c) => c.label);
  expect(labels).toEqual(["Blur", "Min area", "Clip percent", "Count", "Tilt_deg"]);
});

test("splitStages sets the pen use stage apart", () => {
  const { own, shared, pens } = splitStages([st("tone"), st("split"), st("hatch"), st("order")]);
  expect(own.map((s) => s.name)).toEqual(["hatch"]);
  expect(shared.map((s) => s.name)).toEqual(["order", "tone"]);
  expect(pens.map((s) => s.name)).toEqual(["split"]);
});

const modelStage = { name: "model", label: "Model", schema: { properties: { model: { type: "string", default: "" }, invert: { type: "boolean" } } } };
const loaded = [{ id: "file-0123456789ab", label: "file-0123456789ab" }, { id: "teed-1", label: "Edges (TEED)" }];

test("the model setting is a choice of the built-in model, then the loaded ones, by label", () => {
  const [stage] = withModelChoice([modelStage], loaded, "teed-1");
  const control = controlsFor(stage.schema)[0];
  expect(control).toMatchObject({ kind: "select", default: "" });
  expect(control.kind === "select" && control.options.map((o) => [o.value, o.label])).toEqual([["xdog", "Lines (XDoG)"], ["file-0123456789ab", "file-0123456789ab"], ["teed-1", "Edges (TEED)"]]);
  expect(controlsFor(stage.schema)[1]).toMatchObject({ kind: "checkbox" });
});

test("a model setting none of the choices has stays a choice of its own", () => {
  const options = (current: string) => {
    const control = controlsFor(withModelChoice([modelStage], loaded.slice(1), current)[0].schema)[0];
    return control.kind === "select" ? control.options.map((o) => [o.value, o.label]) : null;
  };
  const built = ["xdog", "Lines (XDoG)"];
  expect(options("xdog")).toEqual([built, ["teed-1", "Edges (TEED)"]]);
  expect(options("")).toEqual([["", "None"], built, ["teed-1", "Edges (TEED)"]]);
  expect(options("contour-1")).toEqual([["contour-1", "contour-1 (not loaded)"], built, ["teed-1", "Edges (TEED)"]]);
});

test("stages other than the model stage are left alone", () => {
  const other = { name: "tone", label: "Image", schema: { properties: { model: { type: "string" } } } };
  expect(withModelChoice([other], loaded, "")).toEqual([other]);
});
