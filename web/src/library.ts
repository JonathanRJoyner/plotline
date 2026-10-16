// The pen library: pens the user saved to pick again, kept in the browser and exchanged as a JSON
// file. It lives in the page only; the engine, the CLI and the MCP never see it.
import type { Pen } from "./events";

const FORMAT = "plotline-pens";
const VERSION = 1;
const COLOUR = /^#[0-9a-fA-F]{6}$/;

// What is wrong with a pen, worded to follow "pen 2", or undefined when it is fine.
const problem = (p: Record<string, unknown>): string | undefined => {
  const w = p.width_mm;
  if (typeof p.name !== "string" || p.name.trim() === "") return " has no name";
  if (typeof w !== "number" || !(Number.isFinite(w) && w > 0)) return "'s line width must be a number above 0";
  if (typeof p.color !== "string" || !COLOUR.test(p.color)) return "'s colour must be written as #rrggbb";
};

/** Whether `pen` can be saved: a file with it in would read back. */
export const usable = (pen: Pen): boolean => problem(pen) === undefined;

/** The pens of a library file's parsed JSON, or an Error saying in plain words what is wrong. */
export function parseLibrary(value: unknown): Pen[] {
  const file = (typeof value === "object" && value !== null ? value : {}) as Record<string, unknown>;
  if (file.format !== FORMAT) throw new Error(`not a pen library: its format must be '${FORMAT}'`);
  if (file.version !== VERSION) throw new Error(`unsupported pen library version ${file.version}`);
  if (!Array.isArray(file.pens)) throw new Error("not a pen library: it has no list of pens");
  return file.pens.map((p: unknown, i): Pen => {
    const pen = (typeof p === "object" && p !== null ? p : {}) as Record<string, unknown>;
    const bad = problem(pen);
    if (bad !== undefined) throw new Error(`pen ${i + 1}${bad}`);
    return { name: pen.name as string, width_mm: pen.width_mm as number, color: pen.color as string };
  });
}

/** The text of a library file. */
export const libraryFile = (pens: Pen[]): string => JSON.stringify({ format: FORMAT, version: VERSION, pens }, null, 2);

/** Pens that differ only in the case of their name or colour are one pen. */
export const samePen = (a: Pen, b: Pen): boolean =>
  a.name.toLowerCase() === b.name.toLowerCase() && a.width_mm === b.width_mm && a.color.toLowerCase() === b.color.toLowerCase();

/** `lib` with `pen` appended, unless the same pen is there. */
export const addPen = (lib: Pen[], pen: Pen): Pen[] => (lib.some((p) => samePen(p, pen)) ? lib : [...lib, pen]);

export const removePen = (lib: Pen[], index: number): Pen[] => lib.filter((_, i) => i !== index);

/** `lib`, then each pen of `more` that is not there yet, in order. */
export const mergeLibraries = (lib: Pen[], more: Pen[]): Pen[] => more.reduce(addPen, lib);

// Kept like the remembered preset: storage can be missing or full, and the library is a
// convenience, so a failed save is ignored and an unreadable value is an empty library.
const KEY = "plotline.pens";
export function loadLibrary(): Pen[] {
  try {
    const text = localStorage.getItem(KEY);
    return text === null ? [] : parseLibrary(JSON.parse(text));
  } catch {
    return [];
  }
}
export function saveLibrary(pens: Pen[]): void {
  try {
    localStorage.setItem(KEY, libraryFile(pens));
  } catch { /* ignored */ }
}
