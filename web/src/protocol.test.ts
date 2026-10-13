import { readFileSync } from "node:fs";
import { beforeAll, expect, test } from "vitest";
import init, { Session, syntheticGradient } from "../pkg/plotline_wasm.js";
import { macrotask } from "./driver";
import type { Ink, Machine, Pen } from "./events";
import { Protocol, type Reply, type State } from "./protocol";

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
  const schema = ok("describe", "grid").value as { stages: unknown[] };
  expect(schema.stages).toHaveLength(5);
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
  expect(ends).toEqual([{ type: "end", id: 1, state: "error", error: "boom", stats: null }]);
});

test("cancel stops the current run", async () => {
  let sent = false;
  const { posted, ok, ended } = setup((m, call) => { if (m.type === "frame" && !sent) { sent = true; expect(call("cancel").ok).toBe(true); } });
  ok("setImage", ...image());
  await ended(1);
  expect(posted.filter((m) => m.type === "end")).toEqual([expect.objectContaining({ id: 1, state: "cancelled" })]);
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
