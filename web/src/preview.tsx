import type { JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { Canvas, FrameEvent, Layer, Pen, ResultEvent } from "./events";
import { eachPath } from "./paths";
import { fitView, zoomAt, type View } from "./view";

type Props = {
  canvas: Canvas;
  frame: FrameEvent | null;
  result: ResultEvent | null;
  running: boolean;
  showSource: boolean;
  showTravel: boolean;
  hidden: Set<number>;
  firstPen: Pen;
  hasImage: boolean;
};

// Adds vertices a..b of `c` (x, y pairs) to `p`. A one-vertex path is a zero-length
// segment, which the round cap draws as a dot.
function trace(p: Path2D, c: Float64Array, a: number, b: number) {
  if (b <= a) return;
  p.moveTo(c[2 * a], c[2 * a + 1]);
  if (b - a === 1) p.lineTo(c[2 * a], c[2 * a + 1]);
  for (let v = a + 1; v < b; v++) p.lineTo(c[2 * v], c[2 * v + 1]);
}

// Draws everything in page millimetres, so a line width is a length on the paper.
function paint(ctx: CanvasRenderingContext2D, s: Props, view: View, w: number, h: number, dpr: number, bitmap: ImageBitmap | null) {
  const { canvas: page, result, firstPen } = s;
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

  // While running, the latest frame stands in for the result and is drawn with the first pen;
  // until the first frame arrives the previous result stays.
  const f = s.running ? s.frame : null;
  const layers: Layer[] = f
    ? f.coords && f.offsets ? [{ pen: firstPen, coords: f.coords, offsets: f.offsets }] : []
    : (result?.layers ?? []).filter((_, i) => !s.hidden.has(i));
  if (f?.points) {
    const p = new Path2D();
    for (let i = 0; i < f.points.length / 2; i++) trace(p, f.points, i, i + 1);
    stroke(firstPen, p);
  }
  for (const l of layers) {
    const p = new Path2D();
    eachPath(l.coords, l.offsets, (a, b) => trace(p, l.coords, a, b));
    stroke(l.pen, p);
  }

  if (s.showTravel && !f) {
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
    ctx.fillText("Open an image to start", page.width_mm / 2, page.height_mm / 2);
  }
}

export function Preview(props: Props): JSX.Element {
  const { canvas, result } = props;
  const box = useRef<HTMLDivElement>(null);
  const cvs = useRef<HTMLCanvasElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View>({ scale: 1, x: 0, y: 0 });
  const [tone, setTone] = useState<{ result: ResultEvent; bitmap: ImageBitmap } | null>(null);
  const bitmap = tone?.result === result ? tone.bitmap : null; // a closed bitmap never matches the new result

  const fit = (w = size.w, h = size.h) => setView(fitView(canvas.width_mm, canvas.height_mm, w, h));

  // The observer fires on observe, so this also fits on mount and when the page size changes.
  useEffect(() => {
    const el = box.current!;
    const ro = new ResizeObserver(() => {
      setSize({ w: el.clientWidth, h: el.clientHeight });
      fit(el.clientWidth, el.clientHeight);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [canvas.width_mm, canvas.height_mm]);

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
  }, [props.canvas, props.frame, props.result, props.running, props.showSource, props.showTravel, props.hidden, props.firstPen, props.hasImage, view, size, bitmap]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <button type="button" style={{ alignSelf: "flex-start" }} onClick={() => fit()}>Fit</button>
      <div ref={box} style={{ position: "relative", flex: 1, minHeight: 0 }}>
        <canvas
          ref={cvs}
          style={{ position: "absolute", inset: 0, width: "100%", height: "100%", touchAction: "none", cursor: "grab" }}
          onWheel={(e) => {
            e.preventDefault();
            const r = e.currentTarget.getBoundingClientRect();
            setView((v) => zoomAt(v, 1.1 ** -Math.sign(e.deltaY), e.clientX - r.left, e.clientY - r.top));
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
    </div>
  );
}
