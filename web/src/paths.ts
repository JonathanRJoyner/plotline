// The engine's flat path layout: `coords` holds x, y pairs; `offsets` holds each
// path's first vertex index, followed by the total vertex count.

export function pathCount(offsets: Uint32Array): number {
  return Math.max(0, offsets.length - 1);
}

// Calls `fn(start, end)` once per path: vertex indexes, start inclusive, end exclusive.
// Vertex `v` sits at coords[2 * v], coords[2 * v + 1].
export function eachPath(
  _coords: Float64Array,
  offsets: Uint32Array,
  fn: (start: number, end: number) => void,
): void {
  for (let i = 0; i < pathCount(offsets); i++) fn(offsets[i], offsets[i + 1]);
}
