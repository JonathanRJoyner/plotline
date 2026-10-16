// The side panels and the status bar of spec 2026-10-12-gui-design section 6.4, laid out as spec
// 2026-10-12-ink-and-cleanup section 5 has it. Each shows a slice of the state and hands changes
// back; the app sends them to the engine.
import { ArrowDown, ArrowUp, BookmarkPlus, Check, Copy, Download, FolderOpen, LayoutGrid, PenLine, Plus, Ruler, Save, Settings as Gear, Target, Wand, X } from "lucide-preact";
import type { ComponentChildren, JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { filterBank } from "./bank";
import type { Area, BankPen, Canvas, Finding, FitResult, Ink, Machine, Optimum, Pen, Stats, ToneMatch } from "./events";
import { download } from "./files";
import { lengthText, pathsText } from "./format";
import { addPen, libraryFile, loadLibrary, mergeLibraries, parseLibrary, removePen, samePen, saveLibrary, usable } from "./library";
import type { Patch, Speeds, State, Suggestion } from "./protocol";
import { controlsFor } from "./schema";
import { place } from "./tour";
import { onPaper } from "./view";

const SIZES: [string, number, number][] = [
  ["A5", 148, 210], ["A4", 210, 297], ["A3", 297, 420], ["Letter", 215.9, 279.4], ["Tabloid", 279.4, 431.8],
];
const near = (a: number, b: number) => Math.abs(a - b) <= 0.05;

type PageProps = { canvas: Canvas; paper: string; error?: string; onChange: (canvas: Canvas) => void; onPaper: (color: string) => void };

export function PagePanel({ canvas, paper, error, onChange, onPaper }: PageProps): JSX.Element {
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
        <label>
          Paper
          {/* onChange fires when the picker closes, so one pick is one undo step. */}
          <input type="color" value={paper} onChange={(e) => onPaper(e.currentTarget.value)} />
        </label>
      </div>
      {error && <p class="error">{error}</p>}
    </fieldset>
  );
}

// What the engine suggests for the image: each reason, with an Apply button when it has a change to
// make and a button that hides it.
type SuggestionsProps = { suggestions: Suggestion[]; error?: string; onApply: (patch: Patch) => void; onDismiss: (kind: string) => void };
export function SuggestionsPanel({ suggestions, error, onApply, onDismiss }: SuggestionsProps): JSX.Element {
  return (
    <fieldset>
      <legend>Suggestions</legend>
      <ul class="suggestions">
        {suggestions.map((s) => {
          const p = s.patch;
          // The key keeps each button with its suggestion when an Apply removes the one above it.
          return (
            <li key={s.kind}>
              <p>{s.reason}</p>
              <div class="buttons">
                {p && <button type="button" aria-label={`Apply: ${s.reason}`} onClick={() => onApply(p)}><Check /> Apply</button>}
                <button type="button" class="icon" title="Dismiss" aria-label={`Dismiss: ${s.reason}`} onClick={() => onDismiss(s.kind)}><X /></button>
              </div>
            </li>
          );
        })}
      </ul>
      {error && <p class="error">{error}</p>}
    </fieldset>
  );
}

type PensProps = {
  pens: Pen[]; hidden: Set<number>; error?: string;
  onEdit: () => void; onToggle: (index: number) => void; onTestchart: () => void;
  // Shown right after the pen list: the Pen use setting.
  children?: ComponentChildren;
};

// The pens as a short list with their preview toggles; they are edited in the pens dialog. The test
// chart replaces the drawing with a calibration page for the first pen, so it needs no image.
export function PensPanel({ pens, hidden, error, onEdit, onToggle, onTestchart, children }: PensProps): JSX.Element {
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
      {hidden.size > 0 && <p class="help">A hidden pen is only hidden in the preview: it still exports.</p>}
      <button type="button" onClick={onEdit}><PenLine /> Edit pens</button>
      <button type="button" data-tour="testchart" title="Replace the drawing with a page of labelled test patches for the first pen" onClick={onTestchart}><Ruler /> Test chart</button>
    </fieldset>
  );
}

// A modal on the native <dialog>, which brings focus trapping and Esc to close. Each change of
// `opens` opens it; the dialog element alone knows whether it is open, so nothing waits on its
// close event, unless it asks with `onClose`. The body holds a list of sections or items on the left
// and fields on the right.
type ModalProps = { opens: number; title: string; icon: JSX.Element; wide?: boolean; onClose?: () => void; children: ComponentChildren };
function Modal({ opens, title, icon, wide, onClose, children }: ModalProps): JSX.Element {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (opens > 0 && !ref.current!.open) ref.current!.showModal();
  }, [opens]);
  const id = `${title.toLowerCase().replace(/\s+/g, "-")}-title`; // an id with a space breaks aria-labelledby
  return (
    <dialog ref={ref} class={wide ? "modal wide" : "modal"} aria-labelledby={id} onClose={onClose}>
      <header>
        <h2 id={id}>{icon} {title}</h2>
        <button type="button" class="icon" title="Close" aria-label="Close" onClick={() => ref.current!.close()}><X /></button>
      </header>
      <div class="body">{children}</div>
    </dialog>
  );
}

export type Thumb = { url: string | null; error: string | null };
type StylesDialogProps = {
  opens: number; styles: Array<{ name: string; label: string; description: string }>; style: string; canvas: Canvas; paper: string;
  thumbs: Record<string, Thumb>; onPick: (style: string) => void; onClose: () => void;
};

// The enlarged picture is this many times the cell's; its frame (padding, border and caption, as
// style.css draws them) adds this many pixels around it.
const ZOOM = 2.5;
const FRAME = { w: 14, h: 36 };

// Every style drawn small on the current image, page and pens, in a grid; a click switches to it
// (spec 2026-10-15-try-styles-and-undo section 2). The pictures arrive one by one; closing stops them.
// Pointing at a cell, or tabbing to it, shows its picture enlarged beside it, inside the dialog. The
// enlargement takes no clicks, and a touch shows none, so a tap still picks.
export function StylesDialog({ opens, styles, style, canvas, paper, thumbs, onPick, onClose }: StylesDialogProps): JSX.Element {
  const [zoom, setZoom] = useState<{ name: string; label: string; left: number; top: number; width: number } | null>(null);
  const ratio = canvas.width_mm / canvas.height_mm;
  const show = (name: string, label: string, cell: HTMLElement) => {
    const paper = cell.querySelector(".paper")!.getBoundingClientRect();
    const box = cell.closest("dialog")!.getBoundingClientRect();
    const width = Math.min(ZOOM * paper.width, box.width - 16 - FRAME.w, (box.height - 16 - FRAME.h) * ratio);
    const r = { left: paper.left - box.left, top: paper.top - box.top, right: paper.right - box.left, bottom: paper.bottom - box.top };
    const { left, top } = place(r, width + FRAME.w, width / ratio + FRAME.h, box.width, box.height);
    setZoom({ name, label, left: box.left + left, top: box.top + top, width });
  };
  const hide = () => setZoom(null);
  const url = zoom && thumbs[zoom.name]?.url;
  return (
    <Modal opens={opens} title="All styles" icon={<LayoutGrid />} wide onClose={() => { hide(); onClose(); }}>
      <ul class="thumbs" onScroll={hide}>
        {styles.map((s) => {
          const t = thumbs[s.name];
          return (
            <li key={s.name}>
              <button
                type="button" title={s.description} aria-current={s.name === style ? "true" : undefined}
                onClick={(e) => { onPick(s.name); e.currentTarget.closest("dialog")!.close(); }}
                onPointerEnter={(e) => e.pointerType !== "touch" && show(s.name, s.label, e.currentTarget)}
                onPointerLeave={hide}
                onFocus={(e) => e.currentTarget.matches(":focus-visible") && show(s.name, s.label, e.currentTarget)}
                onBlur={hide}
              >
                <span class="paper" style={{ aspectRatio: `${canvas.width_mm} / ${canvas.height_mm}`, background: paper, color: onPaper(paper) }}>
                  {t?.url ? <img src={t.url} alt="" /> : <span>{t?.error ?? "Drawing…"}</span>}
                </span>
                {s.label}
              </button>
            </li>
          );
        })}
      </ul>
      {zoom && url && (
        <figure class="zoom" style={{ left: zoom.left, top: zoom.top, width: zoom.width }} aria-hidden="true">
          <img src={url} alt="" style={{ aspectRatio: `${canvas.width_mm} / ${canvas.height_mm}` }} />
          <figcaption>{zoom.label}</figcaption>
        </figure>
      )}
    </Modal>
  );
}

type PensDialogProps = { opens: number; pens: Pen[]; bank: BankPen[]; inks: Ink[]; error?: string; onChange: (pens: Pen[]) => void };

// Your pens in drawing order on the left, the chosen one's name, width and colour on the right,
// and under them your saved pens and the pen bank to give it a pen. Changes apply as they are made.
export function PensDialog({ opens, pens, bank, inks, error, onChange }: PensDialogProps): JSX.Element {
  const [chosen, choose] = useState(0);
  const [kind, setKind] = useState("");
  const [query, setQuery] = useState("");
  const [library, setLibrary] = useState(loadLibrary);
  const [importError, setImportError] = useState<string>();
  const i = Math.min(chosen, pens.length - 1); // the list can shrink under it, as on a refused change
  const pen = pens[i];
  // The list the dialog last sent, which the prop catches up with on the reply, so a second edit made
  // before it builds on the first rather than undoing it.
  const latest = useRef(pens);
  useEffect(() => { latest.current = pens; }, [pens]);
  const send = (list: Pen[]) => { latest.current = list; onChange(list); };
  const set = (patch: Partial<Pen>) => send(latest.current.map((p, j) => (j === i ? { ...p, ...patch } : p)));
  // Puts `list` in place and chooses the pen at `next`.
  const put = (list: Pen[], next: number) => { send(list); choose(next); };
  const move = (by: number) => {
    const list = [...latest.current];
    if (!list[i] || !list[i + by]) return; // the sent list is shorter than the one shown
    [list[i], list[i + by]] = [list[i + by], list[i]];
    put(list, i + by);
  };
  const keep = (list: Pen[]) => { setLibrary(list); saveLibrary(list); };
  // Merges the pens of a library file in, or says why it cannot.
  const importFile = async (file: File) => {
    let value: unknown;
    try {
      value = JSON.parse(await file.text());
    } catch {
      return setImportError(`${file.name} is not a pen library: it is not valid JSON.`);
    }
    try {
      keep(mergeLibraries(library, parseLibrary(value)));
      setImportError(undefined);
    } catch (e) {
      setImportError(`${file.name} could not be imported (${(e as Error).message}).`);
    }
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
          <button type="button" class="icon" title="Add pen" aria-label="Add pen" onClick={() => put([...latest.current, { name: `Pen ${latest.current.length + 1}`, width_mm: 0.3, color: "#000000" }], latest.current.length)}><Plus /></button>
          <button type="button" class="icon" title="Duplicate pen" aria-label="Duplicate pen" onClick={() => put([...latest.current.slice(0, i + 1), { ...(latest.current[i] ?? pen) }, ...latest.current.slice(i + 1)], i + 1)}><Copy /></button>
          <button type="button" class="icon" title="Move up: draws earlier" aria-label="Move up" disabled={i === 0} onClick={() => move(-1)}><ArrowUp /></button>
          <button type="button" class="icon" title="Move down: draws later" aria-label="Move down" disabled={i === pens.length - 1} onClick={() => move(1)}><ArrowDown /></button>
          {/* The session refuses an empty pen list. */}
          <button type="button" class="icon" title="Remove pen" aria-label="Remove pen" disabled={pens.length === 1} onClick={() => put(latest.current.filter((_, j) => j !== i), Math.max(0, i - 1))}><X /></button>
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
        <div data-tour="library">
          <h3>My pens</h3>
          <p class="help">Pens you save stay in this browser. Choosing one gives this pen its name, line width and colour.</p>
          {library.length === 0 ? <p class="help">No saved pens yet. Save this pen keeps the pen above for next time.</p> : (
            <ul class="library">
              {library.map((p, j) => (
                <li>
                  <button type="button" aria-pressed={samePen(p, pen)} onClick={() => set({ name: p.name, width_mm: p.width_mm, color: p.color })}>
                    <span class="swatch" style={{ background: p.color }} />
                    <span class="name">{p.name}</span>
                    <span class="muted">{p.width_mm}</span>
                  </button>
                  <button type="button" class="icon" title="Remove from my pens" aria-label={`Remove ${p.name} from my pens`} onClick={() => keep(removePen(library, j))}><X /></button>
                </li>
              ))}
            </ul>
          )}
          <div class="actions">
            {/* A pen the file could not read back stays out, such as one with no name. */}
            <button type="button" disabled={!usable(pen) || library.some((p) => samePen(p, pen))} onClick={() => keep(addPen(library, { name: pen.name, width_mm: pen.width_mm, color: pen.color }))}><BookmarkPlus /> Save this pen</button>
            <button type="button" disabled={library.length === 0} onClick={() => download("plotline-pens.json", libraryFile(library), "application/json")}><Download /> Export</button>
            <label class="button">
              <FolderOpen /> Import
              <input
                class="visually-hidden" type="file" accept="application/json,.json"
                onChange={(e) => {
                  const file = e.currentTarget.files?.[0];
                  e.currentTarget.value = ""; // so the same file can be imported again
                  if (file) void importFile(file);
                }}
              />
            </label>
          </div>
          {importError && <p class="error">{importError}</p>}
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

export function StatsPanel({ stats, tone }: { stats: Stats | null; tone: ToneMatch | null }): JSX.Element {
  const t = Math.round(stats?.estimated_seconds ?? 0);
  if (!stats) return <p class="stats">No drawing yet</p>;
  // The plot lint's empty finding says what to try.
  if (stats.path_count === 0) return <p class="stats">No marks at these settings.</p>;
  return (
    <p class="stats">
      {pathsText(stats.path_count)} · pen down {lengthText(stats.pen_down_mm)} · pen up {lengthText(stats.pen_up_mm)}{" "}
      · about {Math.floor(t / 60)} min {t % 60} s to plot
      {tone && (
        <span title="How far the drawing's tones are from the image's, seen from a metre or two: the tone error is in the image's own ink, 0 for a perfect match, and SSIM compares their shapes, 1 for identical.">
          {" "}· tone error {tone.error.toFixed(3)} · SSIM {tone.ssim.toFixed(2)}
        </span>
      )}
    </p>
  );
}

// The plot lint's findings for the last drawing, each with an Apply button when a setting fixes it.
export function LintList({ findings, error, onApply }: { findings: Finding[]; error?: string; onApply: (patch: Patch) => void }): JSX.Element {
  return (
    <div data-tour="lint">
      {findings.length > 0 && (
        <ul class="suggestions lint">
          {findings.map((f) => {
            const p = f.patch;
            // Keyed as the suggestions are: a code appears at most once.
            return (
              <li key={f.code} data-severity={f.severity}>
                <p><span class="severity">{f.severity}</span> {f.message}</p>
                {p && <button type="button" aria-label={`Apply: ${f.message}`} onClick={() => onApply(p)}><Check /> Apply</button>}
              </li>
            );
          })}
        </ul>
      )}
      {error && <p class="error">{error}</p>}
    </div>
  );
}

const UNITS = [["minutes", "min"], ["paths", "paths"], ["metres", "m"], ["dots", "dots"]] as const;

// How near the target a fit counts as met: the solver's own tolerance.
const FIT_TOLERANCE = 0.02;

type FitProps = {
  disabled: boolean; error?: string; fitted: FitResult | null; optimum: Optimum | null; settings: State["settings"];
  stages: { name: string; label: string; schema: Record<string, unknown> }[];
  onFit: (metric: string, target: number) => void; onOptimize: (metric?: string, target?: number) => void; onApply: (patch: Patch) => void;
};

// A budget: a target in one of the units, which Fit meets by changing the style's density setting.
// What a fit or a search says holds only for the settings it was made with, so it shows while they do.
export function FitPanel({ disabled, error, fitted, optimum, settings, stages, onFit, onOptimize, onApply }: FitProps): JSX.Element {
  const [target, setTarget] = useState("");
  const [metric, setMetric] = useState("minutes");
  // Closed until the first need, so the stats lead; a result or an error opens it so none is hidden, and nothing closes it again.
  const [open, setOpen] = useState(false);
  useEffect(() => { if (error || fitted || optimum) setOpen(true); }, [error, fitted, optimum]);
  // A fit applies its value, so its message shows once the setting holds it and while it still does.
  const fit = fitted && settings[fitted.stage]?.[fitted.key] === fitted.value ? fitted : null;
  // A search applies nothing, so its suggestion shows while the settings are those of the render it arrived in.
  const now = JSON.stringify(settings);
  const arrival = useRef<{ optimum: Optimum | null; settings: string }>({ optimum: null, settings: "" });
  if (arrival.current.optimum !== optimum) arrival.current = { optimum, settings: now };
  const search = optimum && arrival.current.settings === now ? optimum : null;
  const unit = (m: string) => UNITS.find(([name]) => name === m)?.[1] ?? m;
  // What the fit says: the setting under its label and unit from the settings panel, and whether the target was met.
  const text = (f: FitResult) => {
    const stage = stages.find((s) => s.name === f.stage);
    const c = stage && controlsFor(stage.schema).find((c) => c.key === f.key);
    const label = c?.label ?? f.key;
    const set = `${String(f.value)}${c && "unit" in c && c.unit ? ` ${c.unit}` : ""}`;
    const u = unit(f.metric);
    const got = Number(f.achieved.toPrecision(4));
    const runs = `${f.samples.length} ${f.samples.length === 1 ? "run" : "runs"}`;
    if (Math.abs(f.achieved / f.target - 1) <= FIT_TOLERANCE) return `${label} set to ${set}: ${got} ${u} for a target of ${f.target} ${u}, in ${runs}.`;
    return `Could not reach ${f.target} ${u}. The nearest was ${got} ${u}, with ${label} at ${set}, after ${runs}.`;
  };
  // What the search found, in the same words: the setting under its label and unit.
  const found = (o: Optimum) => {
    const stage = stages.find((s) => s.name === o.stage);
    const c = stage && controlsFor(stage.schema).find((c) => c.key === o.key);
    const label = c?.label ?? o.key;
    const at = (v: unknown) => `${String(v)}${c && "unit" in c && c.unit ? ` ${c.unit}` : ""}`;
    const within = o.budget ? ` within ${o.budget.target} ${unit(o.budget.metric)}` : "";
    const time = (minutes: number) => `${minutes.toFixed(1)} min`;
    if (!o.patch) return `${label} at ${at(o.current)} already follows the image's tones best${within}, with a tone error of ${o.error.toFixed(3)} and ${time(o.minutes)} to plot.`;
    const against = o.current_error === null
      ? `${at(o.current)} goes over the target in ${time(o.current_minutes)}`
      : `against ${o.current_error.toFixed(3)} and ${time(o.current_minutes)} at ${at(o.current)}`;
    return `${label} at ${at(o.value)} follows the image's tones best${within}: a tone error of ${o.error.toFixed(3)} and ${time(o.minutes)} to plot, ${against}.`;
  };
  return (
    <details class="fit-panel" data-tour="fit" open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>Fit and optimize</summary>
      <form
        class="fit"
        onSubmit={(e) => { e.preventDefault(); if (Number(target) > 0) onFit(metric, Number(target)); }}
      >
        <input type="number" min="0" step="any" required aria-label="Target" placeholder="Target" value={target} onInput={(e) => setTarget(e.currentTarget.value)} />
        <select aria-label="Target unit" value={metric} onChange={(e) => setMetric(e.currentTarget.value)}>
          {UNITS.map(([name, label]) => <option value={name}>{label}</option>)}
        </select>
        <button
          type="submit" disabled={disabled}
          title="Reruns the style up to eight times with its density setting changed, and keeps the run nearest the target. The plot time leaves out acceleration, so styles of short strokes or dots plot slower than it says."
        >
          <Target /> Fit to target
        </button>
        <button
          type="button" disabled={disabled}
          title="Reruns the style up to nine times with its density setting from a quarter to four times its value, and suggests the value whose drawing's tones follow the image best. It leaves out values that plot for over twice as long, or an hour if that is more, or that the plot lint warns about, and keeps within the target if one is typed. Nothing changes until you apply it."
          onClick={() => (Number(target) > 0 ? onOptimize(metric, Number(target)) : onOptimize())}
        >
          <Wand /> Optimize
        </button>
        {error ? <span class="error">{error}</span> : fit ? <span>{text(fit)}</span> : search && (
          <span>
            {found(search)}
            {search.patch && <button type="button" aria-label={`Apply: ${found(search)}`} onClick={() => onApply(search.patch!)}><Check /> Apply</button>}
          </span>
        )}
      </form>
    </details>
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
      {run.state === "running" && progress.fit && <span>Fit: {progress.fit.label}</span>}
      {run.state === "running" && progress.optimize && <span>Optimize: {progress.optimize.label}</span>}
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
          class="visually-hidden" type="file" accept="application/json,.json,image/svg+xml,.svg"
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
  onExport: (format: "svg" | "json") => void; onHpgl: () => void; onGcode: (layer: number) => void;
};

// SVG, JSON and one HPGL file for the whole drawing; then, once a machine is set, one G-code file
// per layer. With several pens each G-code file gets a row told apart by its number and swatch,
// since pens can share a name.
export function ExportPanel({ drawing, layers, machine, onExport, onHpgl, onGcode }: ExportProps): JSX.Element {
  return (
    <fieldset data-tour="export">
      <legend>Export</legend>
      <button type="button" disabled={!drawing} onClick={() => onExport("svg")}><Download /> SVG</button>
      <button type="button" disabled={!drawing} onClick={() => onExport("json")}><Download /> JSON</button>
      <button type="button" disabled={!drawing} onClick={onHpgl}><Download /> HPGL</button>
      {!machine ? <p class="help">For G-code, set up a machine in Settings.</p>
        : layers.length < 2 ? <button type="button" disabled={!drawing} onClick={() => onGcode(0)}><Download /> G-code</button>
        : (
          <div class="gcode">
            <p class="help">G-code, one file per pen:</p>
            {layers.map((p, i) => (
              <button type="button" disabled={!drawing} title={p.name} aria-label={`G-code for pen ${i + 1}, ${p.name}`} onClick={() => onGcode(i)}>
                <Download /><span class="swatch" style={{ background: p.color }} /><span class="name">{i + 1}. {p.name}</span>
              </button>
            ))}
          </div>
        )}
    </fieldset>
  );
}

type NumberKey = "z_down_mm" | "z_up_mm" | "z_safe_mm" | "draw_feed" | "travel_feed" | "z_feed" | "settle_ms";
const Z_LIFT: [NumberKey, string, string, string][] = [
  ["z_down_mm", "Pen down Z", "mm", "The Z the pen draws at."],
  ["z_up_mm", "Pen up Z", "mm", "The Z it travels at between paths."],
  ["z_safe_mm", "Safe Z", "mm", "The Z each file lifts to before its first move and before its end lines."],
  ["z_feed", "Z feed", "mm/min", "How fast the pen moves up and down."],
];
// Placeholders that keep a switch to G-code lines from being refused for empty lines; the help
// says to replace them.
const SERVO = { pen_down_gcode: "M280 P0 S30", pen_up_gcode: "M280 P0 S90" };
const FEEDS: [NumberKey, string][] = [["draw_feed", "Draw"], ["travel_feed", "Travel"]];
const AREA: [keyof Area, string][] = [["width", "Width"], ["height", "Height"], ["x", "Offset X"], ["y", "Offset Y"]];
const SPEEDS: [keyof Speeds, string, string, number][] = [
  ["draw_mm_s", "Draw", "mm/s", 0.1], ["travel_mm_s", "Travel", "mm/s", 0.1], ["pen_lift_s", "Pen lift", "s", 0],
];
const SECTIONS = ["Machine", "Pen lift", "G-code", "HPGL", "Plot time"] as const;
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
  hpglUnits: number; onHpglUnits: (units: number) => void;
};

// The setup that rarely changes, in a modal dialog: the machine, its pen lift and G-code, and the
// speeds the time estimate uses (spec 2026-10-12-settings-dialog section 4). Changes apply as they
// are made. Picking is told apart from editing because a pick also sets the page to the machine's
// area.
export function SettingsDialog({ opens, machine, error, speeds, onPick, onEdit, onSpeeds, hpglUnits, onHpglUnits }: SettingsProps): JSX.Element {
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
            <p class="help">A new machine starts from placeholder values. Replace the drawable area, the feeds and every pen-lift value with your machine's own before you plot: a wrong pen-down Z can press the pen into the bed.</p>
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
    "Pen lift": machine ? (
      <>
        <label>
          Lift by{" "}
          <select
            value={machine.pen_lift ?? "z"}
            onChange={(e) => onEdit(e.currentTarget.value === "gcode"
              ? { ...machine, pen_lift: "gcode", pen_down_gcode: machine.pen_down_gcode || SERVO.pen_down_gcode, pen_up_gcode: machine.pen_up_gcode || SERVO.pen_up_gcode }
              : { ...machine, pen_lift: "z" })}
          >
            <option value="z">Z moves</option>
            <option value="gcode">G-code lines</option>
          </select>
        </label>
        {machine.pen_lift === "gcode" ? (
          <>
            <p class="help">For a pen lifted by a servo or a solenoid: plotline writes these lines as they are where it would move Z. The servo command filled in is a placeholder: replace it with your machine's own.</p>
            <label>Pen down <textarea rows={3} value={machine.pen_down_gcode} onChange={(e) => onEdit({ ...machine, pen_down_gcode: e.currentTarget.value })} /></label>
            <label>Pen up <textarea rows={3} value={machine.pen_up_gcode} onChange={(e) => onEdit({ ...machine, pen_up_gcode: e.currentTarget.value })} /></label>
          </>
        ) : Z_LIFT.map(([k, label, unit, help]) => (
          <>
            {num(label, unit, machine[k], (v) => onEdit({ ...machine, [k]: v }))}
            <p class="help">{help}</p>
          </>
        ))}
        {num("Settle", "ms", machine.settle_ms, (v) => onEdit({ ...machine, settle_ms: v }))}
        <p class="help">The pause after each pen move, so the pen has landed or lifted before the next move.</p>
      </>
    ) : choose,
    "G-code": machine ? (
      <>
        <label>Start <textarea rows={6} value={machine.start_gcode} onChange={(e) => onEdit({ ...machine, start_gcode: e.currentTarget.value })} /></label>
        <label>End <textarea rows={6} value={machine.end_gcode} onChange={(e) => onEdit({ ...machine, end_gcode: e.currentTarget.value })} /></label>
        <p class="help">
          {machine.pen_lift === "gcode"
            ? "plotline writes G21 and G90 before the start lines, and lifts the pen after them."
            : "plotline writes G21 and G90 before the start lines, and lifts to safe Z before the end lines."}
          {" "}Home the machine before mounting the pen: the files have no G28.
        </p>
        <label>
          Longest move{" "}
          <input
            type="number" min={0.5} step="any" placeholder="whole" value={machine.max_segment_mm ?? ""}
            onChange={(e) => { const n = e.currentTarget.valueAsNumber; onEdit({ ...machine, max_segment_mm: Number.isFinite(n) ? n : null }); }}
          />{" "}
          mm
        </label>
        <p class="help">For firmware that draws a long straight move as a curve, as a hanging plotter's may: plotline cuts every move into pieces no longer than this. Leave it empty to keep moves whole.</p>
      </>
    ) : choose,
    "HPGL": (
      <>
        <label>
          Plotter units per mm{" "}
          <input
            type="number" min={0} step="any" value={hpglUnits}
            onChange={(e) => {
              // Above 0, as the engine requires; anything else snaps back.
              const v = e.currentTarget.valueAsNumber;
              if (Number.isFinite(v) && v > 0) onHpglUnits(v);
              else e.currentTarget.value = String(hpglUnits);
            }}
          />
        </label>
        <p class="help">The resolution of the HPGL file, which needs no machine. HPGL's plotter unit is 0.025 mm, so 40 to the mm, which most plotters use; change it only if your plotter's manual gives another, up to 1000. It is kept in this browser.</p>
      </>
    ),
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
