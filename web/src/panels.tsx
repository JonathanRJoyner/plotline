// The side panels and the status bar of spec 2026-10-12-gui-design section 6.4. Each shows a
// slice of the state and hands changes back; the app sends them to the engine.
import type { JSX } from "preact";
import type { Canvas, Pen, Stats } from "./events";
import type { Speeds, State } from "./protocol";

const SIZES: [string, number, number][] = [
  ["A5", 148, 210], ["A4", 210, 297], ["A3", 297, 420], ["Letter", 215.9, 279.4], ["Tabloid", 279.4, 431.8],
];
const near = (a: number, b: number) => Math.abs(a - b) <= 0.05;

export function PagePanel({ canvas, onChange }: { canvas: Canvas; onChange: (canvas: Canvas) => void }): JSX.Element {
  const { width_mm: w, height_mm: h } = canvas;
  const size = SIZES.find(([, a, b]) => (near(a, w) && near(b, h)) || (near(a, h) && near(b, w)));
  const set = (patch: Partial<Canvas>) => onChange({ ...canvas, ...patch });
  // A standard size keeps the page's orientation.
  const pick = (name: string) => {
    const [, a, b] = SIZES.find((s) => s[0] === name)!;
    set(w > h ? { width_mm: b, height_mm: a } : { width_mm: a, height_mm: b });
  };
  const mm = (key: "width_mm" | "height_mm" | "margin_mm", label: string) => (
    <label>{label} <input type="number" min="0" step="any" value={canvas[key]} onChange={(e) => set({ [key]: e.currentTarget.valueAsNumber })} /> mm</label>
  );
  return (
    <fieldset>
      <legend>Page</legend>
      <label>
        Size{" "}
        <select value={size?.[0] ?? "custom"} onChange={(e) => pick(e.currentTarget.value)}>
          {SIZES.map(([name]) => <option value={name}>{name}</option>)}
          {!size && <option value="custom">custom</option>}
        </select>
      </label>
      {mm("width_mm", "Width")}
      {mm("height_mm", "Height")}
      {mm("margin_mm", "Margin")}
      <label>
        Fit{" "}
        <select value={canvas.fit} onChange={(e) => set({ fit: e.currentTarget.value as Canvas["fit"] })}>
          <option value="fit">fit</option>
          <option value="fill">fill</option>
        </select>
      </label>
      <button type="button" onClick={() => set({ width_mm: h, height_mm: w })}>Swap width and height</button>
    </fieldset>
  );
}

type PensProps = { pens: Pen[]; hidden: Set<number>; onChange: (pens: Pen[]) => void; onToggle: (index: number) => void };

export function PensPanel({ pens, hidden, onChange, onToggle }: PensProps): JSX.Element {
  const set = (i: number, patch: Partial<Pen>) => onChange(pens.map((p, j) => (j === i ? { ...p, ...patch } : p)));
  return (
    <fieldset>
      <legend>Pens</legend>
      {pens.map((p, i) => (
        <div class="pen">
          <label>Name <input type="text" value={p.name} onChange={(e) => set(i, { name: e.currentTarget.value })} /></label>
          <label>Width <input type="number" min="0" step="0.05" value={p.width_mm} onChange={(e) => set(i, { width_mm: e.currentTarget.valueAsNumber })} /> mm</label>
          <label>Colour <input type="color" value={p.color} onChange={(e) => set(i, { color: e.currentTarget.value })} /></label>
          <label><input type="checkbox" checked={!hidden.has(i)} onChange={() => onToggle(i)} /> Show</label>
          {/* The session refuses an empty pen list. */}
          <button type="button" disabled={pens.length === 1} onClick={() => onChange(pens.filter((_, j) => j !== i))}>Remove</button>
        </div>
      ))}
      <button type="button" onClick={() => onChange([...pens, { name: `Pen ${pens.length + 1}`, width_mm: 0.3, color: "#000000" }])}>Add pen</button>
    </fieldset>
  );
}

const SPEEDS: [keyof Speeds, string, string, number][] = [
  ["draw_mm_s", "Draw", "mm/s", 0.1], ["travel_mm_s", "Travel", "mm/s", 0.1], ["pen_lift_s", "Pen lift", "s", 0],
];

export function SpeedsPanel({ speeds, onChange }: { speeds: Speeds; onChange: (speeds: Speeds) => void }): JSX.Element {
  return (
    <fieldset>
      <legend>Speeds</legend>
      {SPEEDS.map(([key, label, unit, min]) => (
        <label>
          {label}{" "}
          <input
            type="number" min={min} step="any" value={speeds[key]}
            onChange={(e) => {
              // The engine takes any number, but a speed at or below zero leaves no estimate.
              const v = e.currentTarget.valueAsNumber;
              if (Number.isFinite(v) && (min === 0 ? v >= 0 : v > 0)) onChange({ ...speeds, [key]: v });
              else e.currentTarget.value = String(speeds[key]);
            }}
          />{" "}
          {unit}
        </label>
      ))}
    </fieldset>
  );
}

export function StatsPanel({ stats }: { stats: Stats | null }): JSX.Element {
  const t = Math.round(stats?.estimated_seconds ?? 0);
  return (
    <fieldset>
      <legend>Stats</legend>
      {stats ? (
        <dl>
          <dt>Paths</dt><dd>{stats.path_count}</dd>
          <dt>Pen down</dt><dd>{stats.pen_down_mm.toFixed(1)} mm</dd>
          <dt>Pen up</dt><dd>{stats.pen_up_mm.toFixed(1)} mm</dd>
          <dt>Time</dt><dd>{Math.floor(t / 60)} min {t % 60} s</dd>
        </dl>
      ) : <p>No drawing yet</p>}
    </fieldset>
  );
}

type StatusProps = {
  stages: { name: string; label: string }[];
  progress: Record<string, { label: string; progress: number }>;
  run: State["run"];
};

// Each stage is done, running with its frame's label, or waiting. The first stage not done runs
// before its first frame arrives.
export function StatusBar({ stages, progress, run }: StatusProps): JSX.Element {
  const done = (name: string) => (progress[name]?.progress ?? 0) >= 1;
  const current = run.state === "running" ? stages.find((s) => !done(s.name)) : undefined;
  return (
    <fieldset class="status">
      <legend>Run</legend>
      <ol>
        {stages.map((s) => {
          const p = progress[s.name];
          if (done(s.name)) return <li>{s.label}: done ({p.label})</li>;
          if (s === current) return <li>{s.label}: {p?.label ?? "running"} <progress value={p?.progress ?? 0} max={1} /></li>;
          return <li>{s.label}: waiting</li>;
        })}
      </ol>
      <p>{run.id > 0 ? `Run ${run.id}: ` : ""}{run.state}{run.error ? `: ${run.error}` : ""}</p>
    </fieldset>
  );
}

type PresetsProps = { warnings: string[]; onSave: () => void; onLoad: (file: File) => void };

export function PresetsPanel({ warnings, onSave, onLoad }: PresetsProps): JSX.Element {
  return (
    <fieldset>
      <legend>Presets</legend>
      <button type="button" onClick={onSave}>Save preset</button>
      <label>
        Load preset{" "}
        <input
          type="file" accept="application/json,.json"
          onChange={(e) => {
            const file = e.currentTarget.files?.[0];
            e.currentTarget.value = ""; // so the same file can be loaded again
            if (file) onLoad(file);
          }}
        />
      </label>
      {warnings.length > 0 && <ul class="warnings">{warnings.map((w) => <li>{w}</li>)}</ul>}
    </fieldset>
  );
}

type ExportProps = { drawing: boolean; onExport: (format: "svg" | "json") => void };

export function ExportPanel({ drawing, onExport }: ExportProps): JSX.Element {
  return (
    <fieldset>
      <legend>Export</legend>
      <button type="button" disabled={!drawing} onClick={() => onExport("svg")}>SVG</button>{" "}
      <button type="button" disabled={!drawing} onClick={() => onExport("json")}>JSON</button>
    </fieldset>
  );
}
