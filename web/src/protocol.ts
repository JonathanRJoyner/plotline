// The command side of the worker (spec 2026-10-12-gui-design section 5): one command in, one
// reply out, and the run events `drive` posts. Every decision lives here so tests can cover it;
// `worker.ts` only connects it to `self`.
import { describe, listInks, listPens, listStyles, syntheticGradient, type Run, type Session } from "../pkg/plotline_wasm.js";
import { drive } from "./driver";
import type { Canvas, Event, Machine, Pen, Stats } from "./events";

export type Speeds = { draw_mm_s: number; travel_mm_s: number; pen_lift_s: number };
export type State = {
  image: boolean; style: string; canvas: Canvas; pens: Pen[]; machine: Machine | null;
  settings: Record<string, Record<string, unknown>>; speeds: Speeds;
  run: { id: number; state: string; error: string | null }; stats: Stats | null;
  /** The pens of the last drawing's layers; empty without a drawing. */
  layers: Pen[];
};
export type Command = { id: number; method: string; args: unknown[] };
export type Reply = { id: number; ok: true; state: State; value?: unknown } | { id: number; ok: false; error: string };

// The commands that change what a drawing is made from: each one redraws once it succeeds.
const REDRAWS = new Set(["setImageBytes", "setImage", "sample", "setCanvas", "setPens", "setStyle", "update", "loadPreset"]);

const message = (e: unknown): string => String((e as { message?: unknown } | null)?.message ?? e);

export class Protocol {
  private image = false;
  private counter = 0;
  private current: Run | null = null;
  private go: (() => void) | null = null;
  private readonly methods: Record<string, (...args: any[]) => unknown>;

  constructor(private readonly session: Session, private readonly post: (msg: Reply | Event, transfer?: Transferable[]) => void) {
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
      sample: () => this.methods.setImage(400, 300, syntheticGradient(400, 300)),
      setCanvas: (canvas: Canvas) => s.setCanvas(JSON.stringify(canvas)),
      setPens: (pens: Pen[]) => s.setPens(JSON.stringify(pens)),
      setStyle: (name: string) => s.setStyle(name),
      update: (stage: string, values: Record<string, unknown>) => s.update(stage, JSON.stringify(values)),
      setSpeeds: (speeds: Speeds) => s.setSpeeds(JSON.stringify(speeds)),
      run: () => this.start(),
      cancel: () => this.current?.cancel(),
      preset: () => JSON.parse(s.preset()),
      loadPreset: (preset: unknown) => s.loadPreset(JSON.stringify(preset)),
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
  private start(): void {
    const run = this.session.start();
    const id = ++this.counter;
    this.current = run;
    this.go = () => {
      drive(this.session, run, id, this.post)
        .catch((e) => this.post({ type: "end", id, state: "error", error: message(e), stats: null }, []))
        .finally(() => { if (this.current === run) this.current = null; }); // `drive` freed it
    };
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
    return {
      image: this.image, style, canvas, pens, machine, settings, speeds: JSON.parse(s.speeds()), layers,
      run: { id: this.counter, state: s.state(), error: s.error() ?? null },
      stats: stats ? JSON.parse(stats) : null,
    };
  }
}
