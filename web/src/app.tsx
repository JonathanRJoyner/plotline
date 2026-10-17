// The page (spec 2026-10-12-gui-design section 6): the engine client, the last state it replied
// with, the run view that takes the run events, and the three columns. Only replies re-render the
// page; a run's frames and its end re-render just the parts that show them, so they never reset
// an input the user is typing into.
import { CirclePlay, CircleHelp, Crosshair, Download, Image as ImageIcon, ImageUp, LayoutGrid, Maximize, Monitor, Moon, Play, Redo2, RotateCcw, Route, Settings as Gear, Sparkles, Square, Sun, Undo2 } from "lucide-preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { choice, CompareControls, offers, type Against } from "./compare";
import { Engine } from "./engine";
import type { BankPen, FitResult, FrameEvent, Ink, Machine, Optimum, Pen, ResultEvent } from "./events";
import { ACCEPT, download, imageFile } from "./files";
import { History, setupOf } from "./history";
import { loadLibrary, saveLibrary } from "./library";
import { restoreModels } from "./models";
import { ExportPanel, FitPanel, LintList, PagePanel, PensDialog, PensPanel, PresetsPanel, type ShareLink, SettingsDialog, StatsPanel, StatusBar, SuggestionsPanel } from "./panels";
import { type Entry, groupStyles, StylePicker, type Thumb } from "./picker";
import { duration, fits as timed, moment, PLAY_SECONDS, PlaybackBar, type Timeline } from "./playback";
import { Preview } from "./preview";
import type { CatalogueEntry, Patch, State } from "./protocol";
import { withModelChoice } from "./schema";
import { Settings } from "./settings";
import { decodeShare, encodeShare, PREFIX, shareable, withMachine } from "./share";
import { firstVisit, Tour } from "./tour";
import { zoomPercent, type View } from "./view";

type Style = Entry;
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

// The style picker's pictures are this many pixels wide: sharp when shown at two and a half
// times a cell's width, as pointing at a cell does. Every mark in them is at least THUMB_SHRINK
// pixels wide, so the marks still show in a cell about a quarter as wide; at one pixel, a dot
// style's sparse dots averaged to a blank cell. crates/core/tests/laws.rs checks every style.
const THUMB_PX = 600;
const THUMB_SHRINK = 2;
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
  status: StatusStore; machine: boolean;
  onExport: (format: "svg" | "json") => void; onHpgl: () => void; onGcode: (layer: number) => void;
};

function Export({ status, machine, onExport, onHpgl, onGcode }: ExportProps) {
  const { stats, layers } = useStatus(status);
  return <ExportPanel drawing={stats !== null} layers={layers} machine={machine} onExport={onExport} onHpgl={onHpgl} onGcode={onGcode} />;
}

// The file input of an Open button, in the toolbar and in the empty preview's card.
function OpenInput({ onOpen }: { onOpen: (file: File | undefined) => void }) {
  return (
    <input
      class="visually-hidden" type="file" accept={ACCEPT}
      onChange={(e) => {
        const file = e.currentTarget.files?.[0];
        e.currentTarget.value = ""; // so the same file can be opened again
        onOpen(file);
      }}
    />
  );
}

type RunViewProps = Omit<Parameters<typeof Preview>[0], "frames" | "result" | "running" | "showSource" | "showTravel" | "compare" | "previous" | "playback" | "view" | "onView" | "fits" | "focusing"> & {
  engine: Engine; status: StatusStore; stages: Stage[]; settings: State["settings"]; image: number; error?: string; fitError?: string;
  onOpen: (file: File | undefined) => void; onSample: () => void; onRun: () => void; onStop: () => void; onSvg: () => void;
  onSettings: () => void; onTour: () => void; onUndo?: () => void; onRedo?: () => void;
  onApply: (patch: Patch) => void; lintError?: string;
  onFit: (metric: string, target: number) => void; onFitted: (fit: FitResult) => void;
  onOptimize: (metric?: string, target?: number) => void;
};

// The toolbar, the preview and the status bar, which hold the run events and the view themselves.
// None of them has an input that can be half typed, so a re-render per frame resets nothing.
function RunView({ engine, status, stages, settings, image, error, fitError, onOpen, onSample, onRun, onStop, onSvg, onSettings, onTour, onUndo, onRedo, onFit, onFitted, onOptimize, onApply, lintError, ...preview }: RunViewProps) {
  const { run, stats, lint, tone } = useStatus(status);
  // Focus points shape the Image stage's tone map; a style without one, such as learned, keeps none.
  const toned = stages.some((s) => s.name === "tone");
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
  // The plot playback: whether it is shown, whether it plays, its time in seconds and the engine's
  // timeline of the result at the current speeds.
  const [replay, setReplay] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [at, setAt] = useState(0);
  const [timeline, setTimeline] = useState<Timeline | null>(null);
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
  // A new result, or new stats after a change of speeds, is timed again.
  useEffect(() => {
    if (!replay || !result) return;
    let dead = false;
    engine.call<Timeline | null>("timeline").then(
      ({ value }) => { if (!dead) setTimeline(value && timed(value, result.layers) ? value : null); },
      () => {},
    );
    return () => { dead = true; };
  }, [replay, result, stats?.estimated_seconds]);
  const timing = timeline && result && timed(timeline, result.layers) ? timeline : null;
  const total = timing ? duration(timing) : null;
  const playback = useMemo(() => (replay && timing && result ? moment(result.layers, timing, Math.min(at, total ?? 0)) : null), [replay, timing, result, at, total]);
  // The whole plot plays in PLAY_SECONDS of the page's clock, then stops at the end.
  useEffect(() => {
    if (!playing || total === null) return;
    let id = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = (now - last) / 1000;
      last = now;
      setAt((a) => Math.min(total, a + (dt * total) / PLAY_SECONDS));
      id = requestAnimationFrame(tick);
    };
    id = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(id);
  }, [playing, total]);
  useEffect(() => {
    if (playing && total !== null && at >= total) setPlaying(false);
  }, [playing, at, total]);
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
      <div class="toolbar">
        <span class="group" data-tour="image">
          <label class="button" title="Open image">
            <ImageUp /> <span class="label">Open image</span>
            <OpenInput onOpen={onOpen} />
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
          <label class="chip" data-tour="focus" title={toned ? "Focus: click the drawn image to keep detail around that point, click a point to remove it; dragging still pans" : "Focus: this style has no Image settings, so it keeps no focus points"}>
            <input class="visually-hidden" type="checkbox" disabled={!toned} checked={focusing && toned} onChange={(e) => setFocusing(e.currentTarget.checked)} />
            <Crosshair /> <span class="label">Focus</span>
          </label>
          <button type="button" title="Show the whole page; 0 does the same with the preview focused" onClick={() => setFits((n) => n + 1)}><Maximize /> <span class="label">Fit page</span></button>
          <span class="zoom" title="The preview's size against the paper's">{zoomPercent(view.scale)}%</span>
        </span>
        <span class="group end shrink">
          <button type="button" title="Export SVG" disabled={stats === null} onClick={onSvg}><Download /> <span class="label">Export SVG</span></button>
          <button type="button" title="Settings" data-tour="machine" onClick={onSettings}><Gear /> <span class="label">Settings</span></button>
          <button type="button" id="tour-button" title="Tour" onClick={onTour}><CircleHelp /> <span class="label">Tour</span></button>
          <ThemeButton />
        </span>
      </div>
      {error && <p class="error">{error}</p>}
      <div class={comparing && shown ? "preview comparing" : replay && result ? "preview playing" : "preview"} data-tour="preview">
        <Preview
          {...preview} frames={frames} result={result} running={run.state === "running"} showSource={showSource}
          showTravel={showTravel} compare={comparing && shown ? { against: shown, at: swipe } : null} previous={previous} playback={playback}
          view={view} onView={setView} fits={fits} focusing={focusing && toned}
        />
        {/* Not over a test chart, which draws with no image. */}
        {!preview.hasImage && result === null && (
          <div class="empty">
            <p class="lead">Start with an image</p>
            <button type="button" class="primary" onClick={onSample}>Use the sample image</button>
            <label class="button">Open an image <OpenInput onOpen={onOpen} /></label>
            <p class="help">Or drop or paste a PNG, JPEG or WebP anywhere on the page.</p>
          </div>
        )}
        <CompareControls
          on={comparing} enabled={result !== null} against={shown} offered={offered} at={swipe}
          onOn={(on) => { setComparing(on); if (on) { setReplay(false); setPlaying(false); } }} onAgainst={setAgainst} onAt={setSwipe}
        >
          <label class="chip" title="Play the plot: the drawing as the pen lays it down, in plot order" data-tour="play">
            <input
              class="visually-hidden" type="checkbox" checked={replay}
              onChange={(e) => {
                const on = e.currentTarget.checked;
                setReplay(on);
                setPlaying(on);
                setAt(0);
                if (on) setComparing(false);
              }}
            />
            <CirclePlay /> <span class="label">Play</span>
          </label>
        </CompareControls>
        {replay && result && <PlaybackBar total={total} at={Math.min(at, total ?? 0)} playing={playing} onAt={setAt} onPlaying={setPlaying} />}
        {thin && <p class="hint">At this zoom the pen is under a pixel wide: zoom in to see its marks.</p>}
      </div>
      <footer data-tour="status">
        <StatusBar stages={stages} progress={progress} run={run} stopping={stopping} />
        <StatsPanel stats={stats} tone={tone} />
        <LintList findings={lint} error={lintError} onApply={onApply} />
        {/* Fit and Optimize follow the tone match, which a style with no tone stage, such as learned, lacks. */}
        {toned && (
          <FitPanel
            image={image} disabled={!preview.hasImage} error={fitError} fitted={fitted} optimum={optimum} settings={settings} stages={stages}
            onFit={(metric, target) => { setFitted(null); setOptimum(null); onFit(metric, target); }}
            onOptimize={(metric, target) => { setFitted(null); setOptimum(null); onOptimize(metric, target); }}
            onApply={(patch) => { setOptimum(null); onApply(patch); }}
          />
        )}
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
  const [catalogue, setCatalogue] = useState<CatalogueEntry[]>([]);
  const [bank, setBank] = useState<BankPen[]>([]);
  const [inks, setInks] = useState<Ink[]>([]);
  const [state, setState] = useState<State | null>(null);
  const [status] = useState(() => new StatusStore({ run: { id: 0, state: "idle", error: null }, stats: null, layers: [], lint: [], tone: null }));
  const [stages, setStages] = useState<Stage[]>([]);
  const [looks, setLooks] = useState<Look[]>([]);
  const [errors, setErrors] = useState<Record<string, string>>({}); // per stage; "" for the rest
  const [warnings, setWarnings] = useState<string[]>([]);
  const [shared, setShared] = useState<(ShareLink & { setup: string }) | null>(null); // the last link made, with the setup it holds
  const [draft, setDraft] = useState<State["settings"]>({}); // settings shown before the engine has them
  const [hidden, setHidden] = useState(new Set<number>());
  const [help, setHelp] = useState(helpShown);
  const [hpglUnits, setHpglUnits] = useState(savedHpglUnits);
  const [settingsOpens, setSettingsOpens] = useState(0);
  const [pensOpens, setPensOpens] = useState(0);
  const [library, setLibrary] = useState(loadLibrary); // My pens, kept in this browser
  const [dismissed, setDismissed] = useState(new Set<string>()); // suggestion kinds hidden for this image
  const [images, setImages] = useState(0); // the images opened, so what was said of one goes with it
  const [tourOpens, setTourOpens] = useState(() => (firstVisit() ? 1 : 0));
  const unsent = useRef<State["settings"]>({});
  const timers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const [history] = useState(() => new History());
  const restoring = useRef(false);
  const [stylesOpens, setStylesOpens] = useState(0);
  const [thumbs, setThumbs] = useState<Record<string, Thumb>>({});
  const thumbsFor = useRef(0); // the id of the pictures the style picker shows; 0 for none
  const pickerOpen = useRef(false);
  // The latest drawThumbs, for the paste listener, which keeps the first render's functions.
  const redraw = useRef<(image: boolean) => Promise<void>>(async () => {});

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

  // Sends a command and takes the state from its reply; a refusal rejects with the engine's message.
  async function send(method: string, args: unknown[] = []) {
    const reply = await engine.call(method, args);
    take(reply.state);
    return reply;
  }

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
    // Before the setup loads, so the first image is measured with it; its state is not taken here.
    engine.call("setPenLibrary", [library]).then(() => {}, () => {});
    engine.call<BankPen[]>("listPens").then((r) => setBank(r.value), () => {});
    engine.call<Ink[]>("listInks").then((r) => setInks(r.value), () => {});
    engine.call<CatalogueEntry[]>("catalogue").then((r) => setCatalogue(r.value), () => {});
    // A share link's setup is read once and its fragment removed, so a reload does not apply it over later edits.
    const fragment = location.hash.startsWith(PREFIX) ? location.hash : null;
    if (fragment) window.history.replaceState(null, "", location.pathname + location.search);
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
      // The kept models go back into the engine; each reply brings the state with the model in it.
      void restoreModels(send);
      // After the remembered setup, so the link's wins and Undo goes back to the remembered one; a link that fails leaves it.
      if (fragment) {
        const read = await decodeShare(fragment);
        const own = await call<{ machine?: unknown }>("preset");
        if ("text" in read) await loadPresetText(withMachine(read.text, own?.machine), "the shared link");
        else setErrors((es) => ({ ...es, preset: read.error }));
      }
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

  // Opens the style picker and draws its pictures.
  function openPicker() {
    pickerOpen.current = true;
    setStylesOpens((n) => n + 1);
    void drawThumbs(state?.image ?? false);
  }

  // Draws the picker's pictures in its order, top first, after dropping the old ones; with no
  // image there are none to draw.
  async function drawThumbs(image: boolean) {
    setThumbs((old) => {
      for (const t of Object.values(old)) if (t.url) URL.revokeObjectURL(t.url);
      return {};
    });
    thumbsFor.current = 0;
    if (!image || !styles) return;
    const names = groupStyles(styles).flatMap(([, es]) => es.filter((e) => !e.picture).map((e) => e.name));
    thumbsFor.current = (await call<number>("thumbs", [THUMB_PX, THUMB_SHRINK, names])) ?? 0;
  }
  redraw.current = drawThumbs;

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

  // A new image starts with every suggestion, and with no Optimize sentence from the last one.
  function newImage() {
    setDismissed(new Set());
    setImages((n) => n + 1);
  }

  async function openImage(file: File | undefined) {
    if (!file) return;
    newImage();
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      await call("setImageBytes", [bytes], "image", [bytes.buffer]);
      // An image dropped or pasted with the picker open redraws its pictures; after a refusal the
      // previous image's come back, or with none each card's error, which no card shows.
      if (pickerOpen.current) await redraw.current(true);
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

  // A preset file and a share link load alike; `what` names the source in a refusal.
  async function loadPresetText(text: string, what: string) {
    const warned = await call<string[]>("loadPresetText", [text], "preset");
    // The engine's message names no source, and a refusal is the only way `warned` is undefined.
    if (!warned) setErrors((es) => ({ ...es, preset: es.preset && `Could not load ${what} (${es.preset}).` }));
    setWarnings(warned ?? []);
  }

  const loadPreset = async (file: File) => loadPresetText(await file.text(), file.name);

  // Copies a link that holds the preset. A browser that refuses the clipboard gets the link shown to copy by hand.
  async function copyLink() {
    const preset = await call("preset");
    if (!preset) return;
    const link = location.origin + location.pathname + (await encodeShare(shareable(preset)));
    let copied = true;
    try {
      await navigator.clipboard.writeText(link);
    } catch {
      copied = false;
    }
    setShared({ link, copied, setup: setupOf(preset as State) });
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

  const current = styles.find((s) => s.name === state.style);
  const settings = { ...state.settings };
  for (const k in draft) settings[k] = { ...settings[k], ...draft[k] };
  const suggestions = state.suggestions.filter((s) => !dismissed.has(s.kind));
  const model = String(settings.model?.model ?? "");

  return (
    <div
      class="app"
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => { e.preventDefault(); openDropped(e.dataTransfer); }}
    >
      <a class="skip" href="#style-settings">Skip to the style settings</a>
      {/* Before the settings in the page, so Tab reaches the toolbar first; the grid puts it between the columns. */}
      <main>
        <RunView
          engine={engine} status={status} stages={stages} settings={settings} image={images} canvas={state.canvas} paper={state.paper} hidden={hidden}
          pens={state.pens} hasImage={state.image} onOpen={openImage}
          focus={state.focus} radius={Number(settings.tone?.focus_mm ?? 40)}
          onFocusAt={addFocus} onFocusRemove={(i) => removeFocus((state.settings.tone?.focus as number[][] | undefined)?.[i])}
          error={errors[""] || (errors.image && `Could not open the image (${errors.image}).${state.image ? " The previous image is still open." : ""}`)}
          onSample={() => { newImage(); call("sample", [], "image"); }} onRun={() => call("run")} onStop={() => call("cancel")} onSvg={() => exportAs("svg")}
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
      <aside class="side-left">
        <h1 class="brand"><Mark /> plotline</h1>
        <label data-tour="help">
          <input type="checkbox" checked={help} onChange={(e) => { setHelp(e.currentTarget.checked); keepHelp(e.currentTarget.checked); }} /> Show help
        </label>
        <fieldset id="style-settings" tabIndex={-1} data-tour="style">
          <legend>Style</legend>
          <label>
            Style{" "}
            <button type="button" class="pick" data-tour="styles" aria-haspopup="dialog" aria-label={`Style: ${current?.label ?? state.style}`} value={state.style} onClick={openPicker}>
              <LayoutGrid /> <span>{current?.label ?? state.style}</span>
            </button>
          </label>
          <p class="help">{current?.description}</p>
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
          <button type="button" data-tour="reset" onClick={() => call("resetSettings")}><RotateCcw /> Reset to defaults</button>
        </fieldset>
        <div class="stages" data-tour="settings">
          <Settings stages={withModelChoice(stages, state.models, model)} values={settings} errors={errors} help={help} onChange={onSetting} />
        </div>
      </aside>
      <aside class="side-right">
        {/* Always present, as the lint list's is, so the tour can ring it before any image. */}
        <div data-tour="suggestions">
          {suggestions.length > 0 && (
            <SuggestionsPanel
              suggestions={suggestions} error={errors.suggestions} onApply={(patch) => call("apply", [patch], "suggestions")}
              onDismiss={(kind) => setDismissed((d) => new Set(d).add(kind))}
            />
          )}
        </div>
        <PagePanel canvas={state.canvas} paper={state.paper} error={errors.page} onChange={(c) => call("setCanvas", [c], "page")} onPaper={(c) => call("setPaper", [c], "page")} />
        <PensPanel
          pens={state.pens} hidden={hidden} error={errors.pens} onTestchart={() => call("testchart", [], "pens")}
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
        <Export status={status} machine={state.machine !== null} onExport={exportAs} onHpgl={exportHpgl} onGcode={exportGcode} />
        <PresetsPanel warnings={warnings} error={errors.preset} shared={shared?.setup === setupOf(state) ? shared : null} onSave={savePreset} onLoad={loadPreset} onCopy={copyLink} />
      </aside>
      <SettingsDialog
        opens={settingsOpens} machine={state.machine} error={errors.machine} speeds={state.speeds} onPick={pickMachine}
        onEdit={(m) => call("setMachine", [m], "machine")} onSpeeds={(s) => call("setSpeeds", [s])}
        hpglUnits={hpglUnits} onHpglUnits={(n) => { setHpglUnits(n); keepHpglUnits(n); }}
      />
      <PensDialog
        opens={pensOpens} pens={state.pens} bank={bank} inks={inks} error={errors.pens} onChange={(p) => call("setPens", [p], "pens")}
        library={library} onLibrary={(l) => { setLibrary(l); saveLibrary(l); void call("setPenLibrary", [l], "pens"); }}
      />
      <StylePicker
        opens={stylesOpens} styles={styles} style={state.style} canvas={state.canvas} paper={state.paper} hasImage={state.image} thumbs={thumbs}
        model={model} models={state.models} catalogue={catalogue} send={send}
        // A model's entry sets the style and its model in one change, so it redraws once.
        onPick={(e) => e.model !== undefined
          ? (e.name !== state.style || e.model !== model) && call("apply", [{ style: e.name !== state.style ? e.name : undefined, settings: { model: { model: e.model } } }])
          : e.name !== state.style && call("setStyle", [e.name])}
        onClose={() => { pickerOpen.current = false; call("cancelThumbs"); }}
      />
      <Tour opens={tourOpens} />
    </div>
  );
}
