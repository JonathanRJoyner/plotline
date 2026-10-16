// The command side of the worker (spec 2026-10-12-gui-design section 5): one command in, one
// reply out, and the run events `drive` posts. Every decision lives here so tests can cover it;
// `worker.ts` only connects it to `self`.
import { describe, listInks, listPens, listStyles, presetFromText, syntheticStillLife, type Run, type Session } from "../pkg/plotline_wasm.js";
import { drive, macrotask, postDone } from "./driver";
import type { Canvas, Event, Finding, Machine, Pen, Stats, ThumbEvent, ToneMatch } from "./events";

/** A change the engine suggests, in the shape of `Session::analysis`: `settings` apply after `style`. */
export type Patch = { style?: string; settings?: Record<string, Record<string, unknown>>; page?: { width_mm: number; height_mm: number } };
export type Suggestion = { kind: string; reason: string; patch: Patch | null };
export type Speeds = { draw_mm_s: number; travel_mm_s: number; pen_lift_s: number };
export type State = {
  image: boolean; style: string;
  /** The name of the style's look the settings match; null when they match none, which the page calls Custom. */
  look: string | null;
  canvas: Canvas; pens: Pen[]; machine: Machine | null;
  settings: Record<string, Record<string, unknown>>; speeds: Speeds;
  run: { id: number; state: string; error: string | null }; stats: Stats | null;
  /** The pens of the last drawing's layers; empty without a drawing. */
  layers: Pen[];
  /** What would suit the image, from the current settings; empty without an image. */
  suggestions: Suggestion[];
  /** The plot lint's findings for the last drawing, from the current settings; empty without one. */
  lint: Finding[];
  /** How the last drawing's tones follow its tone map; null without one or with nothing to compare. */
  tone: ToneMatch | null;
};
export type Command = { id: number; method: string; args: unknown[] };
export type Reply = { id: number; ok: true; state: State; value?: unknown } | { id: number; ok: false; error: string };

// The commands that change what a drawing is made from: each one redraws once it succeeds.
const REDRAWS = new Set(["setImageBytes", "setImage", "sample", "setCanvas", "setPens", "setStyle", "applyLook", "resetSettings", "update", "loadPreset", "loadPresetText", "apply", "restore"]);

const message = (e: unknown): string => String((e as { message?: unknown } | null)?.message ?? e);

export class Protocol {
  private image = false;
  private counter = 0;
  private current: Run | null = null;
  private go: (() => void) | null = null;
  private thumbsId = 0;
  private readonly methods: Record<string, (...args: any[]) => unknown>;

  constructor(private readonly session: Session, private readonly post: (msg: Reply | Event | ThumbEvent, transfer?: Transferable[]) => void) {
    const s = session;
    this.methods = {
      listStyles: () => JSON.parse(listStyles()),
      listPens: () => JSON.parse(listPens()),
      listInks: () => JSON.parse(listInks()),
      setMachine: (machine: Machine | null) => s.setMachine(JSON.stringify(machine)),
      gcode: (layer: number) => s.gcode(layer),
      describe: (name: string) => JSON.parse(describe(name)),
      setImageBytes: (bytes: Uint8Array) => { s.setImageBytes(bytes); this.image = true; },
      setImage: (width: number, height: number, rgb: Float32Array) => { s.setImage(width, height, rgb); this.image = true; },
      // The default A4 drawing area's shape, 190 x 277 mm, so the sample fills it.
      sample: () => this.methods.setImage(600, 875, syntheticStillLife(600, 875)),
      setCanvas: (canvas: Canvas) => s.setCanvas(JSON.stringify(canvas)),
      setPens: (pens: Pen[]) => s.setPens(JSON.stringify(pens)),
      setStyle: (name: string) => s.setStyle(name),
      applyLook: (name: string) => s.applyLook(name),
      resetSettings: () => s.resetSettings(),
      update: (stage: string, values: Record<string, unknown>) => s.update(stage, JSON.stringify(values)),
      // The session calls, not `methods`, so the whole patch redraws once. A refusal partway leaves
      // the earlier parts applied; only a stale suggestion can cause one.
      apply: ({ style, settings, page }: Patch) => {
        if (style) s.setStyle(style);
        for (const [stage, values] of Object.entries(settings ?? {})) s.update(stage, JSON.stringify(values));
        if (page) s.setCanvas(JSON.stringify({ ...JSON.parse(s.preset()).canvas, ...page }));
      },
      setSpeeds: (speeds: Speeds) => s.setSpeeds(JSON.stringify(speeds)),
      run: () => this.start(),
      // The settings stay as they are: the page applies the value the run's end carries.
      fit: (metric: string, target: number) => this.start(() => s.fit(metric, target), "fit"),
      // The settings stay as they are, and the page only offers what the search found.
      optimize: (metric?: string, target?: number) => this.start(() => s.optimize(metric ?? null, target ?? null), "optimize"),
      cancel: () => this.current?.cancel(),
      // The calibration chart for the page and the first pen. The engine cancels the run, and the
      // events a finished run posts follow the reply. A refusal changes nothing.
      testchart: () => {
        s.testchart();
        const id = ++this.counter;
        this.current = null;
        this.go = () => postDone(s, id, this.post);
      },
      preset: () => JSON.parse(s.preset()),
      loadPreset: (preset: unknown) => s.loadPreset(JSON.stringify(preset)),
      // The text of a preset file, an exported SVG or an exported JSON drawing.
      loadPresetText: (text: string) => s.loadPreset(presetFromText(text)),
      // A setup in the preset's shape over the current preset, so the format and version stay ours.
      restore: (setup: Record<string, unknown>) => s.loadPreset(JSON.stringify({ ...JSON.parse(s.preset()), ...setup })),
      // Draws every style after the reply, which carries the id its `thumb` events will have.
      thumbs: (width: number) => {
        const id = ++this.thumbsId;
        this.go = () => void this.thumbs(id, width);
        return id;
      },
      cancelThumbs: () => { this.thumbsId++; },
      svg: () => s.svg(),
      json: () => s.json(),
    };
  }

  handle({ id, method, args }: Command): void {
    try {
      if (!Object.hasOwn(this.methods, method)) throw new Error(`unknown method: ${method}`);
      const value = this.methods[method](...args);
      if (REDRAWS.has(method) && this.image) this.start();
      this.post({ id, ok: true, state: this.state(), value });
    } catch (e) {
      this.post({ id, ok: false, error: message(e) });
    }
    const go = this.go;
    this.go = null;
    go?.();
  }

  // Starts a run; `handle` lets it go after the reply, which reports it running. `drive` would
  // finish a small drawing before its first await. `drive` posts the run's events and frees it,
  // and the catch gives it its one `end` even if it throws.
  private start(begin = () => this.session.start(), kind: "run" | "fit" | "optimize" = "run"): void {
    const run = begin();
    const id = ++this.counter;
    this.current = run;
    this.go = () => {
      drive(this.session, run, id, this.post, { kind })
        .catch((e) => this.post({ type: "end", id, state: "error", error: message(e), stats: null, tone: null, lint: [] }, []))
        .finally(() => { if (this.current === run) this.current = null; }); // `drive` freed it
    };
  }

  // Draws every listed style in turn as a trial, which leaves the session's own run, drawing and
  // settings alone, and posts each as a `thumb` event. It yields as `drive` does, so commands get
  // their turn, and a newer `thumbs` or a `cancelThumbs` ends it.
  private async thumbs(id: number, width: number): Promise<void> {
    let lastYield = performance.now();
    for (const { name } of JSON.parse(listStyles()) as Array<{ name: string }>) {
      if (id !== this.thumbsId) return;
      let png: Uint8Array | null = null;
      let error: string | null = null;
      try {
        const trial = this.session.trial(name);
        try {
          for (let frame = trial.nextFrame(); frame; frame = trial.nextFrame()) {
            frame.free();
            if (performance.now() - lastYield >= 50) {
              await macrotask();
              lastYield = performance.now();
              if (id !== this.thumbsId) return;
            }
          }
          error = trial.error() ?? null;
          png = trial.previewPng(width) ?? null;
        } finally {
          trial.free();
        }
      } catch (e) {
        error = message(e);
      }
      this.post({ type: "thumb", id, style: name, png, error }, png ? [png.buffer as ArrayBuffer] : []);
    }
  }

  private state(): State {
    const s = this.session;
    const { style, canvas, pens, machine, settings } = JSON.parse(s.preset());
    const stats = s.stats();
    const layers: Pen[] = [];
    const drawing = s.drawing();
    if (drawing) {
      try {
        for (let i = 0; i < drawing.layerCount(); i++) layers.push(JSON.parse(drawing.layerPen(i)));
      } finally {
        drawing.free();
      }
    }
    const analysis = s.analysis();
    const lint = s.lint();
    const tone = s.toneMatch();
    return {
      image: this.image, style, look: s.look() ?? null, canvas, pens, machine, settings, speeds: JSON.parse(s.speeds()), layers,
      suggestions: analysis ? JSON.parse(analysis).suggestions : [],
      lint: lint ? JSON.parse(lint) : [],
      tone: tone ? JSON.parse(tone) : null,
      run: { id: this.counter, state: s.state(), error: s.error() ?? null },
      stats: stats ? JSON.parse(stats) : null,
    };
  }
}
