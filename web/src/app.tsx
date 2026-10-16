// The page (spec 2026-10-12-gui-design section 6): the engine client, the last state it replied
// with, the run view that takes the run events, and the three columns. Only replies re-render the
// page; a run's frames and its end re-render just the parts that show them, so they never reset
// an input the user is typing into.
import { CircleHelp, Crosshair, Download, Image as ImageIcon, ImageUp, LayoutGrid, Maximize, Monitor, Moon, Play, Redo2, RotateCcw, Route, Settings as Gear, Sparkles, Square, Sun, Undo2 } from "lucide-preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { choice, CompareControls, offers, type Against } from "./compare";
import { Engine } from "./engine";
import type { BankPen, FitResult, FrameEvent, Ink, Machine, Optimum, Pen, ResultEvent } from "./events";
import { ACCEPT, download, imageFile } from "./files";
import { History, setupOf } from "./history";
import { ExportPanel, FitPanel, LintList, PagePanel, PensDialog, PensPanel, PresetsPanel, SettingsDialog, StatsPanel, StatusBar, StylesDialog, SuggestionsPanel, type Thumb } from "./panels";
import { Preview } from "./preview";
import type { Patch, State } from "./protocol";
import { Settings } from "./settings";
import { firstVisit, Tour } from "./tour";
import { zoomPercent, type View } from "./view";

type Style = { name: string; label: string; description: string };
type Stage = { name: string; label: string; schema: Record<string, unknown> };
type Look = { name: string; label: string; description: string };

/** The logo's mark, as docs/images/logo-light.svg draws it: a curve between two ring handles. */
const Mark = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
    <path d="M5 17C5 10 10 5 17 5" />
    <circle cx="5" cy="19" r="2" />
    <circle cx="19" cy="5" r="2" />
  </svg>
);

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

// The styles dialog's pictures are this many pixels wide: sharp when shown at two and a half
// times a cell's width, as pointing at a cell does.
const THUMB_PX = 600;
// The style a first visit starts on: its lines read as the picture at the fit zoom, where grid's dots barely show.
const FIRST_STYLE = "waves";
// Where Ctrl+Z belongs to the browser's own undo of typing.
const TYPING = "textarea, [contenteditable], input:not([type=checkbox], [type=radio], [type=range], [type=color], [type=file], [type=button])";

// The last preset, kept in the browser so a reload keeps the setup. Storage can be missing or
// full, and the setup is a convenience, so both are ignored.
const REMEMBERED = "plotline.preset";
function remember(text: string | null) {
  try {
    if (text === null) localStorage.removeItem(REMEMBERED);
    else localStorage.setItem(REMEMBERED, text);
  } catch { /* ignored */ }
}
function remembered(): unknown {
  try {
    return JSON.parse(localStorage.getItem(REMEMBERED) ?? "null");
  } catch {
    return null;
  }
}

// The colour theme, kept like the preset. The stylesheet follows the system unless the root says
// light or dark.
const THEME = "plotline.theme";
const THEMES = [["system", Monitor, "System theme"], ["light", Sun, "Light theme"], ["dark", Moon, "Dark theme"]] as const;
export function savedTheme(): string {
  try {
    return localStorage.getItem(THEME) ?? "system";
  } catch {
    return "system";
  }
}

function ThemeButton() {
  const [i, setI] = useState(() => Math.max(0, THEMES.findIndex(([t]) => t === document.documentElement.dataset.theme)));
  const [, Icon, label] = THEMES[i];
  const next = () => {
    const j = (i + 1) % THEMES.length;
    document.documentElement.dataset.theme = THEMES[j][0];
    try { localStorage.setItem(THEME, THEMES[j][0]); } catch { /* ignored */ }
    setI(j);
  };
  return <button type="button" class="icon" title={label} aria-label={label} onClick={next}><Icon /></button>;
}

type Status = Pick<State, "run" | "stats" | "layers" | "lint" | "tone">;

// The run state, stats, layers, lint findings and tone match, which both replies and run events
// change, for the parts that show them.
class StatusStore {
  readonly subs = new Set<(s: Status) => void>();
  constructor(public value: Status) {}
  set(value: Status) {
    this.value = value;
    for (const f of this.subs) f(value);
  }
}

function useStatus(store: StatusStore): Status {
  const [s, setS] = useState(store.value);
  useEffect(() => {
    store.subs.add(setS);
    setS(store.value);
    return () => { store.subs.delete(setS); };
  }, []);
  return s;
}

// Whether the help paragraphs show under the controls; off by default, since the labels carry
// the help as tooltips. Missing storage is ignored as for the preset.
const HELP = "plotline.help";
function helpShown(): boolean {
  try {
    return localStorage.getItem(HELP) === "1";
  } catch {
    return false;
  }
}
function keepHelp(on: boolean) {
  try {
    localStorage.setItem(HELP, on ? "1" : "0");
  } catch { /* ignored */ }
}

// The plotter units to the millimetre of the HPGL export, kept like the theme. HPGL's own unit is
// 0.025 mm, which is 40 to the mm. Missing storage, or a value that is no number above 0, gives 40.
const HPGL_UNITS = "plotline.hpglUnits";
const DEFAULT_HPGL_UNITS = 40;
function savedHpglUnits(): number {
  try {
    const n = Number(localStorage.getItem(HPGL_UNITS));
    return Number.isFinite(n) && n > 0 ? n : DEFAULT_HPGL_UNITS;
  } catch {
    return DEFAULT_HPGL_UNITS;
  }
}
function keepHpglUnits(n: number) {
  try {
    localStorage.setItem(HPGL_UNITS, String(n));
  } catch { /* ignored */ }
}

type ExportProps = {
  status: StatusStore; machine: boolean; error?: string;
  onExport: (format: "svg" | "json") => void; onHpgl: () => void; onGcode: (layer: number) => void; onTestchart: () => void;
};

function Export({ status, machine, error, onExport, onHpgl, onGcode, onTestchart }: ExportProps) {
  const { stats, layers } = useStatus(status);
  return <ExportPanel drawing={stats !== null} layers={layers} machine={machine} error={error} onExport={onExport} onHpgl={onHpgl} onGcode={onGcode} onTestchart={onTestchart} />;
}

type RunViewProps = Omit<Parameters<typeof Preview>[0], "frames" | "result" | "running" | "showSource" | "showTravel" | "compare" | "previous" | "view" | "onView" | "fits" | "focusing"> & {
  engine: Engine; status: StatusStore; stages: Stage[]; error?: string; fitError?: string;
  onOpen: (file: File | undefined) => void; onSample: () => void; onRun: () => void; onStop: () => void; onSvg: () => void;
  onSettings: () => void; onTour: () => void; onUndo?: () => void; onRedo?: () => void;
  onApply: (patch: Patch) => void; lintError?: string;
  onFit: (metric: string, target: number) => void; onFitted: (fit: FitResult) => void;
  onOptimize: (metric?: string, target?: number) => void;
};

// The toolbar, the preview and the status bar, which hold the run events and the view themselves.
// None of them has an input that can be half typed, so a re-render per frame resets nothing.
function RunView({ engine, status, stages, error, fitError, onOpen, onSample, onRun, onStop, onSvg, onSettings, onTour, onUndo, onRedo, onFit, onFitted, onOptimize, onApply, lintError, ...preview }: RunViewProps) {
  const { run, stats, lint, tone } = useStatus(status);
  // The latest frame with geometry from each pen, by pen index, for the preview.
  const [frames, setFrames] = useState<(FrameEvent | undefined)[]>([]);
  const [result, setResult] = useState<ResultEvent | null>(null);
  // The result the latest one replaced, for the compare view.
  const [previous, setPrevious] = useState<ResultEvent | null>(null);
  const [progress, setProgress] = useState<Record<string, { label: string; progress: number }>>({});
  const [showSource, setShowSource] = useState(false);
  const [showTravel, setShowTravel] = useState(false);
  const [comparing, setComparing] = useState(false);
  const [against, setAgainst] = useState<Against>("previous");
  const [swipe, setSwipe] = useState(50);
  const [focusing, setFocusing] = useState(false);
  const [view, setView] = useState<View>({ scale: 1, x: 0, y: 0 });
  const [fits, setFits] = useState(0);
  const [fitted, setFitted] = useState<FitResult | null>(null);
  const [optimum, setOptimum] = useState<Optimum | null>(null);
  // The run Stop was pressed for: a stage stops between steps, so the run can stay running a while.
  const [stopFor, setStopFor] = useState(0);
  const stopping = run.state === "running" && stopFor === run.id;
  // A mark under a device pixel wide draws as a faint grey or not at all, which reads as a blank page.
  const thin = stats !== null && preview.pens[0].width_mm * view.scale * (window.devicePixelRatio || 1) < 1;
  const latest = useRef(0);
  const offered = offers(previous, result);
  const shown = choice(against, offered);
  const current = useRef<ResultEvent | null>(null); // the `result` state for the event handler, which is set up once
  useEffect(() => {
    engine.onEvent = (e) => {
      if (e.type === "run") {
        latest.current = e.id;
        setFrames([]); // the preview keeps the previous result until this run's first frame with geometry
        setProgress({});
        return;
      }
      if (e.id < latest.current) return;
      if (e.type === "frame") {
        // A new array each time: the preview redraws on its identity. A frame without geometry,
        // such as the tone's, leaves the held ones.
        if (e.points || e.coords) setFrames((fs) => { const next = [...fs]; next[e.pen] = e; return next; });
        // A fit's or a search's trial has ended, so the stages start over for the next one.
        if (e.stage === "fit" || e.stage === "optimize") setProgress({ [e.stage]: { label: e.label, progress: e.progress } });
        else setProgress((p) => ({ ...p, [e.stage]: { label: e.label, progress: e.progress } }));
      } else if (e.type === "result") {
        if (current.current && current.current !== e) setPrevious(current.current);
        current.current = e;
        setResult(e);
        status.set({ ...status.value, layers: e.layers.map((l) => l.pen) });
      } else {
        status.set({ run: { id: e.id, state: e.state, error: e.error }, stats: e.stats, layers: status.value.layers, lint: e.lint, tone: e.tone });
        if (e.fit) {
          setFitted(e.fit);
          onFitted(e.fit);
        }
        if (e.optimum) setOptimum(e.optimum);
      }
    };
  }, []);
  return (
    <>
      {/* Grouped, so a narrow window wraps whole groups. */}
      <div class="toolbar" id="toolbar" tabIndex={-1}>
        <span class="group" data-tour="image">
          <label class="button" title="Open image">
            <ImageUp /> <span class="label">Open image</span>
            <input
              class="visually-hidden" type="file" accept={ACCEPT}
              onChange={(e) => {
                const file = e.currentTarget.files?.[0];
                e.currentTarget.value = ""; // so the same file can be opened again
                onOpen(file);
              }}
            />
          </label>
          <button type="button" title="Sample image" onClick={onSample}><Sparkles /> <span class="label">Sample image</span></button>
        </span>
        <span class="group" data-tour="run">
          <button type="button" class="primary" disabled={!preview.hasImage} onClick={onRun}><Play /> Run</button>
          <button type="button" title="Stop" disabled={run.state !== "running" || stopping} onClick={() => { setStopFor(run.id); onStop(); }}><Square /> <span class="label">Stop</span></button>
        </span>
        <span class="group" data-tour="undo">
          <button type="button" class="icon" title="Undo (Ctrl+Z)" aria-label="Undo" disabled={!onUndo} onClick={onUndo}><Undo2 /></button>
          <button type="button" class="icon" title="Redo (Ctrl+Shift+Z)" aria-label="Redo" disabled={!onRedo} onClick={onRedo}><Redo2 /></button>
        </span>
        <span class="group shrink" data-tour="view">
          <label class="chip" title="Source">
            <input class="visually-hidden" type="checkbox" checked={showSource} onChange={(e) => setShowSource(e.currentTarget.checked)} />
            <ImageIcon /> <span class="label">Source</span>
          </label>
          <label class="chip" title="Travel">
            <input class="visually-hidden" type="checkbox" checked={showTravel} onChange={(e) => setShowTravel(e.currentTarget.checked)} />
            <Route /> <span class="label">Travel</span>
          </label>
        </span>
        <span class="group shrink">
          <label class="chip" data-tour="focus" title="Focus: click the drawn image to keep detail around that point, click a point to remove it; dragging still pans">
            <input class="visually-hidden" type="checkbox" checked={focusing} onChange={(e) => setFocusing(e.currentTarget.checked)} />
            <Crosshair /> <span class="label">Focus</span>
          </label>
          <button type="button" title="Show the whole page; 0 does the same with the preview focused" onClick={() => setFits((n) => n + 1)}><Maximize /> <span class="label">Fit page</span></button>
          <span class="zoom" title="The preview's size against the paper's">{zoomPercent(view.scale)}%</span>
        </span>
        <span class="group end shrink">
          <button type="button" title="Export SVG" disabled={stats === null} onClick={onSvg}><Download /> <span class="label">Export SVG</span></button>
          <button type="button" title="Settings" data-tour="machine" onClick={onSettings}><Gear /> <span class="label">Settings</span></button>
          <button type="button" title="Tour" onClick={onTour}><CircleHelp /> <span class="label">Tour</span></button>
          <ThemeButton />
        </span>
      </div>
      {error && <p class="error">{error}</p>}
      <div class={comparing && shown ? "preview comparing" : "preview"} data-tour="preview">
        <Preview
          {...preview} frames={frames} result={result} running={run.state === "running"} showSource={showSource}
          showTravel={showTravel} compare={comparing && shown ? { against: shown, at: swipe } : null} previous={previous}
          view={view} onView={setView} fits={fits} focusing={focusing}
        />
        <CompareControls
          on={comparing} enabled={result !== null} against={shown} offered={offered} at={swipe}
          onOn={setComparing} onAgainst={setAgainst} onAt={setSwipe}
        />
        {thin && <p class="hint">At this zoom the pen is under a pixel wide: zoom in to see its marks.</p>}
      </div>
      <footer data-tour="status">
        <StatusBar stages={stages} progress={progress} run={run} stopping={stopping} />
        <StatsPanel stats={stats} tone={tone} />
        <LintList findings={lint} error={lintError} onApply={onApply} />
        <FitPanel
          disabled={!preview.hasImage} error={fitError} fitted={fitted} optimum={optimum} stages={stages}
          onFit={(metric, target) => { setFitted(null); setOptimum(null); onFit(metric, target); }}
          onOptimize={(metric, target) => { setFitted(null); setOptimum(null); onOptimize(metric, target); }}
          onApply={(patch) => { setOptimum(null); onApply(patch); }}
        />
      </footer>
    </>
  );
}

export function App() {
  const [fatal, setFatal] = useState<string | null>(null);
  const [engine] = useState(() => {
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    worker.addEventListener("error", (e) => setFatal(e.message || "the engine stopped"));
    return new Engine(worker);
  });
  const [styles, setStyles] = useState<Style[] | null>(null);
  const [bank, setBank] = useState<BankPen[]>([]);
  const [inks, setInks] = useState<Ink[]>([]);
  const [state, setState] = useState<State | null>(null);
  const [status] = useState(() => new StatusStore({ run: { id: 0, state: "idle", error: null }, stats: null, layers: [], lint: [], tone: null }));
  const [stages, setStages] = useState<Stage[]>([]);
  const [looks, setLooks] = useState<Look[]>([]);
  const [errors, setErrors] = useState<Record<string, string>>({}); // per stage; "" for the rest
  const [warnings, setWarnings] = useState<string[]>([]);
  const [draft, setDraft] = useState<State["settings"]>({}); // settings shown before the engine has them
  const [hidden, setHidden] = useState(new Set<number>());
  const [help, setHelp] = useState(helpShown);
  const [hpglUnits, setHpglUnits] = useState(savedHpglUnits);
  const [settingsOpens, setSettingsOpens] = useState(0);
  const [pensOpens, setPensOpens] = useState(0);
  const [tourOpens, setTourOpens] = useState(() => (firstVisit() ? 1 : 0));
  const unsent = useRef<State["settings"]>({});
  const timers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const [history] = useState(() => new History());
  const restoring = useRef(false);
  const [stylesOpens, setStylesOpens] = useState(0);
  const [thumbs, setThumbs] = useState<Record<string, Thumb>>({});
  const thumbsFor = useRef(0); // the id of the pictures the styles dialog shows; 0 for none

  // The focus points the page holds, as shares of the image: the last state's, or while edits are
  // on their way the list they will leave, so a quick second edit builds on the first.
  const focusShares = useRef<number[][]>([]);
  const focusEdits = useRef(0);
  const save = useRef<ReturnType<typeof setTimeout>>();
  const take = (s: State) => {
    setState(s);
    if (focusEdits.current === 0) focusShares.current = (s.settings.tone?.focus as number[][] | undefined) ?? [];
    history.see(setupOf(s), restoring.current);
    status.set({ run: s.run, stats: s.stats, layers: s.layers, lint: s.lint, tone: s.tone });
    clearTimeout(save.current);
    save.current = setTimeout(() => engine.call("preset").then((r) => remember(JSON.stringify(r.value)), () => {}), 500);
  };

  // Sends a command and takes the state from its reply. A refusal shows the message under
  // `where` and asks for the state again.
  async function call<T = unknown>(method: string, args: unknown[] = [], where = "", transfer?: Transferable[]): Promise<T | undefined> {
    try {
      const reply = await engine.call<T>(method, args, transfer);
      take(reply.state);
      setErrors((es) => ({ ...es, [where]: "" }));
      return reply.value;
    } catch (e) {
      setErrors((es) => ({ ...es, [where]: message(e) }));
      engine.call("preset").then((r) => take(r.state), () => {});
    }
  }

  useEffect(() => {
    engine.call<BankPen[]>("listPens").then((r) => setBank(r.value), () => {});
    engine.call<Ink[]>("listInks").then((r) => setInks(r.value), () => {});
    engine.call<Style[]>("listStyles").then(async ({ state, value }) => {
      setStyles(value);
      const saved = remembered();
      if (saved) {
        try {
          state = (await engine.call("loadPreset", [saved])).state;
        } catch {
          remember(null); // a preset this build refuses would block every start
        }
      } else {
        try {
          state = (await engine.call("setStyle", [FIRST_STYLE])).state;
        } catch { /* keeps the grid state */ }
      }
      take(state);
    }, (e) => setFatal(message(e)));
    const paste = (e: ClipboardEvent) => openDropped(e.clipboardData);
    document.addEventListener("paste", paste);
    // Ctrl+Z undoes and Ctrl+Shift+Z redoes, except while typing or with a dialog open.
    const keys = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.key.toLowerCase() !== "z") return;
      if ((e.target as Element).closest?.(TYPING) || document.querySelector("dialog.modal[open]")) return;
      e.preventDefault();
      restore(e.shiftKey ? history.redo() : history.undo());
    };
    document.addEventListener("keydown", keys);
    engine.onThumb = (e) => {
      if (e.id !== thumbsFor.current) return;
      const url = e.png && URL.createObjectURL(new Blob([e.png as Uint8Array<ArrayBuffer>], { type: "image/png" }));
      setThumbs((t) => ({ ...t, [e.style]: { url, error: e.error } }));
    };
    return () => {
      document.removeEventListener("paste", paste);
      document.removeEventListener("keydown", keys);
    };
  }, []);

  // Puts back a setup from the history; its reply is no new step.
  async function restore(setup: string | undefined) {
    if (setup === undefined) return;
    restoring.current = true;
    try {
      await call("restore", [JSON.parse(setup)]);
    } finally {
      restoring.current = false;
    }
  }

  async function tryStyles() {
    setThumbs((old) => {
      for (const t of Object.values(old)) if (t.url) URL.revokeObjectURL(t.url);
      return {};
    });
    thumbsFor.current = 0;
    setStylesOpens((n) => n + 1);
    thumbsFor.current = (await call<number>("thumbs", [THUMB_PX])) ?? 0;
  }

  const style = state?.style;
  useEffect(() => {
    if (style) {
      call<{ stages: Stage[]; looks: Look[] }>("describe", [style]).then((v) => {
        if (!v) return;
        setStages(v.stages);
        setLooks(v.looks);
      });
    }
  }, [style]);

  // Hiding goes by layer index, so a changed pen list starts with every pen shown.
  const pens = JSON.stringify(state?.pens);
  useEffect(() => setHidden(new Set()), [pens]);

  // Shows a setting at once and sends the stage's changes once 150 ms pass without another.
  function onSetting(stage: string, key: string, value: unknown) {
    setDraft((d) => ({ ...d, [stage]: { ...d[stage], [key]: value } }));
    unsent.current[stage] = { ...unsent.current[stage], [key]: value };
    clearTimeout(timers.current[stage]);
    timers.current[stage] = setTimeout(async () => {
      const values = unsent.current[stage];
      delete unsent.current[stage];
      await call("update", [stage, values], stage);
      // Changes made since wait for their own reply; the state now holds the rest.
      if (!unsent.current[stage]) setDraft(({ [stage]: _, ...d }) => d);
    }, 150);
  }

  async function openImage(file: File | undefined) {
    if (!file) return;
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      await call("setImageBytes", [bytes], "image", [bytes.buffer]);
    } catch (e) {
      setErrors((es) => ({ ...es, image: message(e) }));
    }
  }

  // Dropped or pasted data: a file of another type says so, where text pasted stays silent.
  function openDropped(dt: DataTransfer | null) {
    const file = imageFile(dt);
    const other = dt?.files[0];
    if (!file && other) setErrors((es) => ({ ...es, image: `${other.name} is not a PNG, JPEG or WebP image` }));
    else openImage(file);
  }

  async function savePreset() {
    const preset = await call("preset");
    if (preset) download(`plotline-${state?.style ?? "drawing"}-preset.json`, JSON.stringify(preset, null, 2), "application/json");
  }

  async function loadPreset(file: File) {
    const warned = await call<string[]>("loadPresetText", [await file.text()], "preset");
    // The engine's message names no file, and a refusal is the only way `warned` is undefined.
    if (!warned) setErrors((es) => ({ ...es, preset: es.preset && `Could not load ${file.name} (${es.preset}).` }));
    setWarnings(warned ?? []);
  }

  async function exportAs(format: "svg" | "json") {
    const text = await call<string>(format);
    if (text !== undefined) download(`plotline-${state?.style ?? "drawing"}.${format}`, text, format === "svg" ? "image/svg+xml" : "application/json");
  }

  async function exportHpgl() {
    const text = await call<string>("hpgl", [hpglUnits]);
    if (text !== undefined) download(`plotline-${state?.style ?? "drawing"}.hpgl`, text, "text/plain");
  }

  async function exportGcode(layer: number) {
    const text = await call<string>("gcode", [layer]);
    const pen = (status.value.layers[layer]?.name ?? String(layer + 1)).replace(/[^\w.-]+/g, "_");
    if (text !== undefined) download(`plot_${layer + 1}_${pen}.gcode`, text, "text/plain");
  }

  // A picked machine also makes the page its drawable area; an edited one changes only itself.
  async function pickMachine(machine: Machine | null) {
    await call("setMachine", [machine], "machine");
    if (machine && state) {
      // Under "machine", so a refused page shows in the dialog rather than behind it.
      await call("setCanvas", [{ ...state.canvas, width_mm: machine.area.width, height_mm: machine.area.height }], "machine");
    }
  }

  // Each change of the focus points is one update of the tone stage, so it reruns from there and
  // is one step of the history. It sends the list the page holds, changed, and the page holds the
  // new list at once, so a second change before the reply still has the first. A refusal, such as
  // a 17th point, shows under the toolbar, and the engine's own list comes back.
  async function editFocus(change: (points: number[][]) => number[][]) {
    focusShares.current = change(focusShares.current);
    focusEdits.current++;
    try {
      await call("update", ["tone", { focus: focusShares.current }]);
    } finally {
      focusEdits.current--;
    }
  }
  // The engine says which share of the image lies under the page point; off the image, nothing.
  async function addFocus(mm: [number, number]) {
    const share = await call<number[] | null>("imagePoint", mm);
    if (share) await editFocus((points) => [...points, share]);
  }
  // A point goes by what it is, not its place in the list, which an edit in between may move.
  const removeFocus = (share: number[] | undefined) =>
    share && editFocus((points) => points.filter((p) => p[0] !== share[0] || p[1] !== share[1]));

  // A fit leaves the settings alone; this applies the value it chose, and the redraw that
  // follows comes from the cache.
  const applyFit = (fit: FitResult) => call("update", [fit.stage, { [fit.key]: fit.value }], fit.stage);

  if (fatal) return <p class="loading">{fatal}</p>;
  if (!styles || !state) return <p class="loading">Loading the engine</p>;

  const settings = { ...state.settings };
  for (const k in draft) settings[k] = { ...settings[k], ...draft[k] };

  return (
    <div
      class="app"
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => { e.preventDefault(); openDropped(e.dataTransfer); }}
    >
      <a class="skip" href="#toolbar">Skip to the toolbar</a>
      <aside>
        <h1 class="brand"><Mark /> plotline</h1>
        <label data-tour="help">
          <input type="checkbox" checked={help} onChange={(e) => { setHelp(e.currentTarget.checked); keepHelp(e.currentTarget.checked); }} /> Show help
        </label>
        <fieldset data-tour="style">
          <legend>Style</legend>
          <label>
            Style{" "}
            <select value={state.style} onChange={(e) => call("setStyle", [e.currentTarget.value])}>
              {styles.map((s) => <option value={s.name}>{s.label}</option>)}
            </select>
          </label>
          <p class="help">{styles.find((s) => s.name === state.style)?.description}</p>
          {looks.length > 1 && (
            <>
              <label data-tour="look">
                Look{" "}
                <select value={state.look ?? ""} onChange={(e) => call("applyLook", [e.currentTarget.value])}>
                  {state.look === null && <option value="" disabled>Custom</option>}
                  {looks.map((l) => <option value={l.name}>{l.label}</option>)}
                </select>
              </label>
              <p class="help">{state.look === null ? "Not one of the looks: you changed a setting that the looks set." : looks.find((l) => l.name === state.look)?.description}</p>
            </>
          )}
          <button type="button" data-tour="styles" disabled={!state.image} onClick={tryStyles}><LayoutGrid /> Try all styles</button>
          <button type="button" data-tour="reset" onClick={() => call("resetSettings")}><RotateCcw /> Reset to defaults</button>
        </fieldset>
        <div class="stages" data-tour="settings">
          <Settings stages={stages} values={settings} errors={errors} help={help} onChange={onSetting} />
        </div>
      </aside>
      <main>
        <RunView
          engine={engine} status={status} stages={stages} canvas={state.canvas} hidden={hidden}
          pens={state.pens} hasImage={state.image} onOpen={openImage}
          focus={state.focus} radius={Number(settings.tone?.focus_mm ?? 40)}
          onFocusAt={addFocus} onFocusRemove={(i) => removeFocus((state.settings.tone?.focus as number[][] | undefined)?.[i])}
          error={errors[""] || (errors.image && `Could not open the image (${errors.image}).${state.image ? " The previous image is still open." : ""}`)}
          onSample={() => call("sample", [], "image")} onRun={() => call("run")} onStop={() => call("cancel")} onSvg={() => exportAs("svg")}
          onSettings={() => {
            setErrors((es) => ({ ...es, machine: "" })); // an old refusal would show over a good machine
            setSettingsOpens((n) => n + 1);
          }}
          onTour={() => setTourOpens((n) => n + 1)}
          onUndo={history.past.length > 0 ? () => restore(history.undo()) : undefined}
          onRedo={history.future.length > 0 ? () => restore(history.redo()) : undefined}
          onApply={(patch) => call("apply", [patch], "lint")} lintError={errors.lint}
          fitError={errors.fit} onFit={(metric, target) => call("fit", [metric, target], "fit")} onFitted={applyFit}
          onOptimize={(metric, target) => call("optimize", metric === undefined ? [] : [metric, target], "fit")}
        />
      </main>
      <aside>
        {/* Always present, as the lint list's is, so the tour can ring it before any image. */}
        <div data-tour="suggestions">
          {state.suggestions.length > 0 && <SuggestionsPanel suggestions={state.suggestions} error={errors.suggestions} onApply={(patch) => call("apply", [patch], "suggestions")} />}
        </div>
        <PagePanel canvas={state.canvas} error={errors.page} onChange={(c) => call("setCanvas", [c], "page")} />
        <PensPanel
          pens={state.pens} hidden={hidden} error={errors.pens}
          onEdit={() => {
            setErrors((es) => ({ ...es, pens: "" })); // as for the settings dialog
            setPensOpens((n) => n + 1);
          }}
          onToggle={(i) => setHidden((h) => {
            const next = new Set(h); // a new set each time: the preview redraws on its identity
            if (!next.delete(i)) next.add(i);
            return next;
          })}
        >
          <Settings part="pens" stages={stages} values={settings} errors={errors} help={help} onChange={onSetting} />
        </PensPanel>
        <Export
          status={status} machine={state.machine !== null} error={errors.testchart}
          onExport={exportAs} onHpgl={exportHpgl} onGcode={exportGcode} onTestchart={() => call("testchart", [], "testchart")}
        />
        <PresetsPanel warnings={warnings} error={errors.preset} onSave={savePreset} onLoad={loadPreset} />
      </aside>
      <SettingsDialog
        opens={settingsOpens} machine={state.machine} error={errors.machine} speeds={state.speeds} onPick={pickMachine}
        onEdit={(m) => call("setMachine", [m], "machine")} onSpeeds={(s) => call("setSpeeds", [s])}
        hpglUnits={hpglUnits} onHpglUnits={(n) => { setHpglUnits(n); keepHpglUnits(n); }}
      />
      <PensDialog
        opens={pensOpens} pens={state.pens} bank={bank} inks={inks} error={errors.pens} onChange={(p) => call("setPens", [p], "pens")}
      />
      <StylesDialog
        opens={stylesOpens} styles={styles} style={state.style} canvas={state.canvas} thumbs={thumbs}
        onPick={(name) => call("setStyle", [name])} onClose={() => call("cancelThumbs")}
      />
      <Tour opens={tourOpens} />
    </div>
  );
}
