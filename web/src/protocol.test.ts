import { readFileSync } from "node:fs";
import { beforeAll, expect, test } from "vitest";
import init, { Session, syntheticGradient } from "../pkg/plotline_wasm.js";
import { macrotask } from "./driver";
import type { Finding, Ink, Machine, Pen } from "./events";
import { History, setupOf } from "./history";
import { Protocol, type Reply, type State } from "./protocol";
import { controlsFor } from "./schema";

beforeAll(async () => { await init({ module_or_path: readFileSync(new URL("../pkg/plotline_wasm_bg.wasm", import.meta.url)) }); });

type Ok = Extract<Reply, { ok: true }>;

// `onPost` sees each message before it is kept, and may throw or send a command in the middle of a run.
function setup(onPost: (msg: Record<string, unknown>, call: (method: string, ...args: unknown[]) => Reply) => void = () => {}) {
  const posted: Array<Record<string, unknown>> = [];
  const p = new Protocol(new Session(), (msg) => {
    onPost(msg as unknown as Record<string, unknown>, call);
    posted.push(msg as unknown as Record<string, unknown>);
  });
  let next = 0;
  const call = (method: string, ...args: unknown[]): Reply => {
    const id = ++next;
    p.handle({ id, method, args });
    return posted.find((m) => "ok" in m && m.id === id) as Reply;
  };
  const ok = (method: string, ...args: unknown[]): Ok => {
    const reply = call(method, ...args);
    if (!reply.ok) throw new Error(reply.error);
    return reply;
  };
  const ended = async (id: number) => { while (!posted.some((m) => m.type === "end" && m.id === id)) await macrotask(); };
  return { posted, call, ok, ended };
}

const image = () => [64, 48, syntheticGradient(64, 48)];

test("state before an image", () => {
  const { ok } = setup();
  const reply = ok("preset");
  expect(reply.state.image).toBe(false);
  expect(reply.state.style).toBe("grid");
  expect(reply.state.run).toEqual({ id: 0, state: "idle", error: null });
  expect(reply.state.stats).toBeNull();
  expect((reply.value as { format: string }).format).toBe("plotline-preset");
});

test("setImage starts a run", async () => {
  const { posted, ok, ended } = setup();
  const { state } = ok("setImage", ...image());
  expect(state.image).toBe(true);
  expect(state.run.state).toBe("running");
  await ended(1);
  expect(posted.some((m) => m.type === "result" && m.id === 1)).toBe(true);
});

test("bad image bytes are refused", () => {
  const { posted, call, ok } = setup();
  const reply = call("setImageBytes", new Uint8Array([1, 2, 3]));
  expect(reply.ok).toBe(false);
  if (!reply.ok) expect(reply.error).not.toBe("");
  expect(ok("preset").state.image).toBe(false);
  expect(posted.some((m) => m.type === "run")).toBe(false);
});

test("a fit runs like a run and its end carries the value to apply", async () => {
  const { posted, call, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  ok("setStyle", "hatch");
  await ended(2);
  expect(ok("fit", "paths", 400).state.run).toEqual({ id: 3, state: "running", error: null });
  await ended(3);
  const end = posted.find((m) => m.type === "end" && m.id === 3) as { state: string; fit: { stage: string; key: string; samples: unknown[] } };
  expect(end.state).toBe("done");
  expect([end.fit.stage, end.fit.key]).toEqual(["hatch", "spacing_mm"]);
  expect(end.fit.samples.length).toBeGreaterThan(1);
  // The fit leaves the settings to the page.
  expect((ok("preset").state.settings as { hatch: { spacing_mm: number } }).hatch.spacing_mm).toBe(1);
  expect(posted.some((m) => m.type === "frame" && m.id === 3 && m.stage === "fit")).toBe(true);
  const refused = call("fit", "dots", 10);
  expect(refused.ok).toBe(false);
  if (!refused.ok) expect(refused.error).toContain("places no dots");
  expect((posted.find((m) => m.type === "end" && m.id === 2) as { fit: unknown }).fit).toBeNull();
});

test("a refused setting changes nothing", async () => {
  const { call, ok, ended } = setup();
  ok("setImage", ...image());
  expect(call("update", "tone", { resolution: 10 }).ok).toBe(false);
  const settings = ok("preset").state.settings as { tone: { resolution: number } };
  expect(settings.tone.resolution).toBe(1000);
  await ended(1);
});

test("a bad preset changes nothing", () => {
  const { call, ok } = setup();
  const before = ok("preset").state;
  const pick = ({ style, canvas, pens }: State) => ({ style, canvas, pens });
  expect(call("loadPreset", { format: "other", version: 1 }).ok).toBe(false);
  expect(call("loadPreset", { format: "plotline-preset", version: 99, style: "grid", canvas: {}, pens: [], settings: {} }).ok).toBe(false);
  expect(pick(ok("preset").state)).toEqual(pick(before));
});

test("a bad pen is refused", () => {
  const { call, ok } = setup();
  const before = ok("preset").state.pens;
  expect(call("setPens", [{ name: "x", width_mm: 0, color: "#000000" }]).ok).toBe(false);
  expect(call("setPens", [{ name: "x", width_mm: 0.3, color: "black" }]).ok).toBe(false);
  expect(ok("preset").state.pens).toEqual(before);
});

test("describe and listStyles", () => {
  const { ok } = setup();
  const styles = ok("listStyles").value as Array<{ name: string }>;
  expect(styles.some((s) => s.name === "grid")).toBe(true);
  expect(styles.some((s) => s.name.startsWith("_"))).toBe(false);
  const schema = ok("describe", "grid").value as { stages: Array<{ name: string; schema: Record<string, unknown> }> };
  expect(schema.stages).toHaveLength(5);
  // The Pens panel shows the split stage's one setting as a select named Pen use.
  const [mode] = controlsFor(schema.stages.find((s) => s.name === "split")!.schema);
  expect(mode).toMatchObject({ kind: "select", label: "Pen use", default: "one_pen" });
  expect(mode.kind === "select" && mode.options.map((o) => o.label)).toEqual(["one pen", "light to dark", "by colour"]);
});

test("svg and json after a run", async () => {
  const { ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  expect(ok("svg").value).toMatch(/^<svg/);
  expect(JSON.parse(ok("json").value as string).format).toBe("plotline-paths");
});

test("a run that fails to post still ends once", async () => {
  const { posted, ok, ended } = setup((m) => { if (m.type === "frame") throw new Error("boom"); });
  ok("setImage", ...image());
  await ended(1);
  const ends = posted.filter((m) => m.type === "end");
  expect(ends).toEqual([{ type: "end", id: 1, state: "error", error: "boom", stats: null, lint: [] }]);
});

test("cancel stops the current run", async () => {
  let sent = false;
  const { posted, ok, ended } = setup((m, call) => { if (m.type === "frame" && !sent) { sent = true; expect(call("cancel").ok).toBe(true); } });
  ok("setImage", ...image());
  await ended(1);
  expect(posted.filter((m) => m.type === "end")).toEqual([expect.objectContaining({ id: 1, state: "cancelled" })]);
});

test("testchart posts the events of a finished run, and the next run replaces the chart", async () => {
  const { posted, ok, ended } = setup();
  const { state } = ok("testchart"); // no image is needed
  expect(state.run).toEqual({ id: 1, state: "done", error: null });
  expect(state.stats!.path_count).toBeGreaterThan(0);
  expect(state.lint.map((f) => f.code)).toContain("overdraw");
  expect(state.layers).toHaveLength(1);
  await ended(1);
  expect(posted.filter((m) => "type" in m).map((m) => m.type)).toEqual(["run", "result", "end"]);
  expect(posted.find((m) => m.type === "end")).toEqual(expect.objectContaining({ id: 1, state: "done", fit: null }));
  expect(ok("svg").value as string).toContain("<svg");
  ok("setImage", ...image());
  await ended(2);
  expect(posted.some((m) => m.type === "result" && m.id === 2)).toBe(true);
});

test("a testchart in the middle of a run cancels it", async () => {
  let sent = false;
  const { posted, ok, ended } = setup((m, call) => { if (m.type === "frame" && !sent) { sent = true; expect(call("testchart").ok).toBe(true); } });
  ok("setImage", ...image());
  await ended(1);
  expect(posted.find((m) => m.type === "end" && m.id === 1)).toEqual(expect.objectContaining({ state: "cancelled" }));
  expect(posted.some((m) => m.type === "result" && m.id === 2)).toBe(true);
  expect(posted.some((m) => m.type === "result" && m.id === 1)).toBe(false);
});

test("a test chart that does not fit is refused and the drawing stays", () => {
  const { posted, call, ok } = setup();
  ok("testchart");
  const stats = ok("preset").state.stats;
  ok("setCanvas", { width_mm: 74, height_mm: 105, margin_mm: 10, fit: "fit" });
  ok("setPens", [{ name: "Fat", width_mm: 2, color: "#000000" }]);
  const before = posted.length;
  const refused = call("testchart");
  expect(refused.ok).toBe(false);
  if (!refused.ok) expect(refused.error).toContain("the page is too small for the test chart");
  expect(posted.length).toBe(before + 1); // the refusal, and no run events
  expect(ok("preset").state.stats).toEqual(stats);
});

test("listPens gives the bank", () => {
  const { ok } = setup();
  const pens = ok("listPens").value as Pen[];
  expect(pens.length).toBeGreaterThan(30);
  expect(pens.find((p) => p.name === "Sakura Pigma Micron 005")?.width_mm).toBe(0.2);
});

test("listInks gives the ink colours", () => {
  const { ok } = setup();
  const inks = ok("listInks").value as Ink[];
  expect(inks[0]).toEqual({ name: "black", color: "#000000" });
  expect(inks.find((i) => i.name === "blue")?.color).toMatch(/^#[0-9a-f]{6}$/);
});

const MACHINE: Machine = {
  name: "Test printer", area: { x: 28, y: 42, width: 200, height: 174 },
  z_down_mm: 4, z_up_mm: 9, z_safe_mm: 20, draw_feed: 3600, travel_feed: 6000, z_feed: 1200, settle_ms: 50,
  start_gcode: "", end_gcode: "",
};

test("setMachine sets the speeds and starts no run", async () => {
  const { posted, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  const { state } = ok("setMachine", MACHINE);
  expect(state.machine?.name).toBe(MACHINE.name);
  expect(state.speeds.pen_lift_s).toBeCloseTo(0.6);
  expect(posted.filter((m) => m.type === "run")).toHaveLength(1);
  expect(ok("setMachine", null).state.machine).toBeNull();
});

test("gcode needs a drawing, whose layers the state lists", async () => {
  const { call, ok, ended } = setup();
  ok("setMachine", MACHINE);
  ok("setCanvas", { width_mm: 100, height_mm: 75, margin_mm: 5, fit: "fit" });
  expect(call("gcode", 0).ok).toBe(false);
  expect(ok("preset").state.layers).toEqual([]);
  ok("update", "tone", { resolution: 64 });
  ok("setImage", ...image());
  await ended(1);
  expect(ok("preset").state.layers.map((p) => p.name)).toEqual(["Black 0.3"]);
  expect(ok("gcode", 0).value as string).toContain("G1 Z4 F1200");
});

test("the lint findings come with the end event and the state, and their patches apply", async () => {
  const { posted, ok, ended } = setup();
  expect(ok("preset").state.lint).toEqual([]);
  ok("update", "tone", { resolution: 64 });
  ok("setImage", ...image());
  await ended(1);
  const end = posted.find((m) => m.type === "end") as { lint: Finding[] };
  const { lint } = ok("preset").state;
  expect(end.lint).toEqual(lint);
  // Grid dots on the gradient are too sparse to reach dark, and a closer spacing fixes it.
  const dark = lint.find((f) => f.code === "dark");
  expect(dark?.patch).toEqual({ settings: { points: { spacing_mm: 0.3 } } });
  const { state } = ok("apply", dark!.patch);
  expect((state.settings.points as { spacing_mm: number }).spacing_mm).toBe(0.3);
  await ended(2);
});

test("suggestions come with the state", () => {
  const { ok } = setup();
  expect(ok("preset").state.suggestions).toEqual([]);
  // 400 x 200 on the portrait A4 default: turning the page would draw it larger.
  const { state } = ok("setImage", 400, 200, syntheticGradient(400, 200));
  expect(state.suggestions.map((s) => s.kind)).toEqual(["orientation"]);
  expect(state.suggestions[0].patch).toEqual({ page: { width_mm: 297, height_mm: 210 } });
});

test("apply runs the patch and redraws once", async () => {
  const { posted, ok, ended } = setup();
  ok("setImage", 400, 200, syntheticGradient(400, 200));
  await ended(1);
  const runs = () => posted.filter((m) => m.type === "run").length;
  expect(runs()).toBe(1);
  const { state } = ok("apply", { style: "ink", settings: { tone: { levels: "auto" } }, page: { width_mm: 297, height_mm: 210 } });
  expect(state.style).toBe("ink");
  expect((state.settings.tone as { levels: string }).levels).toBe("auto");
  expect(state.canvas).toMatchObject({ width_mm: 297, height_mm: 210 });
  expect(state.suggestions.map((s) => s.kind)).not.toContain("orientation");
  expect(runs()).toBe(2);
  await ended(2);
});

test("a refused patch redraws nothing", async () => {
  const { posted, call, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  expect(call("apply", { style: "no such style" }).ok).toBe(false);
  expect(posted.filter((m) => m.type === "run")).toHaveLength(1);
});

test("thumbs draws styles without touching the drawing, and cancelThumbs stops it", async () => {
  let thumbs = 0;
  const { posted, ok, ended } = setup((m, call) => { if (m.type === "thumb" && ++thumbs === 2) expect(call("cancelThumbs").ok).toBe(true); });
  ok("setCanvas", { width_mm: 100, height_mm: 75, margin_mm: 5, fit: "fit" });
  ok("setImage", ...image());
  await ended(1);
  const before = ok("preset").state;
  const svg = ok("svg").value;
  const id = ok("thumbs", 120).value as number;
  for (let i = 0; i < 5000 && thumbs < 2; i++) await macrotask();
  for (let i = 0; i < 50; i++) await macrotask(); // time for a cancelled loop to post by mistake
  const events = posted.filter((m) => m.type === "thumb");
  expect(events.map((m) => [m.id, m.style, m.error])).toEqual([[id, "grid", null], [id, "hatch", null]]);
  expect([...(events[0].png as Uint8Array).slice(1, 4)]).toEqual([80, 78, 71]); // "PNG"
  expect(ok("preset").state).toEqual(before); // style, settings, page, pens, run and stats
  expect(ok("svg").value).toBe(svg); // and the drawing itself
  expect(posted.filter((m) => m.type === "run")).toHaveLength(1);
});

test("thumbs without an image posts each style's error", async () => {
  const { posted, ok } = setup();
  ok("thumbs", 120);
  for (let i = 0; i < 50; i++) await macrotask();
  const events = posted.filter((m) => m.type === "thumb");
  expect(events).toHaveLength((ok("listStyles").value as unknown[]).length);
  expect(events[0]).toMatchObject({ style: "grid", png: null, error: "no image is loaded" });
});

test("restore applies a setup over the preset and redraws", async () => {
  const { posted, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  const before = ok("preset").state;
  ok("setStyle", "hatch");
  ok("setPens", [{ name: "Red", width_mm: 0.5, color: "#ff0000" }]);
  await ended(3);
  const { style, canvas, pens, machine, settings } = before;
  const { state } = ok("restore", { style, canvas, pens, machine, settings });
  expect(state.style).toBe("grid");
  expect(state.pens).toEqual(pens);
  expect(state.settings).toEqual(settings);
  expect(posted.filter((m) => m.type === "run")).toHaveLength(4);
  await ended(4);
});

test("undoing a random run of edits through restore returns the start, and undo then redo changes nothing", () => {
  const { ok } = setup();
  const history = new History(1000);
  const see = (state: State, restored = false) => { history.see(setupOf(state), restored); return setupOf(state); };
  const start = see(ok("preset").state);
  const styles = (ok("listStyles").value as Array<{ name: string }>).map((s) => s.name);
  let seed = 7; // a fixed linear congruential sequence, so a failure repeats
  const rand = () => (seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31;
  const pick = <T,>(xs: T[]) => xs[Math.floor(rand() * xs.length)];
  const edits = [
    () => ok("setStyle", pick(styles)),
    () => ok("resetSettings"),
    () => ok("update", "tone", { gamma: 0.5 + rand() * 1.5, invert: rand() < 0.5 }),
    () => ok("update", "split", { mode: pick(["one_pen", "light_to_dark", "by_colour"]) }),
    () => ok("setCanvas", { width_mm: pick([148, 210, 297]), height_mm: 297, margin_mm: Math.round(rand() * 20), fit: pick(["fit", "fill"]) }),
    () => ok("setPens", [{ name: "P", width_mm: 0.1 + Math.round(rand() * 8) / 10, color: pick(["#000000", "#d0202e"]) }]),
  ];
  for (let i = 0; i < 40; i++) see(pick(edits)().state);
  see(ok("update", "tone", { gamma: 1.3 }).state);
  const steps = history.past.length;
  see(ok("resetSettings").state);
  expect(history.past.length).toBe(steps + 1); // a reset is one step, and the first undo below takes it back
  const end = see(ok("preset").state);
  expect(history.past.length).toBeGreaterThan(20);
  const step = (setup: string | undefined) => {
    const restored = see(ok("restore", JSON.parse(setup!)).state, true);
    expect(restored).toBe(setup); // a restore puts back exactly that setup
    return restored;
  };
  step(history.undo());
  expect(step(history.redo())).toBe(end);
  let now = end;
  while (history.past.length) now = step(history.undo());
  expect(now).toBe(start);
  while (history.future.length) now = step(history.redo());
  expect(now).toBe(end);
});
