// Turns one stage's JSON Schema (schemars 1.x: definitions under `$defs`) into the
// form controls the settings panel renders.

type Json = Record<string, unknown>;
type Base = { key: string; label: string; help: string; advanced: boolean };
type Option = { value: string; label: string; title: string | null };

export type Control =
  | (Base & { kind: "checkbox"; default: boolean })
  | (Base & { kind: "slider"; min: number; max: number; step: number; integer: boolean; unit: string | null; default: number })
  | (Base & { kind: "number"; integer: boolean; unit: string | null; default: number | null })
  | (Base & { kind: "select"; options: Option[]; default: string | null })
  | (Base & { kind: "text"; default: string | null });

const REF = /^#\/(\$defs|definitions)\/(.+)$/;

const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
const num = (v: unknown): number | null => (typeof v === "number" ? v : null);

// The definition a property's `$ref` (or the one in a one-element `allOf`) points at.
function referenced(prop: Json, root: Json): Json | null {
  const wrapped = Array.isArray(prop.allOf) && prop.allOf.length === 1 ? (prop.allOf[0] as Json).$ref : undefined;
  const m = REF.exec(str(prop.$ref) ?? str(wrapped) ?? "");
  if (!m) return null;
  const table = root[m[1]] as Json | undefined;
  return (table?.[m[2]] as Json | undefined) ?? null;
}

// Options from an `enum`, or from a `oneOf` of `const` entries; null when neither. A value shows
// with its underscores as spaces.
function optionsOf(schema: Json): Option[] | null {
  const label = (v: unknown) => String(v).replaceAll("_", " ");
  if (Array.isArray(schema.enum)) {
    return schema.enum.map((v) => ({ value: String(v), label: label(v), title: null }));
  }
  const entries = schema.oneOf;
  if (Array.isArray(entries) && entries.every((e: Json) => "const" in e)) {
    return entries.map((e: Json) => ({ value: String(e.const), label: label(e.const), title: str(e.description) }));
  }
  return null;
}

// A key as a label: a last word that is the unit drops off (the input shows the unit), first letter up.
function labelOf(key: string, unit: string | null): string {
  const words = key.split("_");
  if (words.length > 1 && words.at(-1) === unit?.replace("²", "2")) words.pop();
  const text = words.join(" ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function controlFor(key: string, prop: Json, root: Json): Control {
  const unit = str(prop["x-unit"]);
  const base: Base = {
    key,
    label: str(prop["x-label"]) ?? labelOf(key, unit),
    help: str(prop.description) ?? "",
    advanced: prop["x-advanced"] === true,
  };
  const def = prop.default;

  const target = referenced(prop, root);
  const options = optionsOf(prop) ?? (target && optionsOf(target));
  if (options) return { ...base, kind: "select", options, default: def == null ? null : String(def) };

  if (prop.type === "boolean") return { ...base, kind: "checkbox", default: def === true };

  if (prop.type === "integer" || prop.type === "number") {
    const integer = prop.type === "integer";
    const min = num(prop.minimum);
    const max = num(prop.maximum);
    if (min !== null && max !== null) {
      const step = num(prop["x-step"]) ?? (integer ? 1 : (max - min) / 100);
      return { ...base, kind: "slider", min, max, step, integer, unit, default: num(def) ?? min };
    }
    return { ...base, kind: "number", integer, unit, default: num(def) };
  }

  return { ...base, kind: "text", default: str(def) };
}

export function controlsFor(schema: Record<string, unknown>): Control[] {
  const props = (schema.properties ?? {}) as Record<string, Json>;
  return Object.entries(props).map(([key, prop]) => controlFor(key, prop, schema));
}

// The stages every style shares, in the order the form shows them after the style's own, each
// with the label it is shown under: tone is where the image is prepared.
const SHARED: [string, string | null][] = [["order", null], ["tone", "Image"]];

// The stage that shares the drawing between the pens; the app shows it in the Pens panel.
const PENS = "split";

// A style's own stages in run order, then the shared ones, so the settings that make the style
// what it is come first. The pen use stage is set apart for the Pens panel.
export function splitStages<S extends { name: string; label: string }>(stages: S[]): { own: S[]; shared: S[]; pens: S[] } {
  const own = stages.filter((s) => s.name !== PENS && !SHARED.some(([name]) => name === s.name));
  const shared = SHARED.flatMap(([name, label]) => stages.filter((s) => s.name === name).map((s) => ({ ...s, label: label ?? s.label })));
  return { own, shared, pens: stages.filter((s) => s.name === PENS) };
}
