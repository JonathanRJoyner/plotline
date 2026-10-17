// The plot playback: the drawing as far as the pen has got at a time, in plot order and pen by
// pen, from the engine's timeline (`Session::timeline`), which times every move as the plot time
// does. This file only finds where a time falls among the engine's times and puts the pen between
// the two points it falls between; the preview paints the result.
import { Pause, Play } from "lucide-preact";
import type { JSX } from "preact";
import type { Layer } from "./events";
import { pathCount } from "./paths";

/** Where the pen starts, then per layer each path's pen-up move as start, end pairs and when the pen
 * reaches each vertex, in seconds from the plot's start: the engine's `Timeline`. */
export type Timeline = { start: [number, number]; layers: { travel: Float64Array; vertices: Float64Array }[] };

/** The part of a path drawn so far: its vertices `from` to `to`, end exclusive, then a line to `tip`. */
export type Part = { from: number; to: number; tip: [number, number] };

/** The drawing at one time: per layer how many paths are whole and the part of the next one; the
 * pen-up move under way, from its lift to its landing; and where the pen is. */
export type Moment = {
  drawn: { paths: number; part: Part | null }[];
  travel: { from: [number, number]; to: [number, number] } | null;
  pen: [number, number];
};

/** How long a full playback takes, in seconds of the page's clock, whatever the plot time. */
export const PLAY_SECONDS = 30;

/** Whether `t` is the timeline of `layers`: one time per vertex and a move per path in each. A
 * timeline of an older drawing or one asked for before a new result does not fit. */
export const fits = (t: Timeline, layers: Layer[]): boolean =>
  t.layers.length === layers.length &&
  layers.every((l, i) => 2 * t.layers[i].vertices.length === l.coords.length && t.layers[i].travel.length === 2 * pathCount(l.offsets));

/** The plot time in seconds: when the last path ends, 0 for no paths. */
export function duration(t: Timeline): number {
  let end = 0;
  for (const { travel, vertices } of t.layers) {
    if (vertices.length) end = Math.max(end, vertices[vertices.length - 1]);
    if (travel.length) end = Math.max(end, travel[travel.length - 1]);
  }
  return end;
}

const point = (c: Float64Array, v: number): [number, number] => [c[2 * v], c[2 * v + 1]];
const between = (p: [number, number], q: [number, number], f: number): [number, number] => [p[0] + (q[0] - p[0]) * f, p[1] + (q[1] - p[1]) * f];
// The share of the way from `a` to `b` that `t` is, 1 when they are the same time.
const share = (t: number, a: number, b: number): number => (b > a ? Math.min(1, Math.max(0, (t - a) / (b - a))) : 1);

/** The first index in [lo, hi) whose `at(i)` is above `t`, or `hi`; `at` must not fall. */
function firstAbove(at: (i: number) => number, t: number, lo: number, hi: number): number {
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (at(mid) > t) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

/** The drawing of `layers` at `t` seconds into the plot that `timeline` times. A path is whole once
 * the pen reaches its last vertex; a one-vertex path, a dot, once the pen lands on it. */
export function moment(layers: Layer[], timeline: Timeline, t: number): Moment {
  let pen = timeline.start;
  const drawn: Moment["drawn"] = [];
  let travel: Moment["travel"] = null;
  let found = false;
  layers.forEach((l, li) => {
    const n = pathCount(l.offsets);
    if (found || n === 0) return drawn.push({ paths: 0, part: null });
    const { travel: tr, vertices: vt } = timeline.layers[li];
    const o = l.offsets;
    // When path i ends: at its last vertex, or for an empty path when its pen-up move does.
    const end = (i: number) => (o[i + 1] > o[i] ? vt[o[i + 1] - 1] : tr[2 * i + 1]);
    const i = firstAbove(end, t, 0, n);
    // Where the pen is before path i: the last vertex drawn before it, else where it came from.
    let last = i - 1;
    while (last >= 0 && o[last + 1] === o[last]) last--;
    const before = last >= 0 ? point(l.coords, o[last + 1] - 1) : pen;
    if (i === n) {
      drawn.push({ paths: n, part: null });
      pen = before;
      return;
    }
    found = true;
    const [a, b] = [o[i], o[i + 1]];
    if (b === a) {
      pen = before;
      return drawn.push({ paths: i, part: null });
    }
    const first = point(l.coords, a);
    if (t < vt[a]) {
      // Lifting, moving or lowering: the move shows from lift to landing.
      travel = { from: before, to: first };
      pen = between(before, first, share(t, tr[2 * i], tr[2 * i + 1]));
      return drawn.push({ paths: i, part: null });
    }
    // Drawing: the last vertex reached, then the pen's place on the next move.
    const j = firstAbove((v) => vt[v], t, a, b) - 1;
    const tip = between(point(l.coords, j), point(l.coords, j + 1), share(t, vt[j], vt[j + 1]));
    pen = tip;
    drawn.push({ paths: i, part: { from: a, to: j + 1, tip } });
  });
  return { drawn, travel, pen };
}

// The keys the time slider takes, as shares of the plot: the arrows a hundredth, Page Up and Page
// Down a tenth, Home and End all the way.
const KEYS: Record<string, number> = { ArrowRight: 0.01, ArrowUp: 0.01, ArrowLeft: -0.01, ArrowDown: -0.01, PageUp: 0.1, PageDown: -0.1, Home: -1, End: 1 };

/** The time `key` moves the slider to from `at` in a plot of `end` seconds, or null for a key it
 * does not take. The slider handles its keys itself: a range input snaps its value to its step,
 * so a key that would move it within one step did nothing. */
export function nudge(at: number, end: number, key: string): number | null {
  const k = KEYS[key];
  return k === undefined ? null : Math.min(end, Math.max(0, at + k * end));
}

/** A time as m:ss, or h:mm:ss from an hour. */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const [h, m, r] = [Math.floor(s / 3600), Math.floor((s % 3600) / 60), s % 60];
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(r)}` : `${m}:${pad(r)}`;
}

type Props = {
  /** The plot time in seconds, or null until the timeline has come. */
  total: number | null;
  at: number;
  playing: boolean;
  onAt: (at: number) => void;
  onPlaying: (playing: boolean) => void;
};

// Along the bottom of the preview, where the compare slider would be: play or pause, the time
// slider in thousandths of the plot, and the time gone and left.
export function PlaybackBar({ total, at, playing, onAt, onPlaying }: Props): JSX.Element {
  const end = total ?? 0;
  return (
    <div class="playback">
      <button
        type="button" class="icon" aria-label={playing ? "Pause" : "Play"} title={playing ? "Pause" : "Play"} disabled={total === null}
        onClick={() => {
          // Play from the start again once the end is reached.
          if (!playing && at >= end) onAt(0);
          onPlaying(!playing);
        }}
      >
        {playing ? <Pause /> : <Play />}
      </button>
      <input
        type="range" aria-label="Plot time" min="0" max="1000" step="1" value={end > 0 ? Math.round((1000 * at) / end) : 0} disabled={total === null}
        aria-valuetext={`${clock(at)} of ${clock(end)}`}
        onInput={(e) => { onPlaying(false); onAt((e.currentTarget.valueAsNumber / 1000) * end); }}
        onKeyDown={(e) => {
          const to = nudge(at, end, e.key);
          if (to === null) return;
          e.preventDefault();
          onPlaying(false);
          onAt(to);
        }}
      />
      <span class="time">{total === null ? "Timing the plot…" : `${clock(at)} of ${clock(end)}, ${clock(end - at)} left`}</span>
    </div>
  );
}
