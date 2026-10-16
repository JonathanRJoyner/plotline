export type Pen = { name: string; width_mm: number; color: string };
/** A pen from the bank, with the kind and model it is grouped by. */
export type BankPen = Pen & { kind: string; model: string };
/** A common ink colour, offered when a pen's colour is picked. */
export type Ink = { name: string; color: string };
export type Canvas = { width_mm: number; height_mm: number; margin_mm: number; fit: "fit" | "fill" };
export type Tone = { x: number; y: number; w: number; h: number; width_px: number; height_px: number; ink: Uint8Array };
export type Layer = { pen: Pen; coords: Float64Array; offsets: Uint32Array };
export type Stats = { path_count: number; pen_down_mm: number; pen_up_mm: number; estimated_seconds: number };
export type RunEvent = { type: "run"; id: number };
export type FrameEvent = { type: "frame"; id: number; stage: string; progress: number; label: string; pen: number; points?: Float64Array; coords?: Float64Array; offsets?: Uint32Array };
export type ResultEvent = { type: "result"; id: number; canvas: Canvas; layers: Layer[]; tone: Tone | null };
export type EndEvent = { type: "end"; id: number; state: "done" | "cancelled" | "error"; error: string | null; stats: Stats | null };
export type Event = RunEvent | FrameEvent | ResultEvent | EndEvent;
/** One style drawn for the styles dialog: its preview as PNG bytes, or why it failed. */
export type ThumbEvent = { type: "thumb"; id: number; style: string; png: Uint8Array | null; error: string | null };
export type Area = { x: number; y: number; width: number; height: number };
export type Machine = {
  name: string; area: Area; z_down_mm: number; z_up_mm: number; z_safe_mm: number;
  draw_feed: number; travel_feed: number; z_feed: number; settle_ms: number; start_gcode: string; end_gcode: string;
};
