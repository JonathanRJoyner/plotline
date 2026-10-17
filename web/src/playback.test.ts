import { expect, test } from "vitest";
import type { Layer } from "./events";
import { clock, duration, fits, moment, nudge, type Timeline } from "./playback";

const pen = { name: "Black", width_mm: 0.3, color: "#000000" };
const layer = (coords: number[], offsets: number[]): Layer => ({ pen, coords: new Float64Array(coords), offsets: new Uint32Array(offsets) });

// Two pens: a stroke and a dot, then a three-vertex line. Each path lifts for 0.125 s, moves,
// then lowers for 0.125 s; the times are exact in binary, so the points come out exact.
const layers = [layer([0, 0, 10, 0, 10, 10], [0, 2, 3]), layer([20, 0, 20, 5, 20, 10], [0, 3])];
const timeline: Timeline = {
  start: [5, 5],
  layers: [
    { travel: new Float64Array([0.125, 0.375, 1.125, 1.625]), vertices: new Float64Array([0.5, 1.0, 1.75]) },
    { travel: new Float64Array([1.875, 2.125]), vertices: new Float64Array([2.25, 2.5, 3.0]) },
  ],
};

test("before the first path lands, the pen lifts at the start, moves, then lowers", () => {
  expect(moment(layers, timeline, 0)).toEqual({ drawn: [{ paths: 0, part: null }, { paths: 0, part: null }], travel: { from: [5, 5], to: [0, 0] }, pen: [5, 5] });
  expect(moment(layers, timeline, 0.25).pen).toEqual([2.5, 2.5]);
  const lowering = moment(layers, timeline, 0.4375);
  expect(lowering.pen).toEqual([0, 0]);
  expect(lowering.travel).toEqual({ from: [5, 5], to: [0, 0] });
});

test("a path being drawn shows up to the pen, between the vertices its time falls between", () => {
  const m = moment(layers, timeline, 0.75);
  expect(m.drawn[0]).toEqual({ paths: 0, part: { from: 0, to: 1, tip: [5, 0] } });
  expect(m.travel).toBeNull();
  expect(m.pen).toEqual([5, 0]);
  const later = moment(layers, timeline, 2.625);
  expect(later.drawn).toEqual([{ paths: 2, part: null }, { paths: 0, part: { from: 0, to: 2, tip: [20, 6.25] } }]);
  expect(later.pen).toEqual([20, 6.25]);
});

test("a path is whole once the pen reaches its last vertex, and the next pen starts where the last one ended", () => {
  const m = moment(layers, timeline, 1.0);
  expect(m.drawn[0].paths).toBe(1);
  expect(m.travel).toEqual({ from: [10, 0], to: [10, 10] });
  expect(m.pen).toEqual([10, 0]);
  const next = moment(layers, timeline, 1.75);
  expect(next.drawn).toEqual([{ paths: 2, part: null }, { paths: 0, part: null }]);
  expect(next.travel).toEqual({ from: [10, 10], to: [20, 0] });
  expect(moment(layers, timeline, 5)).toEqual({ drawn: [{ paths: 2, part: null }, { paths: 1, part: null }], travel: null, pen: [20, 10] });
});

test("an empty path moves nowhere, and the move after it starts from the last vertex drawn", () => {
  const gap = [layer([0, 0, 5, 0], [0, 1, 1, 2])];
  const t: Timeline = { start: [0, 0], layers: [{ travel: new Float64Array([0.125, 0.25, 0.375, 0.375, 0.5, 0.75]), vertices: new Float64Array([0.3125, 1.0]) }] };
  const m = moment(gap, t, 0.625);
  expect(m.drawn[0].paths).toBe(2);
  expect(m.travel).toEqual({ from: [0, 0], to: [5, 0] });
  expect(m.pen).toEqual([2.5, 0]);
});

test("the plot lasts until its last time, and a timeline fits only the drawing it was made for", () => {
  expect(duration(timeline)).toBe(3);
  expect(duration({ start: [0, 0], layers: [] })).toBe(0);
  expect(fits(timeline, layers)).toBe(true);
  expect(fits(timeline, layers.slice(0, 1))).toBe(false);
  expect(fits(timeline, [layers[0], layer([20, 0], [0, 1])])).toBe(false);
});

test("the slider's keys move by a hundredth and a tenth of the plot, and Home and End to its ends", () => {
  expect(nudge(50, 200, "ArrowRight")).toBe(52);
  expect(nudge(50, 200, "ArrowDown")).toBe(48);
  expect(nudge(50, 200, "PageUp")).toBe(70);
  expect(nudge(10, 200, "PageDown")).toBe(0);
  expect(nudge(50, 200, "Home")).toBe(0);
  expect(nudge(50, 200, "End")).toBe(200);
  expect(nudge(199, 200, "ArrowRight")).toBe(200);
  expect(nudge(50, 200, "a")).toBeNull();
});

test("times read as m:ss, or h:mm:ss from an hour", () => {
  expect(clock(0)).toBe("0:00");
  expect(clock(59.6)).toBe("1:00");
  expect(clock(754)).toBe("12:34");
  expect(clock(3725)).toBe("1:02:05");
  expect(clock(-3)).toBe("0:00");
});
