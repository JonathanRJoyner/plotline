import type { JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { dividerX, type Compare } from "./compare";
import type { Canvas, FrameEvent, Layer, Pen, ResultEvent } from "./events";
import { eachPath, pathCount } from "./paths";
import { fitView, onPaper, toMm, zoomAt, type View } from "./view";

type Props = {
  canvas: Canvas;
  // The paper's colour as #rrggbb.
  paper: string;
  // The latest frame with geometry from each pen of the run, by pen index; empty before the first.
  frames: (FrameEvent | undefined)[];
  result: ResultEvent | null;
  running: boolean;
  showSource: boolean;
  showTravel: boolean;
  // The swipe against the previous run's drawing or the source image, or null for none.
  compare: Compare | null;
  // The result the latest one replaced, for `compare`; null before a second run.
  previous: ResultEvent | null;
  hidden: Set<number>;
  pens: Pen[];
  hasImage: boolean;
  // The view lives with the toolbar, which shows its zoom; `fits` counts the toolbar's Fit clicks.
  view: View;
  onView: (update: (view: View) => View) => void;
  fits: number;
  // The focus points as page mm, each drawn as a ring of `radius` mm. In focus mode a click on the
  // page asks for a point there, and a click on a point asks for its removal; dragging still pans.
  focus: [number, number][];
  radius: number;
  focusing: boolean;
  onFocusAt: (mm: [number, number]) => void;
  onFocusRemove: (index: number) => void;
};

// A press that moves less than this many pixels before it ends is a click, not a pan.
const CLICK_PX = 4;

// A path object past about a million subpaths draws nothing, so long layers stroke in chunks.
const CHUNK = 200_000;

// The lines and the dots of a chunk, and the radius of a dot, half the pen's width.
export type Chunk = { lines: Path2D; dots: Path2D; r: number };

// Adds vertices a..b of `c` (x, y pairs) to the chunk. A one-vertex path is a pen-wide disc,
// filled rather than stroked: Chrome drew no round cap on a near-zero segment under about a pixel
// wide, so a dot style's page went blank at overview zooms while lines still showed.
export function trace({ lines, dots, r }: Chunk, c: Float64Array, a: number, b: number) {
  if (b <= a) return;
  if (b - a === 1) {
    dots.moveTo(c[2 * a] + r, c[2 * a + 1]);
    dots.arc(c[2 * a], c[2 * a + 1], r, 0, 2 * Math.PI);
    return;
  }
  lines.moveTo(c[2 * a], c[2 * a + 1]);
  for (let v = a + 1; v < b; v++) lines.lineTo(c[2 * v], c[2 * v + 1]);
}

// Draws `n` paths with one pen, `add` putting path i into the chunk.
function strokeChunks(ctx: CanvasRenderingContext2D, pen: Pen, n: number, add: (chunk: Chunk, i: number) => void) {
  ctx.strokeStyle = pen.color;
  ctx.fillStyle = pen.color;
  ctx.lineWidth = pen.width_mm;
  for (let i = 0; i < n; i += CHUNK) {
    const chunk = { lines: new Path2D(), dots: new Path2D(), r: pen.width_mm / 2 };
    for (let j = i; j < Math.min(n, i + CHUNK); j++) add(chunk, j);
    ctx.stroke(chunk.lines);
    ctx.fill(chunk.dots);
  }
}

// Multiply, so pens that overlap darken like layered transparent inks on the plot; one pen on
// white looks the same.
function strokeLayers(ctx: CanvasRenderingContext2D, layers: Layer[]) {
  ctx.save();
  ctx.globalCompositeOperation = "multiply";
  for (const l of layers) strokeChunks(ctx, l.pen, pathCount(l.offsets), (chunk, i) => trace(chunk, l.coords, l.offsets[i], l.offsets[i + 1]));
  ctx.restore();
}

// The paper with its shadow, and the margin's outline, in page millimetres.
function drawPaper(ctx: CanvasRenderingContext2D, page: Canvas, paper: string, scale: number, dpr: number) {
  ctx.save();
  ctx.shadowColor = "rgba(0, 0, 0, 0.35)";
  ctx.shadowBlur = 8 * dpr;
  ctx.shadowOffsetY = 2 * dpr;
  ctx.fillStyle = paper;
  ctx.fillRect(0, 0, page.width_mm, page.height_mm);
  ctx.restore();

  ctx.strokeStyle = "#ccc";
  ctx.lineWidth = 1 / scale;
  const m = page.margin_mm;
  ctx.strokeRect(m, m, page.width_mm - 2 * m, page.height_mm - 2 * m);
}

// A label in a dark pill, so it reads on the white paper and on either theme's backdrop; its right
// edge is at `x` when `end`, else its left. Skipped when it would run off the canvas. In device pixels.
function drawLabel(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, end: boolean, dpr: number) {
  ctx.font = `${12 * dpr}px system-ui, sans-serif`;
  const pad = 6 * dpr;
  const w = ctx.measureText(text).width + 2 * pad;
  const left = end ? x - w : x;
  if (left < 0 || left + w > ctx.canvas.width) return;
  ctx.fillStyle = "rgba(23, 26, 32, 0.8)";
  ctx.beginPath();
  ctx.roundRect(left, y, w, 20 * dpr, 4 * dpr);
  ctx.fill();
  ctx.strokeStyle = "rgba(255, 255, 255, 0.35)"; // sets the pill off from a dark backdrop
  ctx.lineWidth = dpr;
  ctx.stroke();
  ctx.fillStyle = "#fff";
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";
  ctx.fillText(text, left + pad, y + 10 * dpr);
}

// Redraws the part of the canvas left of the divider with the comparison, then draws the divider
// and the labels. The comparison is the previous result's layers on its own page, or the tone map
// at full opacity on the current one. The view's transform is in place on entry and on return.
function paintCompare(ctx: CanvasRenderingContext2D, s: Props, c: Compare, bitmap: ImageBitmap | null, dpr: number) {
  const { width, height } = ctx.canvas;
  const x = dividerX(c.at, width);
  const other = c.against === "previous" ? s.previous : s.result;
  ctx.save();
  const view = ctx.getTransform();
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.beginPath();
  ctx.rect(0, 0, x, height);
  ctx.clip();
  ctx.clearRect(0, 0, width, height);
  ctx.setTransform(view);
  if (other) {
    drawPaper(ctx, other.canvas, s.paper, view.a / dpr, dpr);
    if (c.against === "previous") strokeLayers(ctx, other.layers.filter((_, i) => !s.hidden.has(i)));
    else if (bitmap && other.tone) ctx.drawImage(bitmap, other.tone.x, other.tone.y, other.tone.w, other.tone.h);
  }
  ctx.restore();

  ctx.save();
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  // The accent token resolved for the theme: accent-color inherits from the root, where it is set to the token.
  const accent = getComputedStyle(ctx.canvas).accentColor;
  ctx.fillStyle = accent && accent !== "auto" ? accent : "#4f46e5";
  ctx.fillRect(Math.min(Math.max(Math.round(x), 1), width - 1) - 1, 0, 2, height);
  // Below the menu that sits over the preview's top left corner.
  const top = 54 * dpr;
  drawLabel(ctx, c.against === "previous" ? "previous" : "source", x - 8 * dpr, top, true, dpr);
  drawLabel(ctx, "now", x + 8 * dpr, top, false, dpr);
  ctx.restore();
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

  drawPaper(ctx, page, s.paper, view.scale, dpr);

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
  // Multiply, so pens that overlap darken like layered transparent inks on the plot, and ink takes
  // the paper's colour; one pen on white looks the same. The rest of the page draws normally.
  ctx.save();
  ctx.globalCompositeOperation = "multiply";
  live.forEach((f, i) => {
    const pts = f?.points;
    if (pts && !s.hidden.has(i)) strokeChunks(ctx, penOf(i), pts.length / 2, (chunk, k) => trace(chunk, pts, k, k + 1));
  });
  ctx.restore();
  strokeLayers(ctx, layers);

  if (s.showTravel && !held) {
    const travel = { name: "travel", color: "red", width_mm: 1 / view.scale };
    let p = new Path2D();
    let moves = 0;
    let last: [number, number] | null = null;
    for (const l of layers) {
      eachPath(l.coords, l.offsets, (a, b) => {
        if (b <= a) return;
        if (last) {
          p.moveTo(last[0], last[1]);
          p.lineTo(l.coords[2 * a], l.coords[2 * a + 1]);
          // In chunks like the layers, or a drawing of a million paths shows no travel at all.
          if (++moves % CHUNK === 0) {
            stroke(travel, p);
            p = new Path2D();
          }
        }
        last = [l.coords[2 * b - 2], l.coords[2 * b - 1]];
      });
    }
    stroke(travel, p);
  }

  if (!s.hasImage) {
    ctx.fillStyle = onPaper(s.paper);
    ctx.font = "6px sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("Open, drop or paste an image to start", page.width_mm / 2, page.height_mm / 2);
  }

  if (s.compare && !s.running) paintCompare(ctx, s, s.compare, bitmap, dpr);
}

export function Preview(props: Props): JSX.Element {
  const { canvas, result, view, onView: setView, fits, focus, radius, focusing, onFocusAt, onFocusRemove } = props;
  const box = useRef<HTMLDivElement>(null);
  const cvs = useRef<HTMLCanvasElement>(null);
  const drag = useRef<{ x: number; y: number; travel: number } | null>(null);
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
  }, [props.canvas, props.paper, props.frames, props.result, props.running, props.showSource, props.showTravel, props.compare?.against, props.compare?.at, props.previous, props.hidden, props.pens, props.hasImage, view, size, bitmap]);

  // Zooms by `k`, keeping the page point under (px, py) in place.
  const zoom = (k: number, px: number, py: number) => setView((v) => zoomAt(v, Math.min(200, Math.max(0.05, v.scale * k)) / v.scale, px, py));
  const PAN: Record<string, [number, number]> = { ArrowLeft: [40, 0], ArrowRight: [-40, 0], ArrowUp: [0, 40], ArrowDown: [0, -40] };

  return (
    <div ref={box} style={{ position: "relative", height: "100%" }}>
      <canvas
        ref={cvs}
        tabIndex={0}
        aria-label="Preview of the drawing: arrow keys pan, plus and minus zoom, 0 fits the page"
        style={{ position: "absolute", inset: 0, width: "100%", height: "100%", touchAction: "none", cursor: focusing ? "crosshair" : "grab" }}
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
          // Firefox may count the delta in lines or pages; a line is taken as 33 px.
          const px = e.deltaMode === 1 ? 33 : e.deltaMode === 2 ? r.height : 1;
          zoom(Math.exp(-e.deltaY * px * 0.002), e.clientX - r.left, e.clientY - r.top);
        }}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = { x: e.clientX, y: e.clientY, travel: 0 };
        }}
        onPointerMove={(e) => {
          const d = drag.current;
          if (!d) return;
          const dx = e.clientX - d.x;
          const dy = e.clientY - d.y;
          drag.current = { x: e.clientX, y: e.clientY, travel: d.travel + Math.hypot(dx, dy) };
          setView((v) => ({ ...v, x: v.x + dx, y: v.y + dy }));
        }}
        onPointerUp={(e) => {
          const d = drag.current;
          drag.current = null;
          if (focusing && d && d.travel < CLICK_PX && e.button === 0) {
            const r = e.currentTarget.getBoundingClientRect();
            onFocusAt(toMm(view, e.clientX - r.left, e.clientY - r.top));
          }
        }}
        onPointerCancel={() => (drag.current = null)}
      />
      {/* Over the canvas in the canvas's own pixels. Only the removal target of each point, while
          focusing, takes the pointer. */}
      <svg class="focus-rings" aria-hidden={focusing ? undefined : "true"}>
        {focus.map(([x, y], i) => {
          const [cx, cy] = [view.x + x * view.scale, view.y + y * view.scale];
          const remove = () => onFocusRemove(i);
          return (
            <g key={i}>
              <circle class="ring-halo" cx={cx} cy={cy} r={radius * view.scale} />
              <circle class="ring" cx={cx} cy={cy} r={radius * view.scale} />
              <circle class="dot" cx={cx} cy={cy} r={3} />
              {focusing && (
                // 3 mm, and at least 8 px, around the point.
                <circle
                  class="hit" cx={cx} cy={cy} r={Math.max(3 * view.scale, 8)} role="button" tabIndex={0}
                  aria-label={`Remove focus point ${i + 1}`} onClick={remove}
                  onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); remove(); } }}
                />
              )}
            </g>
          );
        })}
      </svg>
    </div>
  );
}
