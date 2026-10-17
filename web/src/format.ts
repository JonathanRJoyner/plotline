// How the stats line writes its numbers, kept apart from the panel so tests can cover it.

/** "1 path", "2 paths", "1,234 paths". */
export const pathsText = (n: number): string => `${n.toLocaleString("en-GB")} ${n === 1 ? "path" : "paths"}`;

/** A length in mm: millimetres to a tenth below 1000 mm, metres to a hundredth from there. */
export function lengthText(mm: number): string {
  // Rounded first, so 999.96 mm reads "1.00 m" and not "1000.0 mm".
  return Number(mm.toFixed(1)) < 1000 ? `${mm.toFixed(1)} mm` : `${(mm / 1000).toFixed(2)} m`;
}
