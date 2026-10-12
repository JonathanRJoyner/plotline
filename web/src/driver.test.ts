import { readFileSync } from "node:fs";
import { beforeAll, expect, test } from "vitest";
import init, { Session, syntheticGradient } from "../pkg/plotline_wasm.js";
import { drive, macrotask } from "./driver";
import type { Event, FrameEvent } from "./events";
import { pathCount } from "./paths";

beforeAll(async () => { await init({ module_or_path: readFileSync(new URL("../pkg/plotline_wasm_bg.wasm", import.meta.url)) }); });

function session(): Session {
  const s = new Session();
  s.setImage(64, 48, syntheticGradient(64, 48));
  s.setCanvas(JSON.stringify({ width_mm: 100, height_mm: 75, margin_mm: 5, fit: "fit" }));
  s.update("tone", JSON.stringify({ resolution: 64 }));
  return s;
}

const stages = ["tone", "points", "render", "order"];

test("grid runs to a result", async () => {
  const s = session();
  const events: Event[] = [];
  await drive(s, s.start(), 1, (e) => { events.push(e); }, { frameMs: 0 });

  expect(events[0]).toEqual({ type: "run", id: 1 });
  for (const stage of stages) {
    expect(events.some((e) => e.type === "frame" && e.stage === stage && e.progress === 1)).toBe(true);
  }
  const results = events.filter((e) => e.type === "result");
  expect(results).toHaveLength(1);
  const result = results[0];
  expect(result.id).toBe(1);
  expect(result.layers).toHaveLength(1);
  expect(result.layers[0].pen.name).toBe("Black 0.3");
  const end = events[events.length - 1];
  expect(end.type).toBe("end");
  if (end.type !== "end") return;
  expect(end.state).toBe("done");
  expect(end.error).toBeNull();
  const paths = pathCount(result.layers[0].offsets);
  expect(paths).toBeGreaterThan(0);
  expect(paths).toBe(end.stats?.path_count);
  expect(result.tone?.width_px).toBe(64);
  expect(result.tone?.height_px).toBe(48);
  expect(result.tone?.ink.length).toBe(3072);
});

test("throttling keeps every final frame", async () => {
  const s = session();
  const frames: FrameEvent[] = [];
  await drive(s, s.start(), 1, (e) => { if (e.type === "frame") frames.push(e); }, { frameMs: 1e9, now: () => 0 });

  expect(frames.every((f) => f.progress >= 1)).toBe(true);
  for (const stage of stages) expect(frames.some((f) => f.stage === stage)).toBe(true);
});

test("a second start cancels the first", async () => {
  const s = session();
  s.setStyle("_slow");
  s.update("b", JSON.stringify({ frames: 100, delay_ms: 0 }));
  const events: Event[] = [];
  const post = (e: Event) => { events.push(e); };

  const first = drive(s, s.start(), 1, post, { yieldMs: 0 });
  while (!events.some((e) => e.type === "frame")) await macrotask();
  const second = drive(s, s.start(), 2, post, { yieldMs: 0 });
  await Promise.all([first, second]);

  const ends = events.filter((e) => e.type === "end");
  expect(ends).toHaveLength(2);
  expect(ends.find((e) => e.id === 1)?.state).toBe("cancelled");
  expect(ends.find((e) => e.id === 2)?.state).toBe("done");
});
