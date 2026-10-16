import fc from "fast-check";
import { expect, test } from "vitest";
import { History } from "./history";

test("a changed setup is a step, and undo and redo walk the steps", () => {
  const h = new History();
  h.see("a");
  h.see("a"); // an unchanged reply is no step
  h.see("b");
  h.see("c");
  expect(h.past).toEqual(["a", "b"]);
  expect(h.undo()).toBe("b");
  h.see("b", true);
  expect(h.undo()).toBe("a");
  expect(h.undo()).toBeUndefined();
  expect(h.redo()).toBe("b");
  expect(h.future).toEqual(["c"]);
});

test("a new step empties redo, and a restore does not", () => {
  const h = new History();
  h.see("a");
  h.see("b");
  h.undo();
  h.see("a2", true); // a restore that came back written differently
  expect(h.future).toEqual(["b"]);
  h.see("x");
  expect(h.past).toEqual(["a2"]);
  expect(h.future).toEqual([]);
  expect(h.redo()).toBeUndefined();
});

test("the undo stack keeps the latest setups", () => {
  const h = new History(2);
  for (const s of ["a", "b", "c", "d"]) h.see(s);
  expect(h.past).toEqual(["b", "c"]);
});

// The laws undo keeps, on random runs of changes, undos and redos (fast-check, MIT). The seed and
// the run count are fixed; PLOTLINE_LAWS_SCALE and PLOTLINE_LAWS_SEED raise and move them, as for
// the engine's laws in crates/core/tests/laws.rs.
const scale = Math.max(1, Number(process.env.PLOTLINE_LAWS_SCALE) || 1);
const options = { seed: Number(process.env.PLOTLINE_LAWS_SEED) || 0x706c6f74, numRuns: 200 * scale };

type Op = string | "undo" | "redo";
const ops = fc.array(fc.oneof(fc.constantFrom("a", "b", "c", "d"), fc.constantFrom<Op>("undo", "redo")), { maxLength: 40 });

/** Play `ops` on a history that starts at "start", as the app does: undo and redo restore the
 * setup they return, whose reply comes back marked as restored. Returns the current setup. */
function play(h: History, list: Op[]): string {
  let now = "start";
  h.see(now);
  for (const op of list) {
    const setup = op === "undo" ? h.undo() : op === "redo" ? h.redo() : op;
    if (setup === undefined) continue;
    h.see(setup, op === "undo" || op === "redo");
    now = setup;
  }
  return now;
}

test("undo then redo, or redo then undo, comes back to the same setup", () => {
  fc.assert(
    fc.property(ops, fc.integer({ min: 1, max: 5 }), (list, limit) => {
      const h = new History(limit);
      const now = play(h, list);
      if (h.past.length > 0) {
        h.see(h.undo()!, true);
        expect(h.redo()).toBe(now);
        h.see(now, true);
      }
      if (h.future.length > 0) {
        h.see(h.redo()!, true);
        expect(h.undo()).toBe(now);
      }
    }),
    options,
  );
});

test("undo walks back through the latest changes, as many as the limit keeps", () => {
  fc.assert(
    fc.property(fc.array(fc.constantFrom("a", "b", "c"), { maxLength: 20 }), fc.integer({ min: 1, max: 5 }), (list, limit) => {
      const h = new History(limit);
      play(h, list);
      // The setups seen, with repeats of the current one dropped, which are no change.
      const steps = ["start", ...list].filter((s, i, all) => i === 0 || s !== all[i - 1]);
      const back = steps.slice(0, -1).reverse().slice(0, limit);
      for (const setup of back) {
        expect(h.undo()).toBe(setup);
        h.see(setup, true);
      }
      expect(h.undo()).toBeUndefined();
    }),
    options,
  );
});

test("a change empties redo, a repeat changes nothing, and the stacks keep within the limit", () => {
  fc.assert(
    fc.property(ops, fc.integer({ min: 1, max: 5 }), fc.constantFrom("a", "b", "c", "d", "e"), (list, limit, next) => {
      const h = new History(limit);
      const now = play(h, list);
      expect(h.past.length + h.future.length).toBeLessThanOrEqual(limit);
      const [past, future] = [[...h.past], [...h.future]];
      h.see(now);
      expect([h.past, h.future]).toEqual([past, future]);
      h.see(next);
      if (next !== now) {
        expect(h.future).toEqual([]);
        expect(h.past.at(-1)).toBe(now);
      }
    }),
    options,
  );
});
