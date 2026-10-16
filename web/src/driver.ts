// The run loop of spec 2026-10-12-gui-design section 5: pulls frames from one engine run and
// posts them as events, throttled, yielding so commands such as a new `start` get their turn.
import type { Run, Session } from "../pkg/plotline_wasm.js";
import type { EndEvent, Event, FitResult, FrameEvent, Layer, Optimum, ResultEvent } from "./events";

export type Post = (event: Event, transfer: Transferable[]) => void;

// One channel for the whole worker, one handler: Node counts every new listener as a reason to
// stay alive, so `unref` only sticks if the listener is added once. Browsers have no `unref`.
const waiting: Array<() => void> = [];
let port: MessagePort | undefined;

// A zero-delay macrotask. A message round trip, not `setTimeout`, which browsers clamp to 4 ms
// once nested.
export function macrotask(): Promise<void> {
  if (!port) {
    const channel = new MessageChannel();
    channel.port1.onmessage = () => waiting.shift()?.();
    (channel.port1 as { unref?: () => void }).unref?.();
    port = channel.port2;
  }
  const next = port;
  return new Promise((resolve) => { waiting.push(resolve); next.postMessage(null); });
}

// Frees an engine object. A trap inside a call on it leaves it borrowed for good, and freeing it
// then throws an error that would hide the trap's, so that error is dropped.
export function free(o: { free(): void }): void {
  try {
    o.free();
  } catch {
    // left to leak: the session is rebuilt after a trap
  }
}

// The engine returns fresh arrays, so their buffers can move to the page without a copy.
const buffers = (arrays: Array<ArrayBufferView | undefined>): Transferable[] =>
  arrays.flatMap((a) => (a ? [a.buffer as ArrayBuffer] : []));

export function postResult(session: Session, id: number, post: Post): void {
  const drawing = session.drawing();
  if (!drawing) return;
  try {
    const layers: Layer[] = [];
    for (let i = 0; i < drawing.layerCount(); i++) {
      layers.push({ pen: JSON.parse(drawing.layerPen(i)), coords: drawing.layerCoords(i), offsets: drawing.layerOffsets(i) });
    }
    const rect = drawing.toneRect();
    const ink = drawing.toneInk();
    const event: ResultEvent = {
      type: "result", id, canvas: JSON.parse(drawing.canvas()), layers,
      tone: rect && ink ? { ...JSON.parse(rect), ink } : null,
    };
    post(event, buffers([...layers.flatMap((l) => [l.coords, l.offsets]), ink]));
  } finally {
    free(drawing);
  }
}

// Posts `run`, every throttled `frame`, the `result` when the run is done, and one `end`, which
// carries the tone match and what a fit chose or a search found, as `kind` says the run is.
// Frees the run on every path. A later `session.start()` ends this loop with `cancelled`.
export async function drive(
  session: Session, run: Run, id: number, post: Post,
  opts: { frameMs?: number; yieldMs?: number; now?: () => number; yieldFn?: () => Promise<void>; kind?: "run" | "fit" | "optimize" } = {},
): Promise<void> {
  const { frameMs = 33, yieldMs = 50, now = () => performance.now(), yieldFn = macrotask, kind = "run" } = opts;
  try {
    post({ type: "run", id }, []);
    let lastPost = -Infinity;
    let lastYield = now();
    for (let frame = run.nextFrame(); frame; frame = run.nextFrame()) {
      try {
        const t = now();
        const progress = frame.progress();
        if (progress >= 1 || t - lastPost >= frameMs) {
          const geometry = { points: frame.points(), coords: frame.coords(), offsets: frame.offsets() };
          const event: FrameEvent = { type: "frame", id, stage: frame.stage(), progress, label: frame.label(), pen: frame.pen(), ...geometry };
          post(event, buffers(Object.values(geometry)));
          lastPost = t;
        }
      } finally {
        free(frame);
      }
      if (now() - lastYield >= yieldMs) {
        await yieldFn();
        lastYield = now();
      }
    }
    const state = run.state() as EndEvent["state"];
    if (state === "done") postResult(session, id, post);
    const found = run.result();
    const parsed = found ? JSON.parse(found) : null;
    postEnd(session, id, post, state, run.error() ?? null, kind === "fit" ? parsed : null, kind === "optimize" ? parsed : null);
  } finally {
    free(run);
  }
}

function postEnd(
  session: Session, id: number, post: Post, state: EndEvent["state"], error: string | null, fit: FitResult | null, optimum: Optimum | null,
): void {
  const stats = session.stats();
  const tone = session.toneMatch();
  const lint = session.lint();
  post({
    type: "end", id, state, error, stats: stats ? JSON.parse(stats) : null, tone: tone ? JSON.parse(tone) : null,
    fit, optimum, lint: lint ? JSON.parse(lint) : [],
  }, []);
}

// Posts the events of a run that is already done, such as the test chart's: `run`, `result` and `end`.
export function postDone(session: Session, id: number, post: Post): void {
  post({ type: "run", id }, []);
  postResult(session, id, post);
  postEnd(session, id, post, "done", null, null, null);
}
