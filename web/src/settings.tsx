// The settings form of spec 2026-10-12-gui-design section 6.3: one section per stage, built
// from the stage's schema, with the advanced settings collapsed. Spec 2026-10-12-ink-and-cleanup
// section 5 puts the style's own stages first, open, and the shared ones after them, collapsed.
import type { JSX } from "preact";
import type { State } from "./protocol";
import { controlsFor, splitStages, type Control } from "./schema";

type Stage = { name: string; label: string; schema: Record<string, unknown> };
type Props = {
  stages: Stage[];
  values: State["settings"];
  errors: Record<string, string>;
  help: boolean;
  // "pens" shows only the pen use stage's controls, without a section of their own, for the Pens
  // panel; the default shows all the other stages.
  part?: "style" | "pens";
  onChange: (stage: string, key: string, value: unknown) => void;
};

// Sliders send while they move; typed numbers and text only once committed, so a value is not
// sent half typed. The help is the label's tooltip, and also shown under the control when `help`.
function Field({ c, value, help, set }: { c: Control; value: unknown; help: boolean; set: (value: unknown) => void }): JSX.Element {
  const number = (e: JSX.TargetedEvent<HTMLInputElement>) => set(e.currentTarget.valueAsNumber);
  const tip = c.help || undefined;
  let input: JSX.Element;
  if (c.kind === "checkbox") {
    input = <label title={tip}><input type="checkbox" checked={value === true} onChange={(e) => set(e.currentTarget.checked)} /> {c.label}</label>;
  } else if (c.kind === "slider") {
    input = (
      <label title={tip}>
        <span class="row">
          {c.label}
          <span>
            <input type="number" aria-label={c.label} min={c.min} max={c.max} step={c.step} value={value as number} onChange={number} /> {c.unit}
          </span>
        </span>
        <input type="range" aria-label={c.label} min={c.min} max={c.max} step={c.step} value={value as number} onInput={number} />
      </label>
    );
  } else if (c.kind === "number") {
    input = <label title={tip}>{c.label} <input type="number" step={c.integer ? 1 : "any"} value={value as number} onChange={number} /> {c.unit}</label>;
  } else if (c.kind === "select") {
    input = (
      <label title={tip}>
        {c.label}{" "}
        <select value={String(value)} onChange={(e) => set(e.currentTarget.value)}>
          {c.options.map((o) => <option value={o.value} title={o.title ?? undefined}>{o.label}</option>)}
        </select>
      </label>
    );
  } else {
    input = <label title={tip}>{c.label} <input type="text" value={String(value ?? "")} onChange={(e) => set(e.currentTarget.value)} /></label>;
  }
  return <div class="control">{input}{help && c.help && <p class="help">{c.help}</p>}</div>;
}

export function Settings({ stages, values, errors, help, part = "style", onChange }: Props): JSX.Element {
  const { own, shared, pens } = splitStages(stages);
  const stage = (s: Stage, open: boolean) => {
    const controls = controlsFor(s.schema);
    // A stage with no settings, such as the dot renderer, would open on nothing.
    if (controls.length === 0 && !errors[s.name]) return null;
    const field = (c: Control) => <Field c={c} value={values[s.name]?.[c.key] ?? c.default} help={help} set={(v) => onChange(s.name, c.key, v)} />;
    const advanced = controls.filter((c) => c.advanced);
    const body = (
      <>
        {errors[s.name] && <p class="error">{errors[s.name]}</p>}
        {controls.filter((c) => !c.advanced).map(field)}
        {advanced.length > 0 && <details><summary>Advanced</summary>{advanced.map(field)}</details>}
      </>
    );
    if (part === "pens") return <div key={s.name}>{body}</div>;
    // Keyed by name, so a changed style does not hand one stage's open or closed state to another.
    // The tour rings the Image section by this attribute, which tour.test.ts looks for as written.
    if (s.name === "tone") return <details class="stage" data-tour="image-settings" key={s.name} open={open}><summary>{s.label}</summary>{body}</details>;
    return <details class="stage" key={s.name} open={open}><summary>{s.label}</summary>{body}</details>;
  };
  if (part === "pens") return <>{pens.map((s) => stage(s, true))}</>;
  return <>{own.map((s) => stage(s, true))}{shared.map((s) => stage(s, false))}</>;
}
