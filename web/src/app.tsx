// The page (spec 2026-10-12-gui-design section 6): the engine client, the last state it replied
// with, the run view that takes the run events, and the three columns. Only replies re-render the
// page; a run's frames and its end re-render just the parts that show them, so they never reset
// an input the user is typing into.
import { CircleHelp, Download, Image as ImageIcon, ImageUp, LayoutGrid, Maximize, Monitor, Moon, Play, Redo2, RotateCcw, Route, Settings as Gear, Sparkles, Square, Sun, Undo2 } from "lucide-preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { Engine } from "./engine";
import type { BankPen, FitResult, FrameEvent, Ink, Machine, Optimum, Pen, ResultEvent } from "./events";
import { ACCEPT, imageFile } from "./files";
import { History, setupOf } from "./history";
import { ExportPanel, FitPanel, LintList, PagePanel, PensDialog, PensPanel, PresetsPanel, SettingsDialog, StatsPanel, StatusBar, StylesDialog, SuggestionsPanel, type Thumb } from "./panels";
import { Preview } from "./preview";
import type { Patch, State } from "./protocol";
import { Settings } from "./settings";
import { firstVisit, Tour } from "./tour";
import { zoomPercent, type View } from "./view";

type Style = { name: string; label: string; description: string };
type Stage = { name: string; label: string; schema: Record<string, unknown> };

/** The logo's mark, as docs/images/logo-light.svg draws it: a curve between two ring handles. */
const Mark = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
    <path d="M5 17C5 10 10 5 17 5" />
    <circle cx="5" cy="19" r="2" />
    <circle cx="19" cy="5" r="2" />
  </svg>
);

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

// The styles dialog's pictures are this many pixels wide, about twice their size on screen.
const THUMB_PX = 300;
// Where Ctrl+Z belongs to the browser's own undo of typing.
const TYPING = "textarea, [contenteditable], input:not([type=checkbox], [type=radio], [type=range], [type=color], [type=file], [type=button])";

// Saves text as a file through a link to an object URL.
function download(name: string, text: string, type: string) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url));
}

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

type ExportProps = { status: StatusStore; machine: boolean; onExport: (format: "svg" | "json") => void; onGcode: (layer: number) => void };

function Export({ status, machine, onExport, onGcode }: ExportProps) {
  const { stats, layers } = useStatus(status);
  return <ExportPanel drawing={stats !== null} layers={layers} machine={machine} onExport={onExport} onGcode={onGcode} />;
}

type RunViewProps = Omit<Parameters<typeof Preview>[0], "frames" | "result" | "running" | "showSource" | "showTravel" | "view" | "onView" | "fits"> & {
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
  const [progress, setProgress] = useState<Record<string, { label: string; progress: number }>>({});
  const [showSource, setShowSource] = useState(false);
  const [showTravel, setShowTravel] = useState(false);
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
          <label class="button">
            <ImageUp /> Open image
            <input
              class="visually-hidden" type="file" accept={ACCEPT}
              onChange={(e) => {
                const file = e.currentTarget.files?.[0];
                e.currentTarget.value = ""; // so the same file can be opened again
                onOpen(file);
              }}
            />
          </label>
          <button type="button" onClick={onSample}><Sparkles /> Sample image</button>
        </span>
        <span class="group" data-tour="run">
          <button type="button" class="primary" disabled={!preview.hasImage} onClick={onRun}><Play /> Run</button>
          <button type="button" disabled={run.state !== "running" || stopping} onClick={() => { setStopFor(run.id); onStop(); }}><Square /> Stop</button>
        </span>
        <span class="group" data-tour="undo">
          <button type="button" class="icon" title="Undo (Ctrl+Z)" aria-label="Undo" disabled={!onUndo} onClick={onUndo}><Undo2 /></button>
          <button type="button" class="icon" title="Redo (Ctrl+Shift+Z)" aria-label="Redo" disabled={!onRedo} onClick={onRedo}><Redo2 /></button>
        </span>
        <span class="group" data-tour="view">
          <label class="chip">
            <input class="visually-hidden" type="checkbox" checked={showSource} onChange={(e) => setShowSource(e.currentTarget.checked)} />
            <ImageIcon /> Source
          </label>
          <label class="chip">
            <input class="visually-hidden" type="checkbox" checked={showTravel} onChange={(e) => setShowTravel(e.currentTarget.checked)} />
            <Route /> Travel
          </label>
        </span>
        <span class="group">
          <button type="button" onClick={() => setFits((n) => n + 1)}><Maximize /> Fit</button>
          <span class="zoom" title="The preview's size against the paper's">{zoomPercent(view.scale)}%</span>
        </span>
        <span class="group end">
          <button type="button" disabled={stats === null} onClick={onSvg}><Download /> Export SVG</button>
          <button type="button" data-tour="machine" onClick={onSettings}><Gear /> Settings</button>
          <button type="button" onClick={onTour}><CircleHelp /> Tour</button>
          <ThemeButton />
        </span>
      </div>
      {error && <p class="error">{error}</p>}
      <div class="preview" data-tour="preview">
        <Preview
          {...preview} frames={frames} result={result} running={run.state === "running"} showSource={showSource}
          showTravel={showTravel} view={view} onView={setView} fits={fits}
        />
        {thin && <p class="hint">The pen is under a pixel wide at this zoom, so its marks barely show: zoom in to see them.</p>}
      </div>
      <footer data-tour="status">
        <StatusBar stages={stages} progress={progress} run={run} stopping={stopping} />
        <StatsPanel stats={stats} tone={tone} />
        <LintList findings={lint} error={lintError} onApply={onApply} />
        <FitPanel
          disabled={!preview.hasImage} error={fitError} fitted={fitted} optimum={optimum}
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
  const [errors, setErrors] = useState<Record<string, string>>({}); // per stage; "" for the rest
  const [warnings, setWarnings] = useState<string[]>([]);
  const [draft, setDraft] = useState<State["settings"]>({}); // settings shown before the engine has them
  const [hidden, setHidden] = useState(new Set<number>());
  const [help, setHelp] = useState(helpShown);
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

  const save = useRef<ReturnType<typeof setTimeout>>();
  const take = (s: State) => {
    setState(s);
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
      }
      take(state);
    }, (e) => setFatal(message(e)));
    const paste = (e: ClipboardEvent) => openImage(imageFile(e.clipboardData));
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
    if (style) call<{ stages: Stage[] }>("describe", [style]).then((v) => v && setStages(v.stages));
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

  async function savePreset() {
    const preset = await call("preset");
    if (preset) download("preset.json", JSON.stringify(preset, null, 2), "application/json");
  }

  async function loadPreset(file: File) {
    let preset: unknown;
    try {
      preset = JSON.parse(await file.text());
    } catch {
      setErrors((es) => ({ ...es, preset: `${file.name} is not a preset: it is not valid JSON.` }));
      return;
    }
    setWarnings((await call<string[]>("loadPreset", [preset], "preset")) ?? []);
  }

  async function exportAs(format: "svg" | "json") {
    const text = await call<string>(format);
    if (text !== undefined) download(`drawing.${format}`, text, format === "svg" ? "image/svg+xml" : "application/json");
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
      onDrop={(e) => { e.preventDefault(); openImage(imageFile(e.dataTransfer)); }}
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
        {state.suggestions.length > 0 && <SuggestionsPanel suggestions={state.suggestions} error={errors.suggestions} onApply={(patch) => call("apply", [patch], "suggestions")} />}
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
        <Export status={status} machine={state.machine !== null} onExport={exportAs} onGcode={exportGcode} />
        <PresetsPanel warnings={warnings} error={errors.preset} onSave={savePreset} onLoad={loadPreset} />
      </aside>
      <SettingsDialog
        opens={settingsOpens} machine={state.machine} error={errors.machine} speeds={state.speeds} onPick={pickMachine}
        onEdit={(m) => call("setMachine", [m], "machine")} onSpeeds={(s) => call("setSpeeds", [s])}
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
