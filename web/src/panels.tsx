// The side panels and the status bar of spec 2026-10-12-gui-design section 6.4. Each shows a
// slice of the state and hands changes back; the app sends them to the engine.
import { ArrowLeftRight, Download, FolderOpen, Plus, Save, X } from "lucide-preact";
import type { JSX } from "preact";
import type { Area, Canvas, Machine, Pen, Stats } from "./events";
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
      <button type="button" onClick={() => set({ width_mm: h, height_mm: w })}><ArrowLeftRight /> Swap width and height</button>
    </fieldset>
  );
}

type PensProps = { pens: Pen[]; bank: Pen[]; hidden: Set<number>; onChange: (pens: Pen[]) => void; onToggle: (index: number) => void };

export function PensPanel({ pens, bank, hidden, onChange, onToggle }: PensProps): JSX.Element {
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
          <button type="button" disabled={pens.length === 1} onClick={() => onChange(pens.filter((_, j) => j !== i))}><X /> Remove</button>
        </div>
      ))}
      <button type="button" onClick={() => onChange([...pens, { name: `Pen ${pens.length + 1}`, width_mm: 0.3, color: "#000000" }])}><Plus /> Add pen</button>
      <label>
        Add from bank{" "}
        <select
          value=""
          onChange={(e) => {
            const pen = bank.find((p) => p.name === e.currentTarget.value);
            if (pen) onChange([...pens, pen]);
          }}
        >
          <option value="">choose a pen</option>
          {bank.map((p) => <option value={p.name}>{p.name} ({p.width_mm} mm)</option>)}
        </select>
      </label>
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
          if (done(s.name)) return <li data-state="done">{s.label}: done ({p.label})</li>;
          if (s === current) return <li data-state="running">{s.label}: {p?.label ?? "running"} <progress value={p?.progress ?? 0} max={1} /></li>;
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
      <button type="button" onClick={onSave}><Save /> Save preset</button>
      <label class="button">
        <FolderOpen /> Load preset
        <input
          class="visually-hidden" type="file" accept="application/json,.json"
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

type ExportProps = {
  drawing: boolean; layers: Pen[]; machine: boolean;
  onExport: (format: "svg" | "json") => void; onGcode: (layer: number) => void;
};

// One G-code file per layer of the drawing, once a machine is set.
export function ExportPanel({ drawing, layers, machine, onExport, onGcode }: ExportProps): JSX.Element {
  return (
    <fieldset>
      <legend>Export</legend>
      <button type="button" disabled={!drawing} onClick={() => onExport("svg")}><Download /> SVG</button>
      <button type="button" disabled={!drawing} onClick={() => onExport("json")}><Download /> JSON</button>
      {machine && layers.map((p, i) => (
        <div><button type="button" disabled={!drawing} onClick={() => onGcode(i)}><Download /> G-code: {p.name}</button></div>
      ))}
    </fieldset>
  );
}

type NumberKey = "z_down_mm" | "z_up_mm" | "z_safe_mm" | "draw_feed" | "travel_feed" | "z_feed" | "settle_ms";
const MACHINE_FIELDS: [NumberKey, string, string][] = [
  ["z_down_mm", "Pen down Z", "mm"], ["z_up_mm", "Pen up Z", "mm"], ["z_safe_mm", "Safe Z", "mm"],
  ["draw_feed", "Draw feed", "mm/min"], ["travel_feed", "Travel feed", "mm/min"], ["z_feed", "Z feed", "mm/min"],
  ["settle_ms", "Settle", "ms"],
];
const AREA_KEYS: (keyof Area)[] = ["x_min", "x_max", "y_min", "y_max"];
// A starting point to edit, not a real machine: its values must be replaced with the machine's own.
const NEW_MACHINE: Machine = {
  name: "My machine", bed_mm: [200, 200], area: { x_min: 0, x_max: 200, y_min: 0, y_max: 200 },
  z_down_mm: 0, z_up_mm: 5, z_safe_mm: 10, draw_feed: 1500, travel_feed: 3000, z_feed: 600, settle_ms: 0,
  start_gcode: "", end_gcode: "",
};

type MachineProps = {
  machine: Machine | null; error?: string;
  onPick: (machine: Machine | null) => void; onEdit: (machine: Machine) => void;
};

// A select to drop the machine or start a new one, then the machine's fields. Picking is told apart
// from editing because a pick also sets the page to the machine's area.
export function MachinePanel({ machine, error, onPick, onEdit }: MachineProps): JSX.Element {
  const value = machine === null ? "none" : "current";
  const num = (label: string, unit: string, v: number, set: (v: number) => void) => (
    <label>
      {label}{" "}
      <input
        type="number" step="any" value={v}
        onChange={(e) => { const n = e.currentTarget.valueAsNumber; if (Number.isFinite(n)) set(n); else e.currentTarget.value = String(v); }}
      />{" "}
      {unit}
    </label>
  );
  return (
    <fieldset>
      <legend>Machine</legend>
      <label>
        Machine{" "}
        <select
          value={value}
          onChange={(e) => {
            const v = e.currentTarget.value;
            if (v === "none") onPick(null);
            else if (v === "new") onPick(NEW_MACHINE);
          }}
        >
          <option value="none">None (SVG only)</option>
          {machine && <option value="current">{machine.name}</option>}
          <option value="new">New machine</option>
        </select>
      </label>
      {error && <p class="error">{error}</p>}
      {machine && (
        <>
          <label>Name <input type="text" value={machine.name} onChange={(e) => onEdit({ ...machine, name: e.currentTarget.value })} /></label>
          {num("Bed width", "mm", machine.bed_mm[0], (v) => onEdit({ ...machine, bed_mm: [v, machine.bed_mm[1]] }))}
          {num("Bed height", "mm", machine.bed_mm[1], (v) => onEdit({ ...machine, bed_mm: [machine.bed_mm[0], v] }))}
          {AREA_KEYS.map((k) => num(`Area ${k.replace("_", " ")}`, "mm", machine.area[k], (v) => onEdit({ ...machine, area: { ...machine.area, [k]: v } })))}
          {MACHINE_FIELDS.map(([k, label, unit]) => num(label, unit, machine[k], (v) => onEdit({ ...machine, [k]: v })))}
          <label>Start G-code <textarea rows={2} value={machine.start_gcode} onChange={(e) => onEdit({ ...machine, start_gcode: e.currentTarget.value })} /></label>
          <label>End G-code <textarea rows={2} value={machine.end_gcode} onChange={(e) => onEdit({ ...machine, end_gcode: e.currentTarget.value })} /></label>
        </>
      )}
    </fieldset>
  );
}
