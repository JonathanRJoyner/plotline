import type { JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { Canvas, FrameEvent, Layer, Pen, ResultEvent } from "./events";
import { eachPath, pathCount } from "./paths";
import { fitView, zoomAt, type View } from "./view";

type Props = {
  canvas: Canvas;
  // The latest frame with geometry from each pen of the run, by pen index; empty before the first.
  frames: (FrameEvent | undefined)[];
  result: ResultEvent | null;
  running: boolean;
  showSource: boolean;
  showTravel: boolean;
  hidden: Set<number>;
  pens: Pen[];
  hasImage: boolean;
  // The view lives with the toolbar, which shows its zoom; `fits` counts the toolbar's Fit clicks.
  view: View;
  onView: (update: (view: View) => View) => void;
  fits: number;
};

// A path object past about a million subpaths draws nothing, so long layers stroke in chunks.
const CHUNK = 200_000;

// Adds vertices a..b of `c` (x, y pairs) to `p`. A one-vertex path becomes a 0.1 µm segment,
// which the round cap draws as a dot; the canvas spec does not promise a dot for a zero-length one.
function trace(p: Path2D, c: Float64Array, a: number, b: number) {
  if (b <= a) return;
  p.moveTo(c[2 * a], c[2 * a + 1]);
  if (b - a === 1) p.lineTo(c[2 * a] + 1e-4, c[2 * a + 1]);
  for (let v = a + 1; v < b; v++) p.lineTo(c[2 * v], c[2 * v + 1]);
}

// Draws everything in page millimetres, so a line width is a length on the paper. While running,
// each pen's latest frame stands in for the result and is drawn in that pen, so a finished pen's
// lines stay while the next pen's frames arrive.
function paint(ctx: CanvasRenderingContext2D, s: Props, view: View, w: number, h: number, dpr: number, bitmap: ImageBitmap | null) {
  const { result, pens } = s;
  // Until the first frame with geometry arrives the previous result stays, on the page it was made for.
  const live = s.running ? s.frames : [];
  const held = live.some(Boolean);
  const page = !held && result ? result.canvas : s.canvas;
  const cv = ctx.canvas;
  if (cv.width !== Math.round(w * dpr)) cv.width = Math.round(w * dpr);
  if (cv.height !== Math.round(h * dpr)) cv.height = Math.round(h * dpr);
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, cv.width, cv.height);
  ctx.setTransform(dpr * view.scale, 0, 0, dpr * view.scale, dpr * view.x, dpr * view.y);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  const stroke = (pen: Pen, p: Path2D) => {
    ctx.strokeStyle = pen.color;
    ctx.lineWidth = pen.width_mm;
    ctx.stroke(p);
  };
  // Strokes `n` paths with one pen, `add` putting path i into the chunk's path object.
  const strokeChunks = (pen: Pen, n: number, add: (p: Path2D, i: number) => void) => {
    ctx.strokeStyle = pen.color;
    ctx.lineWidth = pen.width_mm;
    for (let i = 0; i < n; i += CHUNK) {
      const p = new Path2D();
      for (let j = i; j < Math.min(n, i + CHUNK); j++) add(p, j);
      ctx.stroke(p);
    }
  };

  ctx.save();
  ctx.shadowColor = "rgba(0, 0, 0, 0.35)";
  ctx.shadowBlur = 8 * dpr;
  ctx.shadowOffsetY = 2 * dpr;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, page.width_mm, page.height_mm);
  ctx.restore();

  ctx.strokeStyle = "#ccc";
  ctx.lineWidth = 1 / view.scale;
  const m = page.margin_mm;
  ctx.strokeRect(m, m, page.width_mm - 2 * m, page.height_mm - 2 * m);

  if (s.showSource && bitmap && result?.tone) {
    const t = result.tone;
    ctx.globalAlpha = 0.3;
    ctx.drawImage(bitmap, t.x, t.y, t.w, t.h);
    ctx.globalAlpha = 1;
  }

  // A frame of a pen the list no longer has is drawn with the first pen.
  const penOf = (i: number) => pens[i] ?? pens[0];
  const layers: Layer[] = held
    ? live.flatMap((f, i) => (f?.coords && f.offsets && !s.hidden.has(i) ? [{ pen: penOf(i), coords: f.coords, offsets: f.offsets }] : []))
    : (result?.layers ?? []).filter((_, i) => !s.hidden.has(i));
  // Multiply, so pens that overlap darken like layered transparent inks on the plot; one pen on
  // white looks the same. The rest of the page draws normally.
  ctx.save();
  ctx.globalCompositeOperation = "multiply";
  live.forEach((f, i) => {
    const pts = f?.points;
    if (pts && !s.hidden.has(i)) strokeChunks(penOf(i), pts.length / 2, (p, k) => trace(p, pts, k, k + 1));
  });
  for (const l of layers) strokeChunks(l.pen, pathCount(l.offsets), (p, i) => trace(p, l.coords, l.offsets[i], l.offsets[i + 1]));
  ctx.restore();

  if (s.showTravel && !held) {
    const p = new Path2D();
    let last: [number, number] | null = null;
    for (const l of layers) {
      eachPath(l.coords, l.offsets, (a, b) => {
        if (b <= a) return;
        if (last) {
          p.moveTo(last[0], last[1]);
          p.lineTo(l.coords[2 * a], l.coords[2 * a + 1]);
        }
        last = [l.coords[2 * b - 2], l.coords[2 * b - 1]];
      });
    }
    stroke({ name: "travel", color: "red", width_mm: 1 / view.scale }, p);
  }

  if (!s.hasImage) {
    ctx.fillStyle = "#888";
    ctx.font = "6px sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("Open, drop or paste an image to start", page.width_mm / 2, page.height_mm / 2);
  }
}

export function Preview(props: Props): JSX.Element {
  const { canvas, result, view, onView: setView, fits } = props;
  const box = useRef<HTMLDivElement>(null);
  const cvs = useRef<HTMLCanvasElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [tone, setTone] = useState<{ result: ResultEvent; bitmap: ImageBitmap } | null>(null);
  const bitmap = tone?.result === result ? tone.bitmap : null; // a closed bitmap never matches the new result

  const fit = (w: number, h: number) => setView(() => fitView(canvas.width_mm, canvas.height_mm, w, h));

  // The observer fires on observe, so this also fits on mount, when the page size changes and on
  // every Fit click.
  useEffect(() => {
    const el = box.current!;
    const ro = new ResizeObserver(() => {
      setSize({ w: el.clientWidth, h: el.clientHeight });
      fit(el.clientWidth, el.clientHeight);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [canvas.width_mm, canvas.height_mm, fits]);

  // One bitmap per result: the tone's grey bytes as opaque RGBA.
  useEffect(() => {
    if (!result?.tone) return;
    const { ink, width_px, height_px } = result.tone;
    const rgba = new Uint8ClampedArray(ink.length * 4);
    for (let i = 0; i < ink.length; i++) {
      rgba[4 * i] = rgba[4 * i + 1] = rgba[4 * i + 2] = ink[i];
      rgba[4 * i + 3] = 255;
    }
    let dead = false;
    let made: ImageBitmap | undefined;
    createImageBitmap(new ImageData(rgba, width_px, height_px)).then((b) => {
      if (dead) return b.close();
      made = b;
      setTone({ result, bitmap: b });
    });
    return () => {
      dead = true;
      made?.close();
    };
  }, [result]);

  // One animation frame per change; a newer change cancels the pending one.
  useEffect(() => {
    const id = requestAnimationFrame(() => {
      const ctx = cvs.current!.getContext("2d")!;
      paint(ctx, props, view, size.w, size.h, window.devicePixelRatio || 1, bitmap);
    });
    return () => cancelAnimationFrame(id);
  }, [props.canvas, props.frames, props.result, props.running, props.showSource, props.showTravel, props.hidden, props.pens, props.hasImage, view, size, bitmap]);

  // Zooms by `k`, keeping the page point under (px, py) in place.
  const zoom = (k: number, px: number, py: number) => setView((v) => zoomAt(v, Math.min(200, Math.max(0.05, v.scale * k)) / v.scale, px, py));
  const PAN: Record<string, [number, number]> = { ArrowLeft: [40, 0], ArrowRight: [-40, 0], ArrowUp: [0, 40], ArrowDown: [0, -40] };

  return (
    <div ref={box} style={{ position: "relative", height: "100%" }}>
      <canvas
        ref={cvs}
        tabIndex={0}
        aria-label="Preview of the drawing: arrow keys pan, plus and minus zoom, 0 fits the page"
        style={{ position: "absolute", inset: 0, width: "100%", height: "100%", touchAction: "none", cursor: "grab" }}
        onKeyDown={(e) => {
          if (e.key === "+" || e.key === "=") zoom(1.25, size.w / 2, size.h / 2);
          else if (e.key === "-") zoom(0.8, size.w / 2, size.h / 2);
          else if (e.key === "0") fit(size.w, size.h);
          else if (PAN[e.key]) setView((v) => ({ ...v, x: v.x + PAN[e.key][0], y: v.y + PAN[e.key][1] }));
          else return;
          e.preventDefault();
        }}
        onWheel={(e) => {
          e.preventDefault();
          const r = e.currentTarget.getBoundingClientRect();
          // In proportion to the delta, so a trackpad's stream of small deltas zooms gradually.
          zoom(Math.exp(-e.deltaY * 0.002), e.clientX - r.left, e.clientY - r.top);
        }}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = { x: e.clientX, y: e.clientY };
        }}
        onPointerMove={(e) => {
          const d = drag.current;
          if (!d) return;
          const dx = e.clientX - d.x;
          const dy = e.clientY - d.y;
          drag.current = { x: e.clientX, y: e.clientY };
          setView((v) => ({ ...v, x: v.x + dx, y: v.y + dy }));
        }}
        onPointerUp={() => (drag.current = null)}
        onPointerCancel={() => (drag.current = null)}
      />
    </div>
  );
}
