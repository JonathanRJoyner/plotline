// The page (spec 2026-10-12-gui-design section 6): the engine client, the last state it replied
// with, the run view that takes the run events, and the three columns. Only replies re-render the
// page; a run's frames and its end re-render just the parts that show them, so they never reset
// an input the user is typing into.
import { Image as ImageIcon, ImageUp, Monitor, Moon, Play, Route, Sparkles, Spline, Square, Sun } from "lucide-preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { Engine } from "./engine";
import type { FrameEvent, Machine, Pen, ResultEvent } from "./events";
import { imageFile } from "./files";
import { ExportPanel, MachinePanel, PagePanel, PensPanel, PresetsPanel, SpeedsPanel, StatsPanel, StatusBar } from "./panels";
import { Preview } from "./preview";
import type { State } from "./protocol";
import { Settings } from "./settings";

type Style = { name: string; label: string; description: string };
type Stage = { name: string; label: string; schema: Record<string, unknown> };

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

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

type Status = Pick<State, "run" | "stats" | "layers">;

// The run state, stats and layers, which both replies and run events change, for the parts that
// show them.
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

const Stop = ({ status, onStop }: { status: StatusStore; onStop: () => void }) => (
  <button type="button" disabled={useStatus(status).run.state !== "running"} onClick={onStop}><Square /> Stop</button>
);

type ExportProps = { status: StatusStore; machine: boolean; onExport: (format: "svg" | "json") => void; onGcode: (layer: number) => void };

function Export({ status, machine, onExport, onGcode }: ExportProps) {
  const { stats, layers } = useStatus(status);
  return <ExportPanel drawing={stats !== null} layers={layers} machine={machine} onExport={onExport} onGcode={onGcode} />;
}

type RunViewProps = Omit<Parameters<typeof Preview>[0], "frame" | "result" | "running"> & {
  engine: Engine; status: StatusStore; stages: Stage[];
};

// The preview and the status bar, which hold the run events themselves.
function RunView({ engine, status, stages, ...view }: RunViewProps) {
  const { run, stats } = useStatus(status);
  const [frame, setFrame] = useState<FrameEvent | null>(null);
  const [result, setResult] = useState<ResultEvent | null>(null);
  const [progress, setProgress] = useState<Record<string, { label: string; progress: number }>>({});
  const latest = useRef(0);
  useEffect(() => {
    engine.onEvent = (e) => {
      if (e.type === "run") {
        latest.current = e.id;
        setFrame(null); // the preview keeps the previous result until this run's first frame
        setProgress({});
        return;
      }
      if (e.id < latest.current) return;
      if (e.type === "frame") {
        setFrame(e);
        setProgress((p) => ({ ...p, [e.stage]: { label: e.label, progress: e.progress } }));
      } else if (e.type === "result") {
        setResult(e);
        status.set({ ...status.value, layers: e.layers.map((l) => l.pen) });
      } else {
        status.set({ run: { id: e.id, state: e.state, error: e.error }, stats: e.stats, layers: status.value.layers });
      }
    };
  }, []);
  return (
    <>
      <div class="preview"><Preview {...view} frame={frame} result={result} running={run.state === "running"} /></div>
      <footer>
        <StatusBar stages={stages} progress={progress} run={run} />
        <StatsPanel stats={stats} />
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
  const [bank, setBank] = useState<Pen[]>([]);
  const [state, setState] = useState<State | null>(null);
  const [status] = useState(() => new StatusStore({ run: { id: 0, state: "idle", error: null }, stats: null, layers: [] }));
  const [stages, setStages] = useState<Stage[]>([]);
  const [errors, setErrors] = useState<Record<string, string>>({}); // per stage; "" for the rest
  const [warnings, setWarnings] = useState<string[]>([]);
  const [draft, setDraft] = useState<State["settings"]>({}); // settings shown before the engine has them
  const [hidden, setHidden] = useState(new Set<number>());
  const [showSource, setShowSource] = useState(false);
  const [showTravel, setShowTravel] = useState(false);
  const unsent = useRef<State["settings"]>({});
  const timers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});

  const save = useRef<ReturnType<typeof setTimeout>>();
  const take = (s: State) => {
    setState(s);
    status.set({ run: s.run, stats: s.stats, layers: s.layers });
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
    engine.call<Pen[]>("listPens").then((r) => setBank(r.value), () => {});
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
    return () => document.removeEventListener("paste", paste);
  }, []);

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
      await call("setImageBytes", [bytes], "", [bytes.buffer]);
    } catch (e) {
      setErrors((es) => ({ ...es, "": message(e) }));
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
    } catch (e) {
      setErrors((es) => ({ ...es, "": message(e) }));
      return;
    }
    setWarnings((await call<string[]>("loadPreset", [preset])) ?? []);
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
      const { x_min, x_max, y_min, y_max } = machine.area;
      await call("setCanvas", [{ ...state.canvas, width_mm: x_max - x_min, height_mm: y_max - y_min }]);
    }
  }

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
      <aside>
        <h1 class="brand"><Spline /> plotline</h1>
        <fieldset>
          <legend>Style</legend>
          <label>
            Style{" "}
            <select value={state.style} onChange={(e) => call("setStyle", [e.currentTarget.value])}>
              {styles.map((s) => <option value={s.name} title={s.description}>{s.label}</option>)}
            </select>
          </label>
        </fieldset>
        <Settings stages={stages} values={settings} errors={errors} onChange={onSetting} />
      </aside>
      <main>
        <div class="toolbar">
          <label class="button">
            <ImageUp /> Open image
            <input
              class="visually-hidden" type="file" accept="image/png,image/jpeg"
              onChange={(e) => {
                const file = e.currentTarget.files?.[0];
                e.currentTarget.value = ""; // so the same file can be opened again
                openImage(file);
              }}
            />
          </label>
          <button type="button" onClick={() => call("sample")}><Sparkles /> Sample image</button>
          <span class="divider" />
          <button type="button" class="primary" disabled={!state.image} onClick={() => call("run")}><Play /> Run</button>
          <Stop status={status} onStop={() => call("cancel")} />
          <span class="divider" />
          <label class="chip">
            <input class="visually-hidden" type="checkbox" checked={showSource} onChange={(e) => setShowSource(e.currentTarget.checked)} />
            <ImageIcon /> Source
          </label>
          <label class="chip">
            <input class="visually-hidden" type="checkbox" checked={showTravel} onChange={(e) => setShowTravel(e.currentTarget.checked)} />
            <Route /> Travel
          </label>
          <span class="spacer" />
          <ThemeButton />
        </div>
        {errors[""] && <p class="error">{errors[""]}</p>}
        <RunView
          engine={engine} status={status} stages={stages} canvas={state.canvas} showSource={showSource}
          showTravel={showTravel} hidden={hidden} firstPen={state.pens[0]} hasImage={state.image}
        />
      </main>
      <aside>
        <MachinePanel
          machine={state.machine} error={errors.machine}
          onPick={pickMachine} onEdit={(m) => call("setMachine", [m], "machine")}
        />
        <PagePanel canvas={state.canvas} onChange={(c) => call("setCanvas", [c])} />
        <PensPanel
          pens={state.pens} bank={bank} hidden={hidden} onChange={(p) => call("setPens", [p])}
          onToggle={(i) => setHidden((h) => {
            const next = new Set(h); // a new set each time: the preview redraws on its identity
            if (!next.delete(i)) next.add(i);
            return next;
          })}
        />
        <SpeedsPanel speeds={state.speeds} onChange={(s) => call("setSpeeds", [s])} />
        <PresetsPanel warnings={warnings} onSave={savePreset} onLoad={loadPreset} />
        <Export status={status} machine={state.machine !== null} onExport={exportAs} onGcode={exportGcode} />
      </aside>
    </div>
  );
}
