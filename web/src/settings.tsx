// The settings form of spec 2026-10-12-gui-design section 6.3: one section per stage, built
// from the stage's schema, with the advanced settings collapsed.
import type { JSX } from "preact";
import type { State } from "./protocol";
import { controlsFor, type Control } from "./schema";

type Stage = { name: string; label: string; schema: Record<string, unknown> };
type Props = {
  stages: Stage[];
  values: State["settings"];
  errors: Record<string, string>;
  onChange: (stage: string, key: string, value: unknown) => void;
};

// Sliders send while they move; typed numbers and text only once committed, so a value is not
// sent half typed.
function Field({ c, value, set }: { c: Control; value: unknown; set: (value: unknown) => void }): JSX.Element {
  const number = (e: JSX.TargetedEvent<HTMLInputElement>) => set(e.currentTarget.valueAsNumber);
  let input: JSX.Element;
  if (c.kind === "checkbox") {
    input = <label><input type="checkbox" checked={value === true} onChange={(e) => set(e.currentTarget.checked)} /> {c.label}</label>;
  } else if (c.kind === "slider") {
    input = (
      <label>
        {c.label}
        <span class="pair">
          <input type="range" min={c.min} max={c.max} step={c.step} value={value as number} onInput={number} />
          <input type="number" aria-label={c.label} min={c.min} max={c.max} step={c.step} value={value as number} onChange={number} />
          {c.unit}
        </span>
      </label>
    );
  } else if (c.kind === "number") {
    input = <label>{c.label} <input type="number" step={c.integer ? 1 : "any"} value={value as number} onChange={number} /> {c.unit}</label>;
  } else if (c.kind === "select") {
    input = (
      <label>
        {c.label}{" "}
        <select value={String(value)} onChange={(e) => set(e.currentTarget.value)}>
          {c.options.map((o) => <option value={o.value} title={o.title ?? undefined}>{o.label}</option>)}
        </select>
      </label>
    );
  } else {
    input = <label>{c.label} <input type="text" value={String(value ?? "")} onChange={(e) => set(e.currentTarget.value)} /></label>;
  }
  return <div class="control">{input}{c.help && <p class="help">{c.help}</p>}</div>;
}

export function Settings({ stages, values, errors, onChange }: Props): JSX.Element {
  return (
    <>
      {stages.map((s) => {
        const controls = controlsFor(s.schema);
        const field = (c: Control) => <Field c={c} value={values[s.name]?.[c.key] ?? c.default} set={(v) => onChange(s.name, c.key, v)} />;
        const advanced = controls.filter((c) => c.advanced);
        return (
          <section>
            <h3>{s.label}</h3>
            {errors[s.name] && <p class="error">{errors[s.name]}</p>}
            {controls.filter((c) => !c.advanced).map(field)}
            {advanced.length > 0 && <details><summary>Advanced</summary>{advanced.map(field)}</details>}
          </section>
        );
      })}
    </>
  );
}
