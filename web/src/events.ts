export type Pen = { name: string; width_mm: number; color: string };
/** A pen from the bank, with the kind and model it is grouped by. */
export type BankPen = Pen & { kind: string; model: string };
/** A common ink colour, offered when a pen's colour is picked. */
export type Ink = { name: string; color: string };
/** `crop` is the shares of the image cut from its left, top, right and bottom; the engine leaves it out while all are zero. */
export type Canvas = { width_mm: number; height_mm: number; margin_mm: number; fit: "fit" | "fill"; crop?: [number, number, number, number] };
export type Tone = { x: number; y: number; w: number; h: number; width_px: number; height_px: number; ink: Uint8Array };
export type Layer = { pen: Pen; coords: Float64Array; offsets: Uint32Array };
export type Stats = { path_count: number; pen_down_mm: number; pen_up_mm: number; estimated_seconds: number };
/** A plot lint finding, in the shape of `Session::lint`; `patch` applies like a suggestion's. */
export type Finding = {
  code: string; severity: "error" | "warning" | "info"; message: string; value: number;
  patch: { style?: string; settings?: Record<string, Record<string, unknown>>; page?: { width_mm: number; height_mm: number } } | null;
};
export type RunEvent = { type: "run"; id: number };
export type FrameEvent = { type: "frame"; id: number; stage: string; progress: number; label: string; pen: number; points?: Float64Array; coords?: Float64Array; offsets?: Uint32Array };
export type ResultEvent = { type: "result"; id: number; canvas: Canvas; layers: Layer[]; tone: Tone | null };
/** What a fit chose, in the shape of the engine's `FitResult`. */
export type FitResult = {
  stage: string; key: string; metric: string; target: number; value: unknown; achieved: number;
  samples: { value: number; metric: number }[];
};
/** How the drawing's tones follow its tone map, in the shape of the engine's `ToneMatch`. */
export type ToneMatch = { error: number; ssim: number; floor: number; reach: number };
/** What an optimize search found, in the shape of the engine's `Optimum`; nothing is applied. */
export type Optimum = {
  stage: string; key: string; value: unknown; error: number; minutes: number;
  current: unknown; current_error: number | null; current_minutes: number;
  budget: { metric: string; target: number } | null;
  trials: {
    value: number; error: number | null; minutes: number | null; metric: number | null;
    over: boolean; slow: boolean; warnings: string[]; failed: string | null;
  }[];
  reason: string; patch: Finding["patch"];
};
/** `fit` is set when a fit ends done, which the page applies with `update`, and `optimum` when a search does, which it only offers. */
export type EndEvent = {
  type: "end"; id: number; state: "done" | "cancelled" | "error"; error: string | null; stats: Stats | null;
  tone: ToneMatch | null; fit?: FitResult | null; optimum?: Optimum | null; lint: Finding[];
};
export type Event = RunEvent | FrameEvent | ResultEvent | EndEvent;
/** One style drawn for the styles dialog: its preview as PNG bytes, or why it failed. */
export type ThumbEvent = { type: "thumb"; id: number; style: string; png: Uint8Array | null; error: string | null };
export type Area = { x: number; y: number; width: number; height: number };
/** The engine leaves out the last four fields while they hold their defaults: Z, "", "", none. */
export type Machine = {
  name: string; area: Area; z_down_mm: number; z_up_mm: number; z_safe_mm: number;
  draw_feed: number; travel_feed: number; z_feed: number; settle_ms: number; start_gcode: string; end_gcode: string;
  pen_lift?: "z" | "gcode"; pen_down_gcode?: string; pen_up_gcode?: string; max_segment_mm?: number | null;
};
