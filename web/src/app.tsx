// The page (spec 2026-10-12-gui-design section 6): the engine client, the last state it replied
// with, the run events for the preview and the status bar, and the three columns.
import { useEffect, useRef, useState } from "preact/hooks";
import { Engine } from "./engine";
import type { FrameEvent, ResultEvent } from "./events";
import { ExportPanel, PagePanel, PensPanel, PresetsPanel, SpeedsPanel, StatsPanel, StatusBar } from "./panels";
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

export function App() {
  const [fatal, setFatal] = useState<string | null>(null);
  const [engine] = useState(() => {
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    worker.addEventListener("error", (e) => setFatal(e.message || "the engine stopped"));
    return new Engine(worker);
  });
  const [styles, setStyles] = useState<Style[] | null>(null);
  const [state, setState] = useState<State | null>(null);
  const [stages, setStages] = useState<Stage[]>([]);
  const [frame, setFrame] = useState<FrameEvent | null>(null);
  const [result, setResult] = useState<ResultEvent | null>(null);
  const [progress, setProgress] = useState<Record<string, { label: string; progress: number }>>({});
  const [errors, setErrors] = useState<Record<string, string>>({}); // per stage; "" for the rest
  const [warnings, setWarnings] = useState<string[]>([]);
  const [draft, setDraft] = useState<State["settings"]>({}); // settings shown before the engine has them
  const [hidden, setHidden] = useState(new Set<number>());
  const [showSource, setShowSource] = useState(false);
  const [showTravel, setShowTravel] = useState(false);
  const latest = useRef(0);
  const unsent = useRef<State["settings"]>({});
  const timers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});

  // Sends a command and takes the state from its reply. A refusal shows the message under
  // `where` and asks for the state again.
  async function call<T = unknown>(method: string, args: unknown[] = [], where = "", transfer?: Transferable[]): Promise<T | undefined> {
    try {
      const reply = await engine.call<T>(method, args, transfer);
      setState(reply.state);
      setErrors((es) => ({ ...es, [where]: "" }));
      return reply.value;
    } catch (e) {
      setErrors((es) => ({ ...es, [where]: message(e) }));
      engine.call("preset").then((r) => setState(r.state), () => {});
    }
  }

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
      } else {
        setState((s) => s && { ...s, run: { id: e.id, state: e.state, error: e.error }, stats: e.stats });
      }
    };
    engine.call<Style[]>("listStyles").then(({ state, value }) => {
      setState(state);
      setStyles(value);
    }, (e) => setFatal(message(e)));
  }, []);

  const style = state?.style;
  useEffect(() => {
    if (style) call<{ stages: Stage[] }>("describe", [style]).then((v) => v && setStages(v.stages));
  }, [style]);

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
    const bytes = new Uint8Array(await file.arrayBuffer());
    call("setImageBytes", [bytes], "", [bytes.buffer]);
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

  if (fatal) return <p class="loading">{fatal}</p>;
  if (!styles || !state) return <p class="loading">Loading the engine</p>;

  const running = state.run.state === "running";
  const settings = { ...state.settings };
  for (const k in draft) settings[k] = { ...settings[k], ...draft[k] };

  return (
    <div class="app">
      <aside>
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
          <label>Open image <input type="file" accept="image/png,image/jpeg" onChange={(e) => openImage(e.currentTarget.files?.[0])} /></label>
          <button type="button" onClick={() => call("sample")}>Sample image</button>
          <button type="button" disabled={!state.image} onClick={() => call("run")}>Run</button>
          <button type="button" disabled={!running} onClick={() => call("cancel")}>Stop</button>
          <label><input type="checkbox" checked={showSource} onChange={(e) => setShowSource(e.currentTarget.checked)} /> Source</label>
          <label><input type="checkbox" checked={showTravel} onChange={(e) => setShowTravel(e.currentTarget.checked)} /> Travel</label>
        </div>
        {errors[""] && <p class="error">{errors[""]}</p>}
        <div class="preview">
          <Preview
            canvas={state.canvas} frame={frame} result={result} running={running} showSource={showSource}
            showTravel={showTravel} hidden={hidden} firstPen={state.pens[0]} hasImage={state.image}
          />
        </div>
        <footer>
          <StatusBar stages={stages} progress={progress} run={state.run} />
          <StatsPanel stats={state.stats} />
        </footer>
      </main>
      <aside>
        <PagePanel canvas={state.canvas} onChange={(c) => call("setCanvas", [c])} />
        <PensPanel
          pens={state.pens} hidden={hidden} onChange={(p) => call("setPens", [p])}
          onToggle={(i) => setHidden((h) => {
            const next = new Set(h); // a new set each time: the preview redraws on its identity
            if (!next.delete(i)) next.add(i);
            return next;
          })}
        />
        <SpeedsPanel speeds={state.speeds} onChange={(s) => call("setSpeeds", [s])} />
        <PresetsPanel warnings={warnings} onSave={savePreset} onLoad={loadPreset} />
        <ExportPanel drawing={state.stats !== null} onExport={exportAs} />
      </aside>
    </div>
  );
}
