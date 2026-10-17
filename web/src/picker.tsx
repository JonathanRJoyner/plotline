// The style picker of spec 2026-10-16-style-picker: every style grouped by family, each drawn
// small on the current image with its label and description, in a dialog with room for more tabs.

/** One style the picker lists; a learned model's entry may bring a note and a picture of its own. */
export type Entry = { name: string; label: string; description: string; note?: string; picture?: string };

// The families and their order, word for word as docs/styles.md heads them; a test keeps the two equal.
export const FAMILIES: ReadonlyArray<readonly [label: string, names: readonly string[]]> = [
  ["Lines", ["grid", "hatch", "waves", "spiral", "truchet", "ink"]],
  ["The Voronoi point family", ["stipple", "line", "dashes", "tree", "mesh", "cells", "scribble"]],
  ["Sketch lines", ["sketch"]],
  ["Other placers", ["halftone", "dither", "circles"]],
  ["Tone only", ["contours", "flow", "hilbert", "mosaic"]],
];
// Where a style in no family goes, so a style the table does not know yet still shows.
export const MORE = "More styles";

/** The entries by family in `FAMILIES` order, then the rest under `MORE` in their own order; no group is empty. */
export function groupStyles<E extends { name: string }>(entries: readonly E[]): Array<[string, E[]]> {
  const byName = new Map(entries.map((e) => [e.name, e]));
  const groups: Array<[string, E[]]> = FAMILIES.map(([label, names]) => [label, names.flatMap((n) => byName.get(n) ?? [])]);
  const known = new Set(FAMILIES.flatMap(([, names]) => names));
  groups.push([MORE, entries.filter((e) => !known.has(e.name))]);
  return groups.filter(([, es]) => es.length > 0);
}
