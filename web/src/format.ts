// How the stats line and the Fit and optimize panel write their numbers, kept apart from the panels so tests can cover them.
import type { Optimum, Trial } from "./events";

/** "1 path", "2 paths", "1,234 paths". */
export const pathsText = (n: number): string => `${n.toLocaleString("en-GB")} ${n === 1 ? "path" : "paths"}`;

/** A length in mm: millimetres to a tenth below 1000 mm, metres to a hundredth from there. */
export function lengthText(mm: number): string {
  // Rounded first, so 999.96 mm reads "1.00 m" and not "1000.0 mm".
  return Number(mm.toFixed(1)) < 1000 ? `${mm.toFixed(1)} mm` : `${(mm / 1000).toFixed(2)} m`;
}

// What left a search's trials out, in the engine's order (`Search::unscored` in crates/core/src/optimize.rs).
const LEFT_OUT: [(t: Trial) => boolean, string][] = [
  [(t) => t.over, "over the target"],
  [(t) => t.slow, "too slow"],
  [(t) => t.warnings.length > 0, "with a new lint warning"],
  [(t) => t.failed !== null, "failed"],
  [(t) => t.failed === null && t.error === null, "with no tone error"],
];

/** Why a search with no trial scored has no suggestion, as the engine says it, under the setting's `label` with each value written by `at`. */
export function unscoredText(o: Optimum, label: string, at: (v: number) => string): string {
  const values = o.trials.map((t) => t.value);
  const range = `${label} from ${at(Math.min(...values))} to ${at(Math.max(...values))}`;
  if (o.trials.every((t) => t.error === null)) {
    // A drawing with no marks plots in no time.
    if (o.trials.every((t) => !t.minutes)) return `No value of ${range} draws anything here, so there is nothing to compare. Try a lower threshold, or the tone's black, white and gamma settings.`;
    return `No drawing of ${range} follows the image's tones enough for a tone error, so there is no value to suggest; judge this style by eye.`;
  }
  const why = LEFT_OUT.map(([out, what]) => [o.trials.filter(out).length, what] as const).filter(([n]) => n > 0).map(([n, what]) => `${n} ${what}`);
  return `No value of ${range} can be suggested: of ${o.trials.length} trials, ${why.join(", ")}.`;
}
