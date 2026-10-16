// The command side of the worker (spec 2026-10-12-gui-design section 5): one command in, one
// reply out, and the run events `drive` posts. Every decision lives here so tests can cover it;
// `worker.ts` only connects it to `self`.
import { describe, listInks, listPens, listStyles, presetFromText, Session, syntheticStillLife, takePanic, type Run } from "../pkg/plotline_wasm.js";
import { drive, free, macrotask, postDone } from "./driver";
import type { Canvas, Event, Finding, Machine, Pen, Stats, ThumbEvent, ToneMatch } from "./events";

/** A change the engine suggests, in the shape of `Session::analysis`: `pens` apply after `style`, `settings` after `pens`. */
export type Patch = { style?: string; pens?: Pen[]; settings?: Record<string, Record<string, unknown>>; page?: { width_mm: number; height_mm: number } };
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
  /** Where the tone stage's focus points are drawn, in page mm; empty without an image. */
  focus: [number, number][];
};
export type Command = { id: number; method: string; args: unknown[] };
export type Reply = { id: number; ok: true; state: State; value?: unknown } | { id: number; ok: false; error: string };

// The commands that change what a drawing is made from: each one redraws once it succeeds.
const REDRAWS = new Set(["setImageBytes", "setImage", "sample", "setCanvas", "setPens", "setStyle", "applyLook", "resetSettings", "update", "loadPreset", "loadPresetText", "apply", "restore"]);

const message = (e: unknown): string => String((e as { message?: unknown } | null)?.message ?? e);
// A trap: a panic or memory running out aborts the call with a `RuntimeError`, and a stack
// overflow with a `RangeError`. Either leaves the objects the call was using borrowed for good.
const trapped = (e: unknown): boolean => e instanceof WebAssembly.RuntimeError || e instanceof RangeError;

export class Protocol {
  private image = false;
  private counter = 0;
  private current: Run | null = null;
  private go: (() => void) | null = null;
  private thumbsId = 0;
  // What a rebuilt session needs: how to set the image again, and the last preset and speeds a
  // reply carried.
  private source: ((s: Session) => void) | null = null;
  private saved: { preset: string; speeds: string } | null = null;
  private readonly methods: Record<string, (...args: any[]) => unknown>;

  constructor(private session: Session, private readonly post: (msg: Reply | Event | ThumbEvent, transfer?: Transferable[]) => void) {
    // Every call reads `this.session`, which `recover` replaces.
    const s = (): Session => this.session;
    this.methods = {
      listStyles: () => JSON.parse(listStyles()),
      listPens: () => JSON.parse(listPens()),
      listInks: () => JSON.parse(listInks()),
      setMachine: (machine: Machine | null) => s().setMachine(JSON.stringify(machine)),
      gcode: (layer: number) => s().gcode(layer),
      describe: (name: string) => JSON.parse(describe(name)),
      setImageBytes: (bytes: Uint8Array) => this.setSource((t) => t.setImageBytes(bytes)),
      setImage: (width: number, height: number, rgb: Float32Array) => this.setSource((t) => t.setImage(width, height, rgb)),
      // The default A4 drawing area's shape, 190 x 277 mm, so the sample fills it.
      sample: () => this.methods.setImage(600, 875, syntheticStillLife(600, 875)),
      setCanvas: (canvas: Canvas) => s().setCanvas(JSON.stringify(canvas)),
      setPens: (pens: Pen[]) => s().setPens(JSON.stringify(pens)),
      setStyle: (name: string) => s().setStyle(name),
      applyLook: (name: string) => s().applyLook(name),
      resetSettings: () => s().resetSettings(),
      update: (stage: string, values: Record<string, unknown>) => s().update(stage, JSON.stringify(values)),
      // The session calls, not `methods`, so the whole patch redraws once. A refusal partway leaves
      // the earlier parts applied; only a stale suggestion can cause one.
      apply: ({ style, pens, settings, page }: Patch) => {
        if (style) s().setStyle(style);
        if (pens) s().setPens(JSON.stringify(pens));
        for (const [stage, values] of Object.entries(settings ?? {})) s().update(stage, JSON.stringify(values));
        if (page) s().setCanvas(JSON.stringify({ ...JSON.parse(s().preset()).canvas, ...page }));
      },
      setSpeeds: (speeds: Speeds) => s().setSpeeds(JSON.stringify(speeds)),
      run: () => this.start(),
      // The settings stay as they are: the page applies the value the run's end carries.
      fit: (metric: string, target: number) => this.start(() => s().fit(metric, target), "fit"),
      // The settings stay as they are, and the page only offers what the search found.
      optimize: (metric?: string, target?: number) => this.start(() => s().optimize(metric ?? null, target ?? null), "optimize"),
      cancel: () => this.current?.cancel(),
      // The share of the image, from its top-left corner, under a page point in mm; null off the image.
      imagePoint: (x: number, y: number) => {
        const p = s().imagePoint(x, y);
        return p ? Array.from(p) : null;
      },
      // The calibration chart for the page and the first pen. The engine cancels the run, and the
      // events a finished run posts follow the reply. A refusal changes nothing.
      testchart: () => {
        s().testchart();
        const id = ++this.counter;
        this.current = null;
        this.go = () => postDone(s(), id, this.post);
      },
      preset: () => JSON.parse(s().preset()),
      loadPreset: (preset: unknown) => s().loadPreset(JSON.stringify(preset)),
      // The text of a preset file, an exported SVG or an exported JSON drawing.
      loadPresetText: (text: string) => s().loadPreset(presetFromText(text)),
      // A setup in the preset's shape over the current preset, so the format and version stay ours.
      restore: (setup: Record<string, unknown>) => s().loadPreset(JSON.stringify({ ...JSON.parse(s().preset()), ...setup })),
      // Draws every style after the reply, which carries the id its `thumb` events will have.
      thumbs: (width: number) => {
        const id = ++this.thumbsId;
        this.go = () => void this.thumbs(id, width);
        return id;
      },
      cancelThumbs: () => { this.thumbsId++; },
      svg: () => s().svg(),
      json: () => s().json(),
    };
  }

  handle({ id, method, args }: Command): void {
    try {
      if (!Object.hasOwn(this.methods, method)) throw new Error(`unknown method: ${method}`);
      const value = this.methods[method](...args);
      if (REDRAWS.has(method) && this.image) this.start();
      this.post({ id, ok: true, state: this.state(), value });
    } catch (e) {
      this.post({ id, ok: false, error: trapped(e) ? this.recover(e, true) : message(e) });
    }
    this.release();
  }

  // Lets a run that `start` set up go.
  private release(): void {
    const go = this.go;
    this.go = null;
    go?.();
  }

  private setSource(set: (s: Session) => void): void {
    set(this.session);
    this.source = set;
    this.image = true;
  }

  // Rebuilds the session after a trap, which leaves its objects borrowed so that every later call
  // on them fails. A new session gets the image again and the last preset and speeds a reply
  // carried, and redraws unless the run that trapped was the latest. Returns the panic's message,
  // which the trap itself does not carry.
  private recover(e: unknown, redraw: boolean): string {
    const error = takePanic() ?? message(e);
    const old = this.session;
    try {
      this.current?.cancel(); // the old session's run would otherwise draw on
    } catch {
      // the run that trapped is borrowed, and its loop has already ended
    }
    this.current = null;
    this.session = new Session();
    try {
      this.source?.(this.session);
      if (this.saved) {
        this.session.loadPreset(this.saved.preset);
        this.session.setSpeeds(this.saved.speeds);
      }
      if (redraw && this.image) this.start();
    } catch {
      // the new session keeps what was set before this failed
    }
    free(old);
    return error;
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
        .catch((e) => {
          const error = trapped(e) ? this.recover(e, this.current !== run) : message(e);
          this.post({ type: "end", id, state: "error", error, stats: null, tone: null, lint: [] }, []);
          this.release(); // the redraw, when an older run trapped
        })
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
            free(frame);
            if (performance.now() - lastYield >= 50) {
              await macrotask();
              lastYield = performance.now();
              if (id !== this.thumbsId) return;
            }
          }
          error = trial.error() ?? null;
          png = trial.previewPng(width) ?? null;
        } finally {
          free(trial);
        }
      } catch (e) {
        if (trapped(e)) {
          error = this.recover(e, true);
          this.release();
        } else {
          error = message(e);
        }
      }
      this.post({ type: "thumb", id, style: name, png, error }, png ? [png.buffer as ArrayBuffer] : []);
    }
  }

  private state(): State {
    const s = this.session;
    const preset = s.preset();
    this.saved = { preset, speeds: s.speeds() };
    const { style, canvas, pens, machine, settings } = JSON.parse(preset);
    const stats = s.stats();
    const layers: Pen[] = [];
    const drawing = s.drawing();
    if (drawing) {
      try {
        for (let i = 0; i < drawing.layerCount(); i++) layers.push(JSON.parse(drawing.layerPen(i)));
      } finally {
        free(drawing);
      }
    }
    const analysis = s.analysis();
    const lint = s.lint();
    const tone = s.toneMatch();
    const focus: [number, number][] = [];
    for (const [x, y] of (settings.tone?.focus ?? []) as [number, number][]) {
      const p = s.pagePoint(x, y);
      if (p) focus.push([p[0], p[1]]);
    }
    return {
      image: this.image, style, look: s.look() ?? null, canvas, pens, machine, settings, speeds: JSON.parse(this.saved.speeds), layers,
      suggestions: analysis ? JSON.parse(analysis).suggestions : [],
      lint: lint ? JSON.parse(lint) : [],
      tone: tone ? JSON.parse(tone) : null,
      focus,
      run: { id: this.counter, state: s.state(), error: s.error() ?? null },
      stats: stats ? JSON.parse(stats) : null,
    };
  }
}
