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
