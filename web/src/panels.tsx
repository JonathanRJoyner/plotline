// The side panels and the status bar of spec 2026-10-12-gui-design section 6.4, laid out as spec
// 2026-10-12-ink-and-cleanup section 5 has it. Each shows a slice of the state and hands changes
// back; the app sends them to the engine.
import { ArrowDown, ArrowUp, Copy, Download, FolderOpen, PenLine, Plus, Save, Settings as Gear, X } from "lucide-preact";
import type { ComponentChildren, JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { filterBank } from "./bank";
import type { Area, BankPen, Canvas, Ink, Machine, Pen, Stats } from "./events";
import type { Speeds, State } from "./protocol";

const SIZES: [string, number, number][] = [
  ["A5", 148, 210], ["A4", 210, 297], ["A3", 297, 420], ["Letter", 215.9, 279.4], ["Tabloid", 279.4, 431.8],
];
const near = (a: number, b: number) => Math.abs(a - b) <= 0.05;

export function PagePanel({ canvas, error, onChange }: { canvas: Canvas; error?: string; onChange: (canvas: Canvas) => void }): JSX.Element {
  const { width_mm: w, height_mm: h } = canvas;
  const size = SIZES.find(([, a, b]) => (near(a, w) && near(b, h)) || (near(a, h) && near(b, w)));
  const set = (patch: Partial<Canvas>) => onChange({ ...canvas, ...patch });
  // A standard size keeps the page's orientation.
  const pick = (name: string) => {
    const [, a, b] = SIZES.find((s) => s[0] === name)!;
    set(w > h ? { width_mm: b, height_mm: a } : { width_mm: a, height_mm: b });
  };
  const mm = (key: "width_mm" | "height_mm" | "margin_mm", label: string) => (
    <label>{label}<span><input type="number" min="0" step="any" value={canvas[key]} onChange={(e) => set({ [key]: e.currentTarget.valueAsNumber })} /> mm</span></label>
  );
  return (
    <fieldset data-tour="page">
      <legend>Page</legend>
      <div class="grid">
        <label>
          Size
          <select value={size?.[0] ?? "custom"} onChange={(e) => pick(e.currentTarget.value)}>
            {SIZES.map(([name]) => <option value={name}>{name}</option>)}
            {!size && <option value="custom">custom</option>}
          </select>
        </label>
        <label>
          Orientation
          {/* The select holds only the two, so any change turns the page. */}
          <select value={w > h ? "landscape" : "portrait"} onChange={() => set({ width_mm: h, height_mm: w })}>
            <option value="portrait">portrait</option>
            <option value="landscape">landscape</option>
          </select>
        </label>
        {mm("width_mm", "Width")}
        {mm("height_mm", "Height")}
        {mm("margin_mm", "Margin")}
        <label>
          Fit
          <select value={canvas.fit} onChange={(e) => set({ fit: e.currentTarget.value as Canvas["fit"] })}>
            <option value="fit">fit</option>
            <option value="fill">fill</option>
          </select>
        </label>
      </div>
      {error && <p class="error">{error}</p>}
    </fieldset>
  );
}

type PensProps = {
  pens: Pen[]; hidden: Set<number>; error?: string;
  // The Pen use setting as the page shows it; unset means one pen.
  mode?: string;
  onEdit: () => void; onToggle: (index: number) => void;
  // Shown right after the pen list: the Pen use setting.
  children?: ComponentChildren;
};

// The pens as a short list with their preview toggles; they are edited in the pens dialog.
export function PensPanel({ pens, hidden, error, mode, onEdit, onToggle, children }: PensProps): JSX.Element {
  return (
    <fieldset data-tour="pens">
      <legend>Pens</legend>
      {pens.map((p, i) => (
        <div class="pen">
          <span class="swatch" style={{ background: p.color }} />
          <span class="name" title={p.name}>{p.name}</span>
          <span class="muted">{p.width_mm} mm</span>
          <input type="checkbox" aria-label={`Show ${p.name} in the preview`} title="Show in the preview" checked={!hidden.has(i)} onChange={() => onToggle(i)} />
        </div>
      ))}
      {error && <p class="error">{error}</p>}
      {children}
      {(mode ?? "one_pen") === "one_pen" && pens.length > 1 && <p class="help">Pen use is set to one pen, so only the first pen draws.</p>}
      {hidden.size > 0 && <p class="help">A hidden pen is only hidden in the preview: it still exports.</p>}
      <button type="button" onClick={onEdit}><PenLine /> Edit pens</button>
    </fieldset>
  );
}

// A modal on the native <dialog>, which brings focus trapping and Esc to close. Each change of
// `opens` opens it; the dialog element alone knows whether it is open, so nothing waits on its
// close event. The body holds a list of sections or items on the left and fields on the right.
function Modal({ opens, title, icon, wide, children }: { opens: number; title: string; icon: JSX.Element; wide?: boolean; children: ComponentChildren }): JSX.Element {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (opens > 0 && !ref.current!.open) ref.current!.showModal();
  }, [opens]);
  const id = `${title.toLowerCase()}-title`;
  return (
    <dialog ref={ref} class={wide ? "modal wide" : "modal"} aria-labelledby={id}>
      <header>
        <h2 id={id}>{icon} {title}</h2>
        <button type="button" class="icon" title="Close" aria-label="Close" onClick={() => ref.current!.close()}><X /></button>
      </header>
      <div class="body">{children}</div>
    </dialog>
  );
}

type PensDialogProps = { opens: number; pens: Pen[]; bank: BankPen[]; inks: Ink[]; error?: string; onChange: (pens: Pen[]) => void };

// Your pens in drawing order on the left, the chosen one's name, width and colour on the right,
// and the pen bank under them to give it a maker's pen. Changes apply as they are made.
export function PensDialog({ opens, pens, bank, inks, error, onChange }: PensDialogProps): JSX.Element {
  const [chosen, choose] = useState(0);
  const [kind, setKind] = useState("");
  const [query, setQuery] = useState("");
  const i = Math.min(chosen, pens.length - 1); // the list can shrink under it, as on a refused change
  const pen = pens[i];
  const set = (patch: Partial<Pen>) => onChange(pens.map((p, j) => (j === i ? { ...p, ...patch } : p)));
  // Puts `list` in place and chooses the pen at `next`.
  const put = (list: Pen[], next: number) => { onChange(list); choose(next); };
  const move = (by: number) => {
    const list = [...pens];
    [list[i], list[i + by]] = [list[i + by], list[i]];
    put(list, i + by);
  };
  const kinds = [...new Set(bank.map((p) => p.kind))];
  const groups = filterBank(bank, kind, query);
  const ink = inks.find((k) => k.color.toLowerCase() === pen.color.toLowerCase());
  return (
    <Modal opens={opens} title="Pens" icon={<PenLine />} wide>
      <nav>
        <ol class="pens">
          {pens.map((p, j) => (
            <li>
              <button type="button" aria-current={j === i ? "true" : undefined} onClick={() => choose(j)}>
                <span class="swatch" style={{ background: p.color }} />
                <span class="name">{p.name}</span>
                <span class="muted">{p.width_mm}</span>
              </button>
            </li>
          ))}
        </ol>
        <div class="actions">
          <button type="button" class="icon" title="Add pen" aria-label="Add pen" onClick={() => put([...pens, { name: `Pen ${pens.length + 1}`, width_mm: 0.3, color: "#000000" }], pens.length)}><Plus /></button>
          <button type="button" class="icon" title="Duplicate pen" aria-label="Duplicate pen" onClick={() => put([...pens.slice(0, i + 1), { ...pen }, ...pens.slice(i + 1)], i + 1)}><Copy /></button>
          <button type="button" class="icon" title="Move up: draws earlier" aria-label="Move up" disabled={i === 0} onClick={() => move(-1)}><ArrowUp /></button>
          <button type="button" class="icon" title="Move down: draws later" aria-label="Move down" disabled={i === pens.length - 1} onClick={() => move(1)}><ArrowDown /></button>
          {/* The session refuses an empty pen list. */}
          <button type="button" class="icon" title="Remove pen" aria-label="Remove pen" disabled={pens.length === 1} onClick={() => put(pens.filter((_, j) => j !== i), Math.max(0, i - 1))}><X /></button>
        </div>
        <p class="help">Pens plot in this order, and with Pen use set to one pen the first draws alone. List light inks first.</p>
      </nav>
      <div class="fields">
        {error && <p class="error">{error}</p>}
        <h3>Pen {i + 1}</h3>
        <label>Name <input type="text" value={pen.name} onChange={(e) => set({ name: e.currentTarget.value })} /></label>
        {num("Line width", "mm", pen.width_mm, (v) => set({ width_mm: v }))}
        <div class="inks" role="group" aria-label="Ink colour">
          <span>Colour</span>
          {inks.map((k) => (
            <button
              type="button" class="ink" title={k.name} aria-label={k.name} aria-pressed={k === ink}
              style={{ background: k.color }} onClick={() => set({ color: k.color })}
            />
          ))}
          <input type="color" title="Any colour" aria-label="Any colour" value={pen.color} onChange={(e) => set({ color: e.currentTarget.value })} />
        </div>
        <h3>Pen bank</h3>
        <p class="help">Choosing a size gives this pen its name and line width, as the maker publishes it. The colour stays.</p>
        <div class="filter">
          <select aria-label="Kind" value={kind} onChange={(e) => setKind(e.currentTarget.value)}>
            <option value="">every kind</option>
            {kinds.map((k) => <option value={k}>{k}</option>)}
          </select>
          <input type="search" placeholder="Search, such as micron 005" aria-label="Search the bank" value={query} onInput={(e) => setQuery(e.currentTarget.value)} />
        </div>
        {groups.length === 0 && <p class="help">No pen in the bank matches.</p>}
        {groups.map(([model, sizes]) => (
          <div class="model">
            <span>{model}<span class="muted"> · {sizes[0].kind}</span></span>
            <div class="sizes">
              {sizes.map((p) => {
                const size = p.name.slice(model.length + 1);
                return (
                  <button type="button" aria-pressed={p.name === pen.name && p.width_mm === pen.width_mm} title={`${p.name}, ${p.width_mm} mm line`} onClick={() => set({ name: p.name, width_mm: p.width_mm })}>
                    {size}{Number(size) !== p.width_mm && <span class="muted">{p.width_mm}</span>}
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </div>
    </Modal>
  );
}

export function StatsPanel({ stats }: { stats: Stats | null }): JSX.Element {
  const t = Math.round(stats?.estimated_seconds ?? 0);
  if (!stats) return <p class="stats">No drawing yet</p>;
  if (stats.path_count === 0) {
    return <p class="stats">No marks at these settings. Try a lower threshold, or the black, white and gamma settings under Image.</p>;
  }
  return (
    <p class="stats">
      {stats.path_count} paths · pen down {stats.pen_down_mm.toFixed(1)} mm · pen up {stats.pen_up_mm.toFixed(1)} mm
      · about {Math.floor(t / 60)} min {t % 60} s to plot
    </p>
  );
}

type StatusProps = {
  stages: { name: string; label: string }[];
  progress: Record<string, { label: string; progress: number }>;
  run: State["run"];
  stopping: boolean;
};

// Each stage is done, running with its frame's label, or waiting. The first stage not done runs
// before its first frame arrives.
export function StatusBar({ stages, progress, run, stopping }: StatusProps): JSX.Element {
  const done = (name: string) => (progress[name]?.progress ?? 0) >= 1;
  const current = run.state === "running" ? stages.find((s) => !done(s.name)) : undefined;
  return (
    <div class="status">
      <span class={run.error ? "error" : undefined}>{run.id > 0 ? `Run ${run.id}: ` : ""}{stopping ? "stopping…" : run.state}{run.error ? `: ${run.error}` : ""}</span>
      <ol>
        {stages.map((s) => {
          const p = progress[s.name];
          if (done(s.name)) return <li data-state="done" title={p.label}>{s.label}: done</li>;
          if (s === current) return <li data-state="running">{s.label}: {p?.label ?? "running"} <progress value={p?.progress ?? 0} max={1} /></li>;
          return <li>{s.label}: waiting</li>;
        })}
      </ol>
    </div>
  );
}

type PresetsProps = { warnings: string[]; error?: string; onSave: () => void; onLoad: (file: File) => void };

export function PresetsPanel({ warnings, error, onSave, onLoad }: PresetsProps): JSX.Element {
  return (
    <fieldset data-tour="presets">
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
      {error && <p class="error">{error}</p>}
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
    <fieldset data-tour="export">
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
const PEN_LIFT: [NumberKey, string, string, string][] = [
  ["z_down_mm", "Pen down Z", "mm", "The Z the pen draws at."],
  ["z_up_mm", "Pen up Z", "mm", "The Z it travels at between paths."],
  ["z_safe_mm", "Safe Z", "mm", "The Z each file lifts to before its first move and before its end lines."],
  ["z_feed", "Z feed", "mm/min", "How fast the pen moves up and down."],
  ["settle_ms", "Settle", "ms", "The pause after each Z move."],
];
const FEEDS: [NumberKey, string][] = [["draw_feed", "Draw"], ["travel_feed", "Travel"]];
const AREA: [keyof Area, string][] = [["width", "Width"], ["height", "Height"], ["x", "Offset X"], ["y", "Offset Y"]];
const SPEEDS: [keyof Speeds, string, string, number][] = [
  ["draw_mm_s", "Draw", "mm/s", 0.1], ["travel_mm_s", "Travel", "mm/s", 0.1], ["pen_lift_s", "Pen lift", "s", 0],
];
const SECTIONS = ["Machine", "Pen lift", "G-code", "Plot time"] as const;
// A starting point to edit, not a real machine: its values must be replaced with the machine's own.
const NEW_MACHINE: Machine = {
  name: "My machine", area: { x: 0, y: 0, width: 200, height: 200 },
  z_down_mm: 0, z_up_mm: 5, z_safe_mm: 10, draw_feed: 1500, travel_feed: 3000, z_feed: 600, settle_ms: 0,
  start_gcode: "", end_gcode: "",
};

function num(label: string, unit: string, v: number, set: (v: number) => void) {
  return (
    <label>
      {label}{" "}
      <input
        type="number" step="any" value={v}
        onChange={(e) => { const n = e.currentTarget.valueAsNumber; if (Number.isFinite(n)) set(n); else e.currentTarget.value = String(v); }}
      />{" "}
      {unit}
    </label>
  );
}

type SettingsProps = {
  opens: number;
  machine: Machine | null; error?: string; speeds: Speeds;
  onPick: (machine: Machine | null) => void; onEdit: (machine: Machine) => void; onSpeeds: (speeds: Speeds) => void;
};

// The setup that rarely changes, in a modal dialog: the machine, its pen lift and G-code, and the
// speeds the time estimate uses (spec 2026-10-12-settings-dialog section 4). Changes apply as they
// are made. Picking is told apart from editing because a pick also sets the page to the machine's
// area.
export function SettingsDialog({ opens, machine, error, speeds, onPick, onEdit, onSpeeds }: SettingsProps): JSX.Element {
  const [section, setSection] = useState<(typeof SECTIONS)[number]>("Machine");
  const value = machine === null ? "none" : "current";
  const choose = <p>Choose a machine first</p>;
  const fields = {
    "Machine": (
      <>
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
            <option value="none">None: export SVG for your plotter's software</option>
            {machine && <option value="current">{machine.name}</option>}
            <option value="new">New machine</option>
          </select>
        </label>
        {machine && (
          <>
            <label>Name <input type="text" value={machine.name} onChange={(e) => onEdit({ ...machine, name: e.currentTarget.value })} /></label>
            <h3>Drawable area</h3>
            {AREA.map(([k, label]) => num(label, "mm", machine.area[k], (v) => onEdit({ ...machine, area: { ...machine.area, [k]: v } })))}
            <p class="help">Offset is the machine position of the area's corner with the smallest X and Y. The page is centred in the area.</p>
            <h3>Feeds</h3>
            {FEEDS.map(([k, label]) => num(label, "mm/min", machine[k], (v) => onEdit({ ...machine, [k]: v })))}
          </>
        )}
      </>
    ),
    "Pen lift": machine ? PEN_LIFT.map(([k, label, unit, help]) => (
      <>
        {num(label, unit, machine[k], (v) => onEdit({ ...machine, [k]: v }))}
        <p class="help">{help}</p>
      </>
    )) : choose,
    "G-code": machine ? (
      <>
        <label>Start <textarea rows={6} value={machine.start_gcode} onChange={(e) => onEdit({ ...machine, start_gcode: e.currentTarget.value })} /></label>
        <label>End <textarea rows={6} value={machine.end_gcode} onChange={(e) => onEdit({ ...machine, end_gcode: e.currentTarget.value })} /></label>
        <p class="help">plotline writes G21 and G90 before the start lines, and lifts to safe Z before the end lines. Home the machine before mounting the pen: the files have no G28.</p>
      </>
    ) : choose,
    "Plot time": (
      <>
        {SPEEDS.map(([key, label, unit, min]) => (
          <label>
            {label}{" "}
            <input
              type="number" min={min} step="any" disabled={machine !== null}
              value={machine ? Number(speeds[key].toFixed(3)) : speeds[key]}
              onChange={(e) => {
                // The engine takes any number, but a speed at or below zero leaves no estimate.
                const v = e.currentTarget.valueAsNumber;
                if (Number.isFinite(v) && (min === 0 ? v >= 0 : v > 0)) onSpeeds({ ...speeds, [key]: v });
                else e.currentTarget.value = String(speeds[key]);
              }}
            />{" "}
            {unit}
          </label>
        ))}
        {machine && <p class="help">From the machine's feeds and pen lift.</p>}
      </>
    ),
  }[section];
  return (
    <Modal opens={opens} title="Settings" icon={<Gear />}>
      <nav>
        {SECTIONS.map((s) => <button type="button" aria-current={s === section ? "true" : undefined} onClick={() => setSection(s)}>{s}</button>)}
      </nav>
      <div class="fields">
        {error && <p class="error">{error}</p>}
        {fields}
      </div>
    </Modal>
  );
}
