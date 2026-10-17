import { readFileSync } from "node:fs";
import { beforeAll, expect, test } from "vitest";
import init, { listStyles, Session, syntheticGradient } from "../pkg/plotline_wasm.js";
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

test("focus points: the engine places a page point in the image and the state says where each is drawn", async () => {
  const { ok, ended } = setup();
  expect(ok("imagePoint", 105, 148.5).value).toBeNull(); // no image yet
  expect(ok("preset").state.focus).toEqual([]);
  ok("setImage", ...image());
  await ended(1);
  // The 64 by 48 image fits the A4 drawing area 190 mm wide and sits centred on the page.
  expect(ok("imagePoint", 105, 148.5).value).toEqual([0.5, 0.5]);
  expect(ok("imagePoint", 10, 77.25).value).toEqual([0, 0]);
  expect(ok("imagePoint", 105, 20).value).toBeNull();
  const { state } = ok("update", "tone", { focus: [[0.5, 0.5], [0.25, 0.75]] });
  expect(state.focus).toEqual([[105, 148.5], [57.5, 184.125]]);
  await ended(2);
  expect(ok("update", "tone", { focus: [] }).state.focus).toEqual([]);
});

test("a focus point outside 0 to 1 ends the run with a message naming it", async () => {
  const { posted, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  ok("update", "tone", { focus: [[0.5, 0.5], [2, 0]] });
  await ended(2);
  const end = posted.find((m) => m.type === "end" && m.id === 2) as { state: string; error: string };
  expect(end.state).toBe("error");
  expect(end.error).toContain("point 2 is [2.0, 0.0]");
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

test("loadPresetText reads a preset, an exported SVG and an exported JSON drawing, and redraws", async () => {
  const { call, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  ok("setStyle", "hatch");
  await ended(2);
  ok("update", "hatch", { spacing_mm: 2.5 });
  await ended(3);
  const files = [JSON.stringify(ok("preset").value), ok("svg").value as string, ok("json").value as string];
  let run = 3;
  for (const text of files) {
    ok("setStyle", "grid");
    await ended(++run);
    const reply = ok("loadPresetText", text);
    expect(reply.state.style).toBe("hatch");
    expect((reply.state.settings as { hatch: { spacing_mm: number } }).hatch.spacing_mm).toBe(2.5);
    expect(reply.state.run.state).toBe("running");
    await ended(++run);
  }
  const refusal = (text: string) => { const r = call("loadPresetText", text); return r.ok ? null : r.error; };
  expect(refusal("plain words")).toBe("not a preset: it is neither JSON nor an SVG");
  expect(refusal("<svg></svg>")).toBe("this SVG carries no plotline preset");
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
  expect(ends).toEqual([{ type: "end", id: 1, state: "error", error: "boom", stats: null, tone: null, lint: [] }]);
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

test("hpgl needs a drawing but no machine, and a resolution above 0", async () => {
  const { call, ok, ended } = setup();
  ok("setCanvas", { width_mm: 100, height_mm: 75, margin_mm: 5, fit: "fit" });
  expect(call("hpgl", 40).ok).toBe(false);
  ok("update", "tone", { resolution: 64 });
  ok("setImage", ...image());
  await ended(1);
  const text = ok("hpgl", 40).value as string;
  expect(text.startsWith("IN;\nSP1;\nPU")).toBe(true);
  expect(text.endsWith("PU;\nSP0;\n")).toBe(true);
  expect(ok("hpgl", 10).value).not.toBe(text);
  expect(call("hpgl", 0).ok).toBe(false);
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

test("apply takes a crop into the page, keeps the rest of it, and zeros take it off", async () => {
  const { call, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  const { state } = ok("apply", { page: { crop: [0.1, 0, 0.2, 0] } });
  expect(state.canvas).toMatchObject({ width_mm: 210, height_mm: 297, margin_mm: 10, crop: [0.1, 0, 0.2, 0] });
  await ended(2);
  // The engine leaves an all-zero crop out of the page, as it does of a preset.
  expect(ok("apply", { page: { crop: [0, 0, 0, 0] } }).state.canvas.crop).toBeUndefined();
  await ended(3);
  expect(call("apply", { page: { crop: [0.5, 0, 0, 0] } }).ok).toBe(false);
});

test("apply sets a patch's pens before its settings and redraws once", async () => {
  const { posted, ok, ended } = setup();
  ok("setImage", 400, 200, syntheticGradient(400, 200));
  await ended(1);
  const pens = [{ name: "Yellow 0.3", width_mm: 0.3, color: "#f5c400" }, { name: "Black 0.3", width_mm: 0.3, color: "#000000" }];
  const { state } = ok("apply", { pens, settings: { split: { mode: "by_colour" } } });
  expect(state.pens).toEqual(pens);
  expect((state.settings.split as { mode: string }).mode).toBe("by_colour");
  expect(posted.filter((m) => m.type === "run")).toHaveLength(2);
  await ended(2);
});

test("a refused patch redraws nothing", async () => {
  const { posted, call, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  expect(call("apply", { style: "no such style" }).ok).toBe(false);
  expect(posted.filter((m) => m.type === "run")).toHaveLength(1);
});

test("applyLook sets the look's settings, the state names it, and the page can undo it through restore", async () => {
  const { posted, call, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  ok("setStyle", "waves");
  await ended(2);
  const before = ok("preset").state;
  expect(before.look).toBe("balanced");
  const runs = () => posted.filter((m) => m.type === "run").length;
  const { state } = ok("applyLook", "fine");
  expect(state.look).toBe("fine");
  expect((state.settings.waves as { spacing_mm: number }).spacing_mm).toBe(1);
  expect(state.settings.tone).toEqual(before.settings.tone); // only the look's keys change
  expect(runs()).toBe(3);
  await ended(3);
  // An unknown look changes nothing and redraws nothing.
  const refused = call("applyLook", "no such look");
  expect(refused.ok).toBe(false);
  if (!refused.ok) expect(refused.error).toContain("balanced");
  expect(ok("preset").state.look).toBe("fine");
  expect(runs()).toBe(3);
  // A setting off the look's value is Custom, and a restore of the earlier setup shows Balanced.
  expect(ok("update", "waves", { spacing_mm: 1.7 }).state.look).toBeNull();
  await ended(4);
  const { style, canvas, pens, machine, settings } = before;
  expect(ok("restore", { style, canvas, pens, machine, settings }).state.look).toBe("balanced");
  await ended(5);
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

test("setPaper only repaints: no run, and a restore brings the old paper back", async () => {
  const { posted, call, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  const before = ok("preset").state;
  expect(before.paper).toBe("#ffffff");
  const { state } = ok("setPaper", "#f5ecd7");
  expect(state.paper).toBe("#f5ecd7");
  expect((ok("preset").value as { paper: string }).paper).toBe("#f5ecd7");
  expect(state.run.id).toBe(1);
  expect(posted.filter((m) => m.type === "run")).toHaveLength(1);
  expect(call("setPaper", "cream").ok).toBe(false);
  expect(ok("preset").state.paper).toBe("#f5ecd7");
  expect(setupOf(state)).not.toBe(setupOf(before));
  const { style, canvas, paper, pens, machine, settings } = before;
  expect(ok("restore", { style, canvas, paper, pens, machine, settings }).state.paper).toBe("#ffffff");
  await ended(2); // a restore redraws, as for any setup
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
    () => ok("setPaper", pick(["#ffffff", "#f5ecd7", "#000000"])),
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

test("a panic ends the run with its message, and the rebuilt session keeps the image and setup", async () => {
  const { posted, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  ok("setStyle", "_slow");
  await ended(2);
  ok("update", "b", { frames: 3, delay_ms: 0, panic_after: 2 });
  await ended(3);
  const end = posted.find((m) => m.type === "end" && m.id === 3);
  expect(end).toMatchObject({ state: "error", error: expect.stringMatching(/^the engine failed: .*slow panicked/s) });
  // A poisoned session would refuse every call from here on.
  expect(ok("preset").state).toMatchObject({ image: true, style: "_slow" });
  ok("setStyle", "grid");
  await ended(4);
  expect(posted.find((m) => m.type === "end" && m.id === 4)).toMatchObject({ state: "done" });
});

test("a trap in one style's picture shows in its cell, and the grid and drawing carry on", async () => {
  const session = new Session();
  const trial = session.trial.bind(session);
  // A trap as a panic gives one; the session is then replaced, so only this one trial traps.
  session.trial = (name: string) => {
    const t = trial(name);
    if (name === "ink") t.nextFrame = () => { throw new WebAssembly.RuntimeError("unreachable"); };
    return t;
  };
  const posted: Array<Record<string, any>> = [];
  const p = new Protocol(session, (msg) => posted.push(msg as Record<string, any>));
  // A small page and tone map, which a style switch carries to every trial, keep the grid quick.
  p.handle({ id: 1, method: "setCanvas", args: [{ width_mm: 100, height_mm: 75, margin_mm: 5, fit: "fit" }] });
  p.handle({ id: 2, method: "update", args: ["tone", { resolution: 64 }] });
  p.handle({ id: 3, method: "setImage", args: image() });
  while (!posted.some((m) => m.type === "end")) await macrotask();
  const styles = JSON.parse(listStyles()) as unknown[];
  p.handle({ id: 4, method: "thumbs", args: [120] });
  for (let i = 0; i < 20000 && posted.filter((m) => m.type === "thumb").length < styles.length; i++) await macrotask();
  const thumbs = posted.filter((m) => m.type === "thumb");
  expect(thumbs.filter((m) => m.error !== null)).toEqual([expect.objectContaining({ style: "ink", error: "unreachable", png: null })]);
  expect(thumbs.filter((m) => m.png !== null)).toHaveLength(styles.length - 1);
  expect(posted.filter((m) => m.type === "run").map((m) => m.id)).toEqual([1, 2]); // the drawing, redrawn on the new session
}, 30_000);
